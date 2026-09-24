//! The one-row input under the panes: what it is for, and what happens to the text on `enter`.
use super::{Action, App, Input, MrKey, Open, Post};
use crate::review::{Draft, Place, Review};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

impl App {
    /// Opens the compose box on `input`, in the pane at its line; `text` prefills it, else the text
    /// left unsent for that target earlier comes back.
    pub(super) fn open_input(&mut self, input: Input, text: &str) {
        let kept = self.unsent.remove(&target_key(&input));
        let text = if text.is_empty() { kept.unwrap_or_default() } else { text.to_owned() };
        self.buffer = crate::tui::field::Field::new(text);
        self.compose_from = self.focus;
        self.show_target(&input);
        self.input = Some(input);
    }

    /// The pane shows where the text goes: the line of a new thread, the thread of a reply.
    fn show_target(&mut self, input: &Input) {
        let Some(open) = &self.open else { return };
        let review = &open.review;
        let place = match input {
            Input::Comment { position } => place_of_position(review, position),
            Input::Reply { .. } if open.pane.is_some() => None,
            Input::Reply { thread } => Some(place_of_thread(review, thread)),
            Input::Ask { .. } | Input::FollowUp => None,
            Input::EditDraft { index } => review.drafts.get(*index).map(|draft| match (&draft.position, &draft.reply_to) {
                (Some(position), _) => place_of_position(review, position).unwrap_or(Place::Mr),
                (None, Some(thread)) => place_of_thread(review, thread),
                (None, None) => Place::Mr,
            }),
        };
        match place {
            Some(place) if open.pane.as_ref().is_none_or(|p| p.place != place) => self.open_pane(place),
            _ => self.focus = super::Focus::Side,
        }
    }

    pub(super) fn handle_input_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let newline = key.modifiers.intersects(KeyModifiers::ALT | KeyModifiers::SHIFT);
        let post = key.modifiers.contains(KeyModifiers::SUPER);
        match key.code {
            KeyCode::Esc => self.leave_input(),
            KeyCode::Enter if post => return self.post_input(),
            KeyCode::Char('s') if ctrl => return self.post_input(),
            KeyCode::Enter if newline => self.buffer.insert('\n'),
            KeyCode::Char('j') if ctrl => self.buffer.insert('\n'),
            KeyCode::Enter => return self.submit_input(),
            KeyCode::Char('o') if ctrl => return self.move_to_editor(),
            KeyCode::Left | KeyCode::Char('b') if alt => self.buffer.word_left(),
            KeyCode::Right | KeyCode::Char('f') if alt => self.buffer.word_right(),
            KeyCode::Backspace if alt => self.buffer.delete_word(),
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

    /// `esc`: the box closes and its text waits for the same target to come back.
    fn leave_input(&mut self) {
        let text = self.buffer.take();
        if let Some(input) = self.input.take()
            && !text.trim().is_empty()
        {
            self.unsent.insert(target_key(&input), text);
        }
        self.focus = self.compose_from;
    }

    /// `ctrl-o`: the text goes on in `$EDITOR`; if the editor gives nothing back, the box keeps it.
    fn move_to_editor(&mut self) -> Vec<Action> {
        let text = self.buffer.take();
        let Some(input) = self.input.take() else { return vec![] };
        if !text.trim().is_empty() {
            self.unsent.insert(target_key(&input), text.clone());
        }
        self.focus = self.compose_from;
        vec![Action::Compose { input, draft: text }]
    }

    /// `⌘enter`, `ctrl-s`: a new thread or a reply goes public at once; any other box saves as
    /// `enter` does. The text waits under its target until the forge takes it, so a failure loses nothing.
    fn post_input(&mut self) -> Vec<Action> {
        let Some(to) = self.input.as_ref().and_then(Post::of) else { return self.submit_input() };
        let Some(key) = self.open.as_ref().map(|o| o.key.clone()) else { return vec![] };
        let body = self.buffer.text().trim().to_owned();
        if body.is_empty() {
            return vec![];
        }
        self.leave_input();
        vec![Action::Post { key, to, body }]
    }

    pub(super) fn apply_posted(&mut self, key: &MrKey, to: &Post) {
        self.unsent.remove(&target_key(&to.input()));
        self.toast("posted");
        if self.open.as_ref().is_some_and(|o| &o.key == key) {
            self.poll.discussions_due = Some(self.now);
        }
    }

    /// The box comes back with the text when nothing else is being written on that MR.
    pub(super) fn post_failed(&mut self, key: &MrKey, to: &Post, message: &str) {
        self.warn(format!("not posted: {message}"));
        if self.input.is_none() && self.open.as_ref().is_some_and(|o| &o.key == key) {
            self.open_input(to.input(), "");
        }
    }

    fn submit_input(&mut self) -> Vec<Action> {
        let text = self.buffer.take().trim().to_owned();
        let Some(input) = self.input.take() else { return vec![] };
        self.focus = self.compose_from;
        if text.is_empty() {
            return vec![];
        }
        self.submit(input, text)
    }

    /// A finished text, from the compose box or the editor, becomes a draft or changes one.
    pub(super) fn submit(&mut self, input: Input, text: String) -> Vec<Action> {
        self.unsent.remove(&target_key(&input));
        let Some(open) = self.open.clone() else { return vec![] };
        match input {
            Input::Comment { position } => self.add_draft(&open, Draft::on(*position, text)),
            Input::Reply { thread } => self.add_draft(&open, Draft::reply(&thread, text)),
            Input::EditDraft { index } => self.change_draft(&open, index, text),
            question @ (Input::Ask { .. } | Input::FollowUp) => self.submit_question(question, text),
        }
    }

    fn add_draft(&mut self, open: &Open, draft: Draft) -> Vec<Action> {
        let mut drafts = open.review.drafts.clone();
        drafts.push(draft.clone());
        let index = drafts.len() - 1;
        self.open = Some(Open { select_from: None, ..open.with_review(open.review.with_drafts(drafts)) });
        vec![Action::SaveDraft { key: open.key.clone(), index, draft: Box::new(draft) }]
    }

    fn change_draft(&mut self, open: &Open, index: usize, text: String) -> Vec<Action> {
        let Some(draft) = open.review.drafts.get(index) else { return vec![] };
        let changed = draft.clone().with_body(text);
        let mut drafts = open.review.drafts.clone();
        drafts[index] = changed.clone();
        self.open = Some(open.with_review(open.review.with_drafts(drafts)));
        match draft.id {
            Some(id) => vec![Action::UpdateDraft { key: open.key.clone(), id, draft: Box::new(changed) }],
            None => vec![],
        }
    }

    /// The compose box's title: `new thread · charge.rs:57`, `new thread · charge.rs:55–57`,
    /// `reply to nina`, `edit draft`.
    pub fn input_label(&self) -> String {
        match &self.input {
            Some(Input::Comment { position }) => {
                let path = position.new_path.as_str();
                let name = path.rsplit('/').next().unwrap_or(path);
                let number = |line: crate::forge::LineRef| match (line.new, line.old) {
                    (Some(n), _) => n.to_string(),
                    (None, Some(o)) => format!("-{o}"),
                    (None, None) => String::new(),
                };
                match position.start {
                    Some(start) => format!("new thread · {name}:{}–{}", number(start), number(position.line)),
                    None => format!("new thread · {name}:{}", number(position.line)),
                }
            }
            Some(Input::Reply { thread }) => {
                let author = self.open.as_ref().and_then(|o| o.review.thread(thread)).map(|t| t.first().author.username.clone());
                format!("reply to {}", author.unwrap_or_else(|| "the thread".into()))
            }
            Some(Input::EditDraft { .. }) => "edit draft".to_owned(),
            Some(Input::Ask { concern: true, .. }) => "comment about".to_owned(),
            Some(Input::Ask { .. }) => "ask Claude".to_owned(),
            Some(Input::FollowUp) => "follow-up".to_owned(),
            None => String::new(),
        }
    }
}

impl Post {
    fn of(input: &Input) -> Option<Self> {
        match input {
            Input::Comment { position } => Some(Post::Thread(position.clone())),
            Input::Reply { thread } => Some(Post::Reply(thread.clone())),
            Input::EditDraft { .. } | Input::Ask { .. } | Input::FollowUp => None,
        }
    }

    fn input(&self) -> Input {
        match self {
            Post::Thread(position) => Input::Comment { position: position.clone() },
            Post::Reply(thread) => Input::Reply { thread: thread.clone() },
        }
    }
}

/// The pane place a position hangs on: its end line, with both numbers it carries.
fn place_of_position(review: &Review, position: &crate::forge::Position) -> Option<Place> {
    let file = review.files.iter().position(|f| f.new_path == position.new_path || f.old_path == position.old_path)?;
    Some(Place::Line { file, new: position.line.new, old: position.line.old })
}

/// Where a thread lives in the pane: its line, the file's outdated threads, or the MR.
fn place_of_thread(review: &Review, id: &str) -> Place {
    let Some(thread) = review.thread(id) else { return Place::Mr };
    let Some(anchor) = &thread.anchor else { return Place::Mr };
    let Some(file) = review.files.iter().position(|f| f.new_path == anchor.path || f.old_path == anchor.path) else { return Place::Mr };
    match (thread.outdated, anchor.side) {
        (true, _) => Place::Outdated { file },
        (false, crate::review::Side::New) => Place::Line { file, new: Some(anchor.line), old: None },
        (false, crate::review::Side::Old) => Place::Line { file, new: None, old: Some(anchor.line) },
    }
}

/// Which unsent text belongs to which target: a line (or range), a thread, a draft.
fn target_key(input: &Input) -> String {
    match input {
        Input::Comment { position } => {
            format!("line {}:{:?}:{:?}:{:?}", position.new_path, position.line, position.start, position.old_path)
        }
        Input::Reply { thread } => format!("reply {thread}"),
        Input::EditDraft { index } => format!("draft {index}"),
        Input::Ask { scope, concern, .. } => format!("ask {concern} {scope:?}"),
        Input::FollowUp => "follow-up".to_owned(),
    }
}
