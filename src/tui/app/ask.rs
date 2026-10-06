//! `a`: questions to Claude about what is under the cursor, answered in the right pane.
use super::{Action, App, Focus, Input, MrKey, Open};
use crate::ai::anthropic::{self, Ask, Outcome, Role, Turn};
use crate::ai::context::{self, Prompt, Scope};
use crate::forge::Position;
use crate::review::Row;
use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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
    Thread(String),
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
    /// `a` then a letter: the question, about the hunk, file, thread or lines under the cursor.
    pub(super) fn ask_key(&mut self, c: char) -> Vec<Action> {
        if c == 'h' {
            return self.open.as_ref().map(|o| vec![Action::LoadAnswers(o.key.clone())]).unwrap_or_default();
        }
        if self.ask_model.is_none() {
            self.warn("Claude is off · set [ai.anthropic] enabled = true and run revu ai login anthropic");
            return vec![];
        }
        let Some(open) = &self.open else { return vec![] };
        let (scope, target) = match c {
            'e' | 'a' => (here(open), self.lines_target()),
            'r' => (file_here(open).map_or(Scope::Mr, Scope::File), Target::Nowhere),
            's' => (Scope::Mr, Target::Nowhere),
            't' => {
                let Some(id) = open.focused_thread().or_else(|| thread_on_line(open)) else {
                    self.toast("no thread here · a t works on a line with ◆");
                    return vec![];
                };
                (Scope::Thread(id.clone()), Target::Thread(id))
            }
            'c' => (lines_here(open, self.position_here().as_ref()), self.lines_target()),
            _ => {
                self.toast("a then e explain · r risks · s summary · t thread · c comment · a ask");
                return vec![];
            }
        };
        match c {
            'e' => self.ask(&scope, &Prompt::Explain, target),
            'r' => self.ask(&scope, &Prompt::Risks, target),
            's' => self.ask(&scope, &Prompt::Summary, target),
            't' => self.ask(&scope, &Prompt::Thread, target),
            'c' => self.question_box(Input::Ask { scope: Box::new(scope), concern: true, target: Box::new(target) }),
            _ => self.question_box(Input::Ask { scope: Box::new(scope), concern: false, target: Box::new(target) }),
        }
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
        let scope = here(open);
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
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(answer) = self.open.as_ref().and_then(|o| o.answer.as_ref()) else { return vec![] };
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.scroll_answer(1),
            KeyCode::Char('k') | KeyCode::Up => self.scroll_answer(-1),
            KeyCode::Char('d') if ctrl => self.scroll_answer(HALF_PAGE as isize),
            KeyCode::Char('u') if ctrl => self.scroll_answer(-(HALF_PAGE as isize)),
            KeyCode::Char('f') if ctrl => self.scroll_answer(super::keys::full_page(self.areas.side)),
            KeyCode::Char('b') if ctrl => self.scroll_answer(-super::keys::full_page(self.areas.side)),
            KeyCode::Char('g') => self.scroll_answer(isize::MIN / 2),
            KeyCode::Char('G') => self.scroll_answer(isize::MAX / 2),
            KeyCode::Char('y') => return vec![Action::Yank(answer.text.clone())],
            KeyCode::Char('c') => self.draft_from_answer(&answer.clone()),
            KeyCode::Enter if answer.state != AnswerState::Streaming => return self.question_box(Input::FollowUp),
            KeyCode::Char('R') => {
                let (label, request, target) = (answer.label.clone(), answer.request.clone(), answer.target.clone());
                return self.start_answer(label, request, target, true);
            }
            KeyCode::Esc | KeyCode::Char('x') => self.close_answer(),
            _ => {}
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
            Target::Thread(id) => self.open_input(Input::Reply { thread: id.clone() }, &answer.text),
            Target::Nowhere => self.toast("a comment needs a line: ask with a e or a c on one, or a t on a thread"),
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
            Input::Ask { scope, concern: true, target } => self.ask(&scope, &Prompt::Comment(text), *target),
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
fn here(open: &Open) -> Scope {
    match open.row() {
        Some(Row::Line { file, hunk, .. } | Row::Pair { file, hunk, .. } | Row::Hunk { file, index: hunk, .. }) => {
            Scope::Hunk(open.review.files[*file].new_path.clone(), *hunk)
        }
        Some(Row::File { index, .. }) => Scope::File(open.review.files[*index].new_path.clone()),
        _ => Scope::Mr,
    }
}

fn file_here(open: &Open) -> Option<String> {
    open.row().and_then(Row::file).map(|i| open.review.files[i].new_path.clone())
}

/// The new-side lines of the position, when it has some; else the hunk around the cursor.
fn lines_here(open: &Open, position: Option<&Position>) -> Scope {
    let numbers = position.and_then(|p| Some((p.start.and_then(|s| s.new).or(p.line.new)?, p.line.new?, p.new_path.clone())));
    match numbers {
        Some((from, to, path)) => Scope::Lines(path, from.min(to)..=from.max(to)),
        None => here(open),
    }
}

fn thread_on_line(open: &Open) -> Option<String> {
    let place = open.review.place_of(open.row()?)?;
    open.review.conversations(&place).into_iter().find_map(|c| c.thread)
}

fn label_of(prompt: &Prompt) -> &'static str {
    match prompt {
        Prompt::Explain => "explain",
        Prompt::Risks => "risks",
        Prompt::Summary => "summary",
        Prompt::Thread => "thread",
        Prompt::Comment(_) => "comment",
        Prompt::Free(_) => "ask",
    }
}

fn scope_label(scope: &Scope) -> String {
    let name = |path: &str| path.rsplit('/').next().unwrap_or(path).to_owned();
    match scope {
        Scope::Mr => "the MR".into(),
        Scope::File(path) => name(path),
        Scope::Hunk(path, index) => format!("{} hunk {}", name(path), index + 1),
        Scope::Thread(_) => "thread".into(),
        Scope::Lines(path, lines) if lines.start() == lines.end() => format!("{}:{}", name(path), lines.start()),
        Scope::Lines(path, lines) => format!("{}:{}–{}", name(path), lines.start(), lines.end()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crate::tui::app::test_support::*;

    fn answer(app: &App) -> Answer {
        app.open.as_ref().unwrap().answer.clone().expect("an answer holds the pane")
    }

    #[test]
    fn a_says_claude_is_off_until_the_config_and_a_key_switch_it_on() {
        let mut app = with_review();
        on_line(&mut app);
        assert_eq!(press(&mut app, "ae"), [] as [Action; 0]);
        assert!(app.live_toast().is_some_and(|t| t.text.contains("[ai.anthropic] enabled = true")));
    }

    #[test]
    fn a_e_explains_the_hunk_under_the_cursor_and_streams_into_the_pane() {
        let mut app = asking();
        on_line(&mut app);
        let (id, request, fresh) = the_ask(&press(&mut app, "ae"));
        assert!(!fresh);
        assert_eq!(request.system.len(), 3, "rules, the MR, the hunk");
        assert!(request.system[2].text.contains("@@"));
        assert_eq!(request.turns.last().unwrap().text, crate::ai::context::Prompt::Explain.question());
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
        assert!(screen.contains("Claude · explain") && screen.contains("a retry.") && screen.contains("1.2k cached"), "{screen}");
    }

    #[test]
    fn pieces_waiting_together_apply_in_order_before_the_next_draw() {
        let mut app = asking();
        on_line(&mut app);
        let (id, _, _) = the_ask(&press(&mut app, "ae"));
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
        let (id, ..) = the_ask(&press(&mut app, "ae"));
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
        let (id, ..) = the_ask(&press(&mut app, "ae"));
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
    fn a_c_asks_for_the_concern_then_drafts_a_comment_about_it() {
        let mut app = asking();
        on_line(&mut app);
        assert_eq!(press(&mut app, "ac"), [] as [Action; 0]);
        assert_eq!(app.input_label(), "comment about");
        let (_, request, _) = the_ask(&type_text(&mut app, "naming"));
        assert!(request.turns[0].text.contains("about: naming."));
        assert!(matches!(answer(&app).target, ask::Target::Lines(_)));
    }

    #[test]
    fn a_t_summarises_the_thread_and_its_answer_becomes_a_reply() {
        let mut app = asking();
        press(&mut app, "]c");
        app.handle_key(code(KeyCode::Enter));
        app.handle_key(code(KeyCode::Enter));
        press(&mut app, "]N");
        let (id, request, _) = the_ask(&press(&mut app, "at"));
        assert!(request.system[2].text.contains("## Thread"));
        app.apply(Incoming::Answer {
            key: mr_key(),
            id,
            part: Part::Done { outcome: done("claude-opus-5"), cached_text: Some("Waits on nina.".into()) },
        });
        press(&mut app, "c");
        assert!(matches!(app.input, Some(Input::Reply { .. })), "{:?}", app.input);
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
        assert_eq!(press(&mut app, "ae"), [] as [Action; 0]);
        press(&mut app, ":ask why");
        assert_eq!(app.handle_key(code(KeyCode::Enter)), [] as [Action; 0]);
    }

    fn past(label: &str, head: &str, asked_at: chrono::DateTime<chrono::Utc>, text: &str) -> PastAnswer {
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
            past("summary", "older1", hours_ago(30), "Cards charged once."),
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
