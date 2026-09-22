//! The one-row input under the panes: what it is for, and what happens to the text on `enter`.
use super::{Action, App, Input, Open};
use crate::review::Draft;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

impl App {
    pub(super) fn open_input(&mut self, input: Input, text: &str) {
        self.buffer = crate::tui::field::Field::new(text);
        self.input = Some(input);
    }

    pub(super) fn handle_input_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => {
                self.input = None;
                self.buffer.clear();
            }
            KeyCode::Enter => return self.submit_input(),
            KeyCode::Left => self.buffer.left(),
            KeyCode::Right => self.buffer.right(),
            KeyCode::Home => self.buffer.start(),
            KeyCode::End => self.buffer.end(),
            KeyCode::Char('a') if ctrl => self.buffer.start(),
            KeyCode::Char('e') if ctrl => self.buffer.end(),
            KeyCode::Char('w') if ctrl => self.buffer.delete_word(),
            KeyCode::Backspace => self.buffer.backspace(),
            KeyCode::Delete => self.buffer.delete(),
            KeyCode::Char(c) if !ctrl => self.buffer.insert(c),
            _ => {}
        }
        vec![]
    }

    fn submit_input(&mut self) -> Vec<Action> {
        let text = self.buffer.take().trim().to_owned();
        let Some(input) = self.input.take() else { return vec![] };
        if text.is_empty() {
            return vec![];
        }
        self.submit(input, text)
    }

    /// A finished text, from the input row or the editor, becomes a draft or changes one.
    pub(super) fn submit(&mut self, input: Input, text: String) -> Vec<Action> {
        let Some(open) = self.open.clone() else { return vec![] };
        match input {
            Input::Comment { position } => self.add_draft(&open, Draft::on(*position, text)),
            Input::Reply { thread } => self.add_draft(&open, Draft::reply(&thread, text)),
            Input::EditDraft { index } => self.change_draft(&open, index, text),
        }
    }

    fn add_draft(&mut self, open: &Open, draft: Draft) -> Vec<Action> {
        let mut drafts = open.review.drafts.clone();
        drafts.push(draft.clone());
        let index = drafts.len() - 1;
        self.open = Some(Open { select_from: None, ..open.with_review(open.review.with_drafts(drafts)) });
        vec![Action::SaveDraft { key: open.key, index, draft: Box::new(draft) }]
    }

    fn change_draft(&mut self, open: &Open, index: usize, text: String) -> Vec<Action> {
        let Some(draft) = open.review.drafts.get(index) else { return vec![] };
        let changed = draft.clone().with_body(text);
        let mut drafts = open.review.drafts.clone();
        drafts[index] = changed.clone();
        self.open = Some(open.with_review(open.review.with_drafts(drafts)));
        match draft.id {
            Some(id) => vec![Action::UpdateDraft { key: open.key, id, draft: Box::new(changed) }],
            None => vec![],
        }
    }

    /// The word before the caret in the input row: `comment charge.rs:13`, `reply`, `edit`.
    pub fn input_label(&self) -> String {
        match &self.input {
            Some(Input::Comment { position }) => {
                let path = position.new_path.as_deref().or(position.old_path.as_deref()).unwrap_or("");
                let name = path.rsplit('/').next().unwrap_or(path);
                match (position.new_line, position.old_line) {
                    (Some(n), _) => format!("comment {name}:{n}"),
                    (None, Some(o)) => format!("comment {name}:-{o}"),
                    (None, None) => "comment".to_owned(),
                }
            }
            Some(Input::Reply { .. }) => "reply".to_owned(),
            Some(Input::EditDraft { .. }) => "edit draft".to_owned(),
            None => String::new(),
        }
    }
}
