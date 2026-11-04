//! `a`: questions to Claude about what is under the cursor, answered in the right pane.
use super::{Action, App, Focus, Input, MrKey, Open};
use crate::act::{Act, Pane};
use crate::ai::anthropic::{self, Outcome};
use crate::ai::context::{self, Prompt, Scope};
use crate::review::Row;
use crossterm::event::KeyEvent;

const HALF_PAGE: usize = 10;

/// Claude's answer in the right pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Answer {
    /// Grows with every question, so pieces of an older stream can be told apart and dropped.
    pub(crate) id: u64,
    /// `ask · charge.rs`, the pane's title.
    pub(crate) label: String,
    pub(crate) text: String,
    pub(crate) state: AnswerState,
    pub(crate) scroll: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AnswerState {
    Streaming,
    Done(Outcome),
    Failed(String),
}

impl Answer {
    fn with_part(self, part: Part) -> Self {
        match part {
            Part::Text(more) => Self { text: self.text + &more, ..self },
            Part::Restart => Self { text: String::new(), ..self },
            Part::Done(outcome) => Self { state: AnswerState::Done(outcome), ..self },
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

    pub(super) fn without_answer(self) -> Self {
        Self { answer: None, ..self }
    }
}

/// What a piece of Claude's stream brings to the app.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Part {
    Text(String),
    /// The model declined and another one starts over: the partial text is void.
    Restart,
    Done(Outcome),
    Failed(String),
}

impl App {
    /// `a` then a letter: `a` asks anything about the hunk, file or lines under the cursor.
    pub(super) fn ask_key(&mut self, c: char) -> Vec<Action> {
        if self.ask_model.is_none() {
            self.warn("Claude is off · set [ai.anthropic] enabled = true and run revu ai login anthropic");
            return vec![];
        }
        let Some(open) = &self.open else { return vec![] };
        if c != 'a' {
            self.toast("a then a asks anything · g agent session");
            return vec![];
        }
        let scope = scope_at_cursor(open);
        self.question_box(Input::Ask { scope: Box::new(scope) })
    }

    /// The question, built from the review as it is now.
    pub(super) fn ask(&mut self, scope: &Scope, question: String) -> Vec<Action> {
        if !self.asked {
            self.asked = true;
            let model = self.ask_model.clone().unwrap_or_default();
            self.toast(format!("asking Claude ({model}) · this MR goes to Anthropic"));
        }
        let Some(open) = &self.open else { return vec![] };
        let request = context::ask(&open.review, scope, &Prompt::Free(question), context::BUDGET);
        self.next_answer += 1;
        let answer = Answer {
            id: self.next_answer,
            label: format!("ask · {}", scope_label(scope)),
            text: String::new(),
            state: AnswerState::Streaming,
            scroll: 0,
        };
        let ask = Action::Ask { key: open.key.clone(), id: self.next_answer, request: Box::new(request) };
        self.update_open(|open| open.showing_answer(answer));
        self.focus = Focus::Side;
        vec![ask]
    }

    pub(super) fn apply_answer(&mut self, key: &MrKey, id: u64, part: Part) {
        let Some(open) = self.open.take() else { return };
        self.open = Some(open.with_answer_part(key, id, part));
    }

    pub(super) fn handle_answer_key(&mut self, key: KeyEvent) -> Vec<Action> {
        match self.act_here(Pane::Answer, key) {
            Some(Act::MoveDown) => self.scroll_answer(1),
            Some(Act::MoveUp) => self.scroll_answer(-1),
            Some(Act::PageDown) => self.scroll_answer(HALF_PAGE as isize),
            Some(Act::PageUp) => self.scroll_answer(-(HALF_PAGE as isize)),
            Some(Act::FullPageDown) => self.scroll_answer(super::keys::full_page(self.areas.side)),
            Some(Act::FullPageUp) => self.scroll_answer(-super::keys::full_page(self.areas.side)),
            Some(Act::Top) => self.scroll_answer(isize::MIN / 2),
            Some(Act::Bottom) => self.scroll_answer(isize::MAX / 2),
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

    /// The compose box for a question, shown in the answer pane.
    fn question_box(&mut self, input: Input) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        if open.answer.is_none() {
            let waiting = Answer {
                id: 0,
                label: "ask".into(),
                text: String::new(),
                state: AnswerState::Done(Outcome { stop: anthropic::Stop::Done, usage: anthropic::Usage::default(), model: String::new() }),
                scroll: 0,
            };
            self.update_open(|open| open.showing_answer(waiting));
        }
        self.buffer = vtui::field::Field::default();
        self.compose_from = Focus::Side;
        self.focus = Focus::Side;
        self.input = Some(input);
        vec![]
    }
}

/// What `a a` is about: the hunk under the cursor, its file on a file row, else the MR.
fn scope_at_cursor(open: &Open) -> Scope {
    match open.row() {
        Some(Row::Line { file, hunk, .. } | Row::Pair { file, hunk, .. } | Row::Hunk { file, index: hunk, .. }) => {
            Scope::Hunk(open.review.files[*file].new_path.clone(), *hunk)
        }
        Some(Row::File { index, .. }) => Scope::File(open.review.files[*index].new_path.clone()),
        _ => Scope::Mr,
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
        let (id, request) = the_ask(&ask_here(&mut app, "what does it do?"));
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
        app.apply(Incoming::Answer { key: mr_key(), id, part: Part::Done(done("claude-opus-5")) });
        assert_eq!(answer(&app).state, AnswerState::Done(done("claude-opus-5")));
        let screen = render(&mut app, 150, 24);
        assert!(screen.contains("Claude · ask") && screen.contains("a retry.") && screen.contains("1.2k cached"), "{screen}");
    }

    #[test]
    fn pieces_waiting_together_apply_in_order_before_the_next_draw() {
        let mut app = asking();
        on_line(&mut app);
        let (id, _) = the_ask(&ask_here(&mut app, "why?"));
        app.take_actions();
        let piece = |text: &str| Incoming::Answer { key: mr_key(), id, part: Part::Text(text.into()) };
        let actions = app.apply_all([piece("one "), piece("two "), piece("three")]);
        assert!(actions.is_empty(), "{actions:?}");
        assert_eq!(answer(&app).text, "one two three");
    }
}
