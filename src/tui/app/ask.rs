//! The pane Claude's answers showed in; nothing opens it any more.
use super::{Action, App, Focus, Open};
use crate::act::{Act, Pane};
use crate::ai::anthropic::Outcome;
use crossterm::event::KeyEvent;

const HALF_PAGE: usize = 10;

/// Claude's answer in the right pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Answer {
    /// `ask · charge.rs`, the pane's title.
    pub(crate) label: String,
    pub(crate) text: String,
    pub(crate) state: AnswerState,
    pub(crate) scroll: usize,
}

#[expect(dead_code, reason = "nothing opens the pane; the next change in the stack deletes it")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AnswerState {
    Streaming,
    Done(Outcome),
    Failed(String),
}

impl Open {
    fn without_answer(self) -> Self {
        Self { answer: None, ..self }
    }
}

impl App {
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
}
