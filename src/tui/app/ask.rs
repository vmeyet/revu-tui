//! `a`: questions to Claude about what is under the cursor, answered in the right pane.
use super::{Action, App, Focus, Input, MrKey, Open};
use crate::act::{Act, Pane};
use crate::ai::anthropic::{self, Ask, Outcome, Role, Turn};
use crate::ai::context::{self, Prompt, Scope};
use crate::forge::Position;
use crate::review::Row;
use chrono::{DateTime, Utc};
use crossterm::event::KeyEvent;

const HALF_PAGE: usize = 10;

/// Claude's answer in the right pane, and the conversation that led to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    /// Grows with every question, so pieces of an older stream can be told apart and dropped.
    pub id: u64,
    /// `explain · charge.rs`, the pane's title.
    pub label: String,
    /// Everything sent so far; the question being answered is its last turn.
    pub request: Ask,
    pub text: String,
    pub state: AnswerState,
    /// Painted from the cache: the same question was answered before for this diff.
    pub cached: bool,
    pub scroll: usize,
    /// Where `c` puts a draft made of the answer: the lines asked about, or the thread.
    pub target: Target,
}

/// An answer Claude gave before on this MR, kept in the cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PastAnswer {
    pub label: String,
    pub asked_at: DateTime<Utc>,
    /// The commit asked about: once the MR moved on, the answer reads an older push.
    pub head: crate::forge::Sha,
    pub request: Ask,
    pub text: String,
    pub outcome: Outcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnswerState {
    Streaming,
    Done(Outcome),
    Failed(String),
}

impl Answer {
    fn with_part(self, part: Part) -> Self {
        match part {
            Part::Text(more) => Self { text: self.text + &more, ..self },
            Part::Restart => Self { text: String::new(), ..self },
            Part::Done { outcome, cached_text } => {
                Self { cached: cached_text.is_some(), text: cached_text.unwrap_or(self.text), state: AnswerState::Done(outcome), ..self }
            }
            Part::Failed(message) => Self { state: AnswerState::Failed(message), ..self },
        }
    }
}

impl Open {
    /// Takes and gives back the whole `Open`: a stream grows the answer piece by piece, so nothing else is copied.
    fn with_answer_part(self, key: &MrKey, id: u64, part: Part) -> Self {
        match self.answer {
            Some(answer) if self.key == *key && answer.id == id => Self { answer: Some(answer.with_part(part)), ..self },
            answer => Self { answer, ..self },
        }
    }

    /// The answer takes the right pane from the threads, the tree, the pipeline and the outline.
    fn showing_answer(self, answer: Answer) -> Self {
        Self { answer: Some(answer), pane: None, tree: None, pipeline: None, outline: None, ..self }
    }

    fn without_answer(self) -> Self {
        Self { answer: None, ..self }
    }
}

/// Where an answer can become a draft.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Lines(Box<Position>),
    Nowhere,
}

/// What a piece of Claude's stream brings to the app.
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    Text(String),
    /// The model declined and another one starts over: the partial text is void.
    Restart,
    /// Finished; `text` is the whole answer when it came from the cache.
    Done {
        outcome: Outcome,
        cached_text: Option<String>,
    },
    Failed(String),
}

impl App {
    /// `a` then a letter: `a` asks anything about the hunk, file or lines under the cursor, `h` lists the answers kept.
    pub(super) fn ask_key(&mut self, c: char) -> Vec<Action> {
        if c == 'h' {
            return self.open.as_ref().map(|o| vec![Action::LoadAnswers(o.key.clone())]).unwrap_or_default();
        }
        if self.ask_model.is_none() {
            self.warn("Claude is off · set [ai.anthropic] enabled = true and run revu ai login anthropic");
            return vec![];
        }
        let Some(open) = &self.open else { return vec![] };
        if c != 'a' {
            self.toast("a then a asks anything · h answers kept · g agent session");
            return vec![];
        }
        let scope = scope_at_cursor(open);
        self.question_box(Input::Ask { scope: Box::new(scope), target: Box::new(self.lines_target()) })
    }

    /// The first question of a conversation, built from the review as it is now.
    pub(super) fn ask(&mut self, scope: &Scope, prompt: &Prompt, target: Target) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let request = context::ask(&open.review, scope, prompt, context::BUDGET);
        let label = format!("{} · {}", label_of(prompt), scope_label(scope));
        self.start_answer(label, request, target, false)
    }

    /// `:ask <question>`: about what `a a` would ask about.
    pub(super) fn ask_free(&mut self, question: String) -> Vec<Action> {
        if self.ask_model.is_none() {
            self.warn("Claude is off · set [ai.anthropic] enabled = true and run revu ai login anthropic");
            return vec![];
        }
        let Some(open) = &self.open else {
            self.toast("open an MR first");
            return vec![];
        };
        let scope = scope_at_cursor(open);
        let target = self.lines_target();
        self.ask(&scope, &Prompt::Free(question), target)
    }

    /// `enter` in the answer: the next question goes on the same conversation.
    pub(super) fn follow_up(&mut self, question: String) -> Vec<Action> {
        let Some(answer) = self.open.as_ref().and_then(|o| o.answer.as_ref()) else { return vec![] };
        let mut request = answer.request.clone();
        request.turns.push(Turn { role: Role::Assistant, text: answer.text.clone() });
        request.turns.push(Turn { role: Role::User, text: question });
        let (label, target) = (answer.label.clone(), answer.target.clone());
        self.start_answer(label, request, target, false)
    }

    fn start_answer(&mut self, label: String, request: Ask, target: Target, fresh: bool) -> Vec<Action> {
        if !self.asked {
            self.asked = true;
            let model = self.ask_model.clone().unwrap_or_default();
            self.toast(format!("asking Claude ({model}) · this MR goes to Anthropic"));
        }
        let Some(open) = &self.open else { return vec![] };
        self.next_answer += 1;
        let answer = Answer {
            id: self.next_answer,
            label,
            request: request.clone(),
            text: String::new(),
            state: AnswerState::Streaming,
            cached: false,
            scroll: 0,
            target,
        };
        let (key, head) = (open.key.clone(), open.review.mr.refs.head.clone());
        let ask = Action::Ask { key, id: self.next_answer, request: Box::new(request), fresh, label: answer.label.clone(), head };
        self.update_open(|open| open.showing_answer(answer));
        self.focus = Focus::Side;
        vec![ask]
    }

    /// The answers kept for the open MR, listed in the search box to pick one; a word when none is.
    pub(super) fn apply_past_answers(&mut self, key: &MrKey, answers: Vec<PastAnswer>) {
        if self.open.as_ref().is_none_or(|o| o.key != *key) {
            return;
        }
        if answers.is_empty() {
            self.toast("no answer kept for this MR yet · a then e, r, s, t, c or a asks");
            return;
        }
        self.past_answers = answers;
        self.open_palette(crate::tui::palette::Mode::Answers);
    }

    /// A kept answer back in the pane, as it was, ready for a follow-up.
    pub(super) fn show_past_answer(&mut self, index: usize) {
        let Some(past) = self.past_answers.get(index).filter(|_| self.open.is_some()) else { return };
        self.next_answer += 1;
        let answer = Answer {
            id: self.next_answer,
            label: past.label.clone(),
            request: past.request.clone(),
            text: past.text.clone(),
            state: AnswerState::Done(past.outcome.clone()),
            cached: true,
            scroll: 0,
            target: Target::Nowhere,
        };
        self.update_open(|open| open.showing_answer(answer));
        self.focus = Focus::Side;
    }

    pub(super) fn apply_answer(&mut self, key: &MrKey, id: u64, part: Part) {
        let Some(open) = self.open.take() else { return };
        self.open = Some(open.with_answer_part(key, id, part));
    }

    pub(super) fn handle_answer_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let act = self.act_here(Pane::Answer, key);
        let Some(answer) = self.open.as_ref().and_then(|o| o.answer.as_ref()) else { return vec![] };
        match act {
            Some(Act::MoveDown) => self.scroll_answer(1),
            Some(Act::MoveUp) => self.scroll_answer(-1),
            Some(Act::PageDown) => self.scroll_answer(HALF_PAGE as isize),
            Some(Act::PageUp) => self.scroll_answer(-(HALF_PAGE as isize)),
            Some(Act::FullPageDown) => self.scroll_answer(super::keys::full_page(self.areas.side)),
            Some(Act::FullPageUp) => self.scroll_answer(-super::keys::full_page(self.areas.side)),
            Some(Act::Top) => self.scroll_answer(isize::MIN / 2),
            Some(Act::Bottom) => self.scroll_answer(isize::MAX / 2),
            Some(Act::AnswerCopy) => return vec![Action::Copy { text: answer.text.clone(), done: "copied".into() }],
            Some(Act::AnswerDraft) => self.draft_from_answer(&answer.clone()),
            Some(Act::AnswerFollowUp) if answer.state != AnswerState::Streaming => return self.question_box(Input::FollowUp),
            Some(Act::AnswerAgain) => {
                let (label, request, target) = (answer.label.clone(), answer.request.clone(), answer.target.clone());
                return self.start_answer(label, request, target, true);
            }
            Some(Act::Back | Act::ClosePane) => self.close_answer(),
            _ => self.did_nothing(),
        }
        vec![]
    }

    fn scroll_answer(&mut self, delta: isize) {
        self.update_open(|open| {
            let answer = open.answer.map(|answer| {
                let scroll = (answer.scroll as isize).saturating_add(delta).max(0) as usize;
                Answer { scroll, ..answer }
            });
            Open { answer, ..open }
        });
    }

    pub(super) fn close_answer(&mut self) {
        self.update_open(Open::without_answer);
        self.focus = Focus::Review;
    }

    /// `c`: the answer, editable first, as a draft on the lines asked about or as a reply in the thread.
    fn draft_from_answer(&mut self, answer: &Answer) {
        if answer.text.trim().is_empty() {
            self.toast("nothing to turn into a comment yet");
            return;
        }
        match &answer.target {
            Target::Lines(position) => self.open_input(Input::Comment { position: position.clone() }, &answer.text),
            Target::Nowhere => self.toast("a comment needs a line: ask with a a on one"),
        }
    }

    /// The compose box for a question, shown in the answer pane.
    fn question_box(&mut self, input: Input) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        if open.answer.is_none() {
            let waiting = Answer {
                id: 0,
                label: "ask".into(),
                request: Ask { system: vec![], turns: vec![] },
                text: String::new(),
                state: AnswerState::Done(Outcome { stop: anthropic::Stop::Done, usage: anthropic::Usage::default(), model: String::new() }),
                cached: false,
                scroll: 0,
                target: Target::Nowhere,
            };
            self.update_open(|open| open.showing_answer(waiting));
        }
        self.buffer = crate::tui::field::Field::default();
        self.compose_from = Focus::Side;
        self.focus = Focus::Side;
        self.input = Some(input);
        vec![]
    }

    /// The typed text of a question box: a concern for a comment, a free question, or a follow-up.
    pub(super) fn submit_question(&mut self, input: Input, text: String) -> Vec<Action> {
        match input {
            Input::Ask { scope, target, .. } => self.ask(&scope, &Prompt::Free(text), *target),
            Input::FollowUp => self.follow_up(text),
            _ => vec![],
        }
    }

    /// `:ai off`: nothing more goes to an AI provider until `:ai on` or `revu` starts again.
    pub(super) fn ai_off(&mut self) {
        self.ask_model = None;
        self.triage = false;
        self.update_open(Open::without_answer);
        self.toast("AI off: :ai on brings it back");
    }

    /// `:ai on`: back to what the config switched on, after `:ai off` or a Jev failure.
    pub(super) fn ai_on(&mut self) {
        let (triage, ask) = self.ai_configured.clone();
        if !triage && ask.is_none() {
            self.toast("no AI provider is on in the config: see revu ai status");
            return;
        }
        let names: Vec<&str> = [triage.then_some("Jev"), ask.as_ref().map(|_| "Claude")].into_iter().flatten().collect();
        self.triage = triage;
        self.ask_model = ask;
        self.toast(format!("AI on: {}", names.join(" and ")));
    }

    fn lines_target(&self) -> Target {
        self.position_here().map_or(Target::Nowhere, |p| Target::Lines(Box::new(p)))
    }
}

/// What `a e` and `a a` are about: the hunk under the cursor, its file on a file row, else the MR.
fn scope_at_cursor(open: &Open) -> Scope {
    match open.row() {
        Some(Row::Line { file, hunk, .. } | Row::Pair { file, hunk, .. } | Row::Hunk { file, index: hunk, .. }) => {
            Scope::Hunk(open.review.files[*file].new_path.clone(), *hunk)
        }
        Some(Row::File { index, .. }) => Scope::File(open.review.files[*index].new_path.clone()),
        _ => Scope::Mr,
    }
}

fn label_of(prompt: &Prompt) -> &'static str {
    match prompt {
        Prompt::Explain => "explain",
        Prompt::Summary => "summary",
        Prompt::Free(_) => "ask",
    }
}

fn scope_label(scope: &Scope) -> String {
    let name = |path: &str| path.rsplit('/').next().unwrap_or(path).to_owned();
    match scope {
        Scope::Mr => "the MR".into(),
        Scope::File(path) => name(path),
        Scope::Hunk(path, index) => format!("{} hunk {}", name(path), index + 1),
        Scope::Lines(path, lines) if lines.start() == lines.end() => format!("{}:{}", name(path), lines.start()),
        Scope::Lines(path, lines) => format!("{}:{}–{}", name(path), lines.start(), lines.end()),
    }
}

#[cfg(test)]
mod tests {
    use crate::tui::app::test_support::*;

    fn answer(app: &App) -> Answer {
        app.open.as_ref().unwrap().answer.clone().expect("an answer holds the pane")
    }

    /// `a a`, then the question typed and sent.
    fn ask_here(app: &mut App, question: &str) -> Vec<Action> {
        press(app, "aa");
        type_text(app, question)
    }

    #[test]
    fn a_says_claude_is_off_until_the_config_and_a_key_switch_it_on() {
        let mut app = with_review();
        on_line(&mut app);
        assert_eq!(press(&mut app, "aa"), [] as [Action; 0]);
        assert!(app.live_toast().is_some_and(|t| t.text.contains("[ai.anthropic] enabled = true")));
    }

    #[test]
    fn a_a_asks_about_the_hunk_under_the_cursor_and_streams_into_the_pane() {
        let mut app = asking();
        on_line(&mut app);
        let (id, request, fresh) = the_ask(&ask_here(&mut app, "what does it do?"));
        assert!(!fresh);
        assert_eq!(request.system.len(), 3, "rules, the MR, the hunk");
        assert!(request.system[2].text.contains("@@"));
        assert_eq!(request.turns.last().unwrap().text, "what does it do?");
        assert_eq!(app.focus, Focus::Side);
        assert!(
            app.live_toast().is_some_and(|t| t.text.contains("claude-opus-5") && t.text.contains("Anthropic")),
            "the first question says where the MR goes"
        );
        app.apply(Incoming::Answer { key: mr_key(), id, part: Part::Text("It adds ".into()) });
        app.apply(Incoming::Answer { key: mr_key(), id, part: Part::Restart });
        app.apply(Incoming::Answer { key: mr_key(), id, part: Part::Text("a retry.".into()) });
        app.apply(Incoming::Answer { key: mr_key(), id: id + 7, part: Part::Text(" stale".into()) });
        assert_eq!(answer(&app).text, "a retry.", "a restart voids the partial text and an older stream is ignored");
        app.apply(Incoming::Answer { key: mr_key(), id, part: Part::Done { outcome: done("claude-opus-5"), cached_text: None } });
        assert_eq!(answer(&app).state, AnswerState::Done(done("claude-opus-5")));
        let screen = render(&mut app, 150, 24);
        assert!(screen.contains("Claude · ask") && screen.contains("a retry.") && screen.contains("1.2k cached"), "{screen}");
    }

    #[test]
    fn pieces_waiting_together_apply_in_order_before_the_next_draw() {
        let mut app = asking();
        on_line(&mut app);
        let (id, _, _) = the_ask(&ask_here(&mut app, "why?"));
        app.take_actions();
        let piece = |text: &str| Incoming::Answer { key: mr_key(), id, part: Part::Text(text.into()) };
        let actions = app.apply_all([piece("one "), piece("two "), piece("three")]);
        assert!(actions.is_empty(), "{actions:?}");
        assert_eq!(answer(&app).text, "one two three");
    }

    #[test]
    fn an_answer_becomes_a_draft_on_the_lines_asked_about() {
        let mut app = asking();
        on_line(&mut app);
        let (id, ..) = the_ask(&ask_here(&mut app, "why?"));
        app.apply(Incoming::Answer {
            key: mr_key(),
            id,
            part: Part::Done { outcome: done("claude-opus-5"), cached_text: Some("Rename this.".into()) },
        });
        assert!(answer(&app).cached);
        press(&mut app, "c");
        assert!(matches!(app.input, Some(Input::Comment { .. })), "{:?}", app.input);
        assert_eq!(app.buffer.text(), "Rename this.", "editable before it is saved");
        let actions = app.handle_key(code(KeyCode::Enter));
        assert!(matches!(actions.as_slice(), [Action::SaveDraft { .. }]), "{actions:?}");
    }

    #[test]
    fn a_follow_up_carries_the_conversation_and_r_asks_again_fresh() {
        let mut app = asking();
        on_line(&mut app);
        let (id, ..) = the_ask(&ask_here(&mut app, "why?"));
        app.apply(Incoming::Answer { key: mr_key(), id, part: Part::Text("First answer.".into()) });
        app.apply(Incoming::Answer { key: mr_key(), id, part: Part::Done { outcome: done("claude-opus-5"), cached_text: None } });
        app.handle_key(code(KeyCode::Enter));
        assert_eq!(app.input, Some(Input::FollowUp));
        let (next, request, _) = the_ask(&type_text(&mut app, "and the tests?"));
        assert!(next > id);
        let roles: Vec<_> = request.turns.iter().map(|t| (t.role, t.text.as_str())).collect();
        assert_eq!(
            roles[1..],
            [(crate::ai::anthropic::Role::Assistant, "First answer."), (crate::ai::anthropic::Role::User, "and the tests?")]
        );
        let (_, again, fresh) = the_ask(&press(&mut app, "R"));
        assert!(fresh && again == request, "R asks the same thing past the cache");
    }

    #[test]
    fn ai_on_brings_back_what_the_config_switched_on() {
        let mut app = asking();
        app.ai_configured = (true, app.ask_model.clone());
        press(&mut app, ":ai off");
        app.handle_key(code(KeyCode::Enter));
        assert_eq!((app.ask_model.is_some(), app.triage), (false, false));
        press(&mut app, ":ai on");
        app.handle_key(code(KeyCode::Enter));
        assert_eq!((app.ask_model.is_some(), app.triage), (true, true));
    }

    #[test]
    fn ai_on_without_a_provider_in_the_config_says_so() {
        let mut app = asking();
        app.ai_configured = (false, None);
        press(&mut app, ":ai on");
        app.handle_key(code(KeyCode::Enter));
        assert!(app.ask_model.is_some(), "nothing changes");
        assert!(app.live_toast().unwrap().text.contains("revu ai status"));
    }

    #[test]
    fn ai_off_stops_every_ai_call_for_the_session() {
        let mut app = asking();
        app.triage = true;
        on_line(&mut app);
        press(&mut app, ":ai off");
        app.handle_key(code(KeyCode::Enter));
        assert_eq!((app.ask_model.as_deref(), app.triage), (None, false));
        assert_eq!(press(&mut app, "aa"), [] as [Action; 0]);
        press(&mut app, ":ask why");
        assert_eq!(app.handle_key(code(KeyCode::Enter)), [] as [Action; 0]);
    }

    fn past(label: &str, head: &str, asked_at: DateTime<Utc>, text: &str) -> PastAnswer {
        let request = crate::ai::anthropic::Ask {
            system: vec![],
            turns: vec![crate::ai::anthropic::Turn { role: crate::ai::anthropic::Role::User, text: "Explain this hunk".into() }],
        };
        PastAnswer { label: label.into(), asked_at, head: head.into(), request, text: text.into(), outcome: done("claude-opus-5") }
    }

    #[test]
    fn a_h_lists_the_kept_answers_and_enter_brings_one_back_ready_for_a_follow_up() {
        let mut app = asking();
        on_line(&mut app);
        assert_eq!(press(&mut app, "ah"), vec![Action::LoadAnswers(mr_key())]);
        let head = app.open.as_ref().unwrap().review.mr.refs.head.clone();
        let hours_ago = |h: i64| app.today - chrono::Duration::hours(h);
        let answers = vec![
            past("explain · charge.rs", head.as_str(), hours_ago(1), "It retries."),
            past("summary", "01de51", hours_ago(30), "Cards charged once."),
        ];
        app.apply(Incoming::PastAnswers { key: mr_key(), answers });
        let screen = render(&mut app, 150, 30);
        assert!(screen.contains("explain · charge.rs") && screen.contains("summary"), "{screen}");
        assert!(screen.contains("1d · older push"), "an answer from before the last push says so:\n{screen}");
        press(&mut app, "sum");
        app.handle_key(code(KeyCode::Enter));
        let shown = answer(&app);
        assert_eq!((shown.label.as_str(), shown.text.as_str(), shown.cached), ("summary", "Cards charged once.", true));
        assert_eq!(app.focus, Focus::Side);
        app.handle_key(code(KeyCode::Enter));
        let actions = type_text(&mut app, "why once?");
        let (_, request, _) = the_ask(&actions);
        assert_eq!(request.turns.len(), 3, "the kept question, its answer, the follow-up");
    }

    #[test]
    fn a_h_with_nothing_kept_says_how_to_ask() {
        let mut app = with_review();
        press(&mut app, "ah");
        app.apply(Incoming::PastAnswers { key: mr_key(), answers: vec![] });
        assert!(app.palette.is_none());
        assert!(app.live_toast().unwrap().text.contains("no answer kept"));
    }
}
