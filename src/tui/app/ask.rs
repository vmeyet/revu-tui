//! `a`: questions to Claude about what is under the cursor, answered in the right pane.
use super::{Action, App, Focus, Input, MrKey, Open};
use crate::ai::anthropic::{self, Ask, Outcome, Role, Turn};
use crate::ai::context::{self, Prompt, Scope};
use crate::forge::Position;
use crate::review::Row;
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnswerState {
    Streaming,
    Done(Outcome),
    Failed(String),
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
        let Some(answer) = self.open.as_ref().and_then(|o| o.answer.clone()) else { return vec![] };
        let mut request = answer.request.clone();
        request.turns.push(Turn { role: Role::Assistant, text: answer.text.clone() });
        request.turns.push(Turn { role: Role::User, text: question });
        self.start_answer(answer.label, request, answer.target, false)
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
        let key = open.key.clone();
        self.open = Some(Open { answer: Some(answer), pane: None, tree: None, ..open.clone() });
        self.focus = Focus::Side;
        vec![Action::Ask { key, id: self.next_answer, request: Box::new(request), fresh }]
    }

    pub(super) fn apply_answer(&mut self, key: &MrKey, id: u64, part: Part) {
        let Some(open) = self.open.as_ref().filter(|o| o.key == *key) else { return };
        let Some(answer) = open.answer.clone().filter(|a| a.id == id) else { return };
        let answer = match part {
            Part::Text(more) => Answer { text: answer.text + &more, ..answer },
            Part::Restart => Answer { text: String::new(), ..answer },
            Part::Done { outcome, cached_text } => Answer {
                cached: cached_text.is_some(),
                text: cached_text.unwrap_or(answer.text),
                state: AnswerState::Done(outcome),
                ..answer
            },
            Part::Failed(message) => Answer { state: AnswerState::Failed(message), ..answer },
        };
        self.open = Some(Open { answer: Some(answer), ..open.clone() });
    }

    pub(super) fn handle_answer_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(answer) = self.open.as_ref().and_then(|o| o.answer.clone()) else { return vec![] };
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.scroll_answer(1),
            KeyCode::Char('k') | KeyCode::Up => self.scroll_answer(-1),
            KeyCode::Char('d') if ctrl => self.scroll_answer(HALF_PAGE as isize),
            KeyCode::Char('u') if ctrl => self.scroll_answer(-(HALF_PAGE as isize)),
            KeyCode::Char('g') => self.scroll_answer(isize::MIN / 2),
            KeyCode::Char('G') => self.scroll_answer(isize::MAX / 2),
            KeyCode::Char('y') => return vec![Action::Yank(answer.text)],
            KeyCode::Char('c') => self.draft_from_answer(&answer),
            KeyCode::Enter if answer.state != AnswerState::Streaming => return self.question_box(Input::FollowUp),
            KeyCode::Char('R') => return self.start_answer(answer.label, answer.request, answer.target, true),
            KeyCode::Esc | KeyCode::Char('x') => self.close_answer(),
            _ => {}
        }
        vec![]
    }

    fn scroll_answer(&mut self, delta: isize) {
        let Some(open) = &self.open else { return };
        let Some(answer) = &open.answer else { return };
        let scroll = (answer.scroll as isize).saturating_add(delta).max(0) as usize;
        self.open = Some(Open { answer: Some(Answer { scroll, ..answer.clone() }), ..open.clone() });
    }

    pub(super) fn close_answer(&mut self) {
        if let Some(open) = &self.open {
            self.open = Some(Open { answer: None, ..open.clone() });
        }
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
            self.open = Some(Open { answer: Some(waiting), pane: None, tree: None, ..open.clone() });
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

    /// `:ai off`: nothing more goes to an AI provider until `revu` starts again.
    pub(super) fn ai_off(&mut self) {
        self.ask_model = None;
        self.triage = false;
        if let Some(open) = &self.open {
            self.open = Some(Open { answer: None, ..open.clone() });
        }
        self.toast("AI off until revu starts again");
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
