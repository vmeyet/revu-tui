//! The one-row input under the panes: what it is for, and what happens to the text on `enter`.
use super::{Action, App, Input, MrKey, Open, Post};
use crate::review::{Draft, DraftId, Place, Review};
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
            Input::EditDraft { .. } if open.lists_every_thread() => None,
            Input::Reply { thread } => Some(place_of_thread(review, thread)),
            Input::Ask { .. } | Input::FollowUp => None,
            Input::EditDraft { draft } => {
                review.drafts.iter().find(|d| d.is(*draft)).map(|draft| match (&draft.position, &draft.reply_to) {
                    (Some(position), _) => place_of_position(review, position).unwrap_or(Place::Mr),
                    (None, Some(thread)) => place_of_thread(review, thread),
                    (None, None) => Place::Mr,
                })
            }
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
            KeyCode::Up => self.buffer.up(),
            KeyCode::Down => self.buffer.down(),
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
        let follow = self.reread(key);
        self.composed.extend(follow);
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
        if self.open.is_none() {
            return vec![];
        }
        match input {
            Input::Comment { position } => self.add_draft(Draft::on(*position, text)),
            Input::Reply { thread } => self.add_draft(Draft::reply(&thread, text)),
            Input::EditDraft { draft } => self.change_draft(draft, text),
            question @ (Input::Ask { .. } | Input::FollowUp) => self.submit_question(question, text),
        }
    }

    fn add_draft(&mut self, draft: Draft) -> Vec<Action> {
        let Some(key) = self.open.as_ref().map(|o| o.key.clone()) else { return vec![] };
        let draft = draft.with_local_id(self.new_local_id());
        let save = Action::SaveDraft { key, draft: Box::new(draft.clone()) };
        self.update_open(|open| Open {
            select_from: None,
            ..open.with_drafts_changed(|drafts| drafts.into_iter().chain([draft]).collect())
        });
        vec![save]
    }

    fn change_draft(&mut self, id: DraftId, text: String) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let Some(index) = open.review.drafts.iter().position(|d| d.is(id)) else { return vec![] };
        let draft = &open.review.drafts[index];
        let changed = draft.clone().with_body(text);
        let update = draft.id.map(|id| Action::UpdateDraft { key: open.key.clone(), id, draft: Box::new(changed.clone()) });
        self.update_open(|open| open.with_draft_replaced(index, changed));
        update.into_iter().collect()
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
        Input::EditDraft { draft } => format!("draft {draft:?}"),
        Input::Ask { scope, concern, .. } => format!("ask {concern} {scope:?}"),
        Input::FollowUp => "follow-up".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crate::tui::app::test_support::*;

    #[test]
    fn c_on_a_line_opens_the_input_and_enter_makes_a_draft() {
        let mut app = with_review();
        assert_eq!(press(&mut app, "c"), vec![], "the cursor is on a file row");
        assert!(app.input.is_none() && app.live_toast().is_some());
        on_line(&mut app);
        press(&mut app, "c");
        assert_eq!(app.input_label(), "new thread · charge.rs:12");
        let actions = type_text(&mut app, "nit: rename");
        let [Action::SaveDraft { key, draft }] = actions.as_slice() else { panic!("{actions:?}") };
        assert_eq!(*key, mr_key());
        assert_eq!(draft.body, "nit: rename");
        assert_eq!(draft.position.as_ref().and_then(|p| p.line.new), Some(12));
        assert_eq!(draft.id, None);
        assert!(app.input.is_none());
        let marker = marker_here(&app).expect("the line is marked");
        assert_eq!((marker.mark, marker.unsaved), (crate::review::Mark::Draft, true), "an unsaved draft of mine, no row inserted");
        assert_eq!(app.unsaved_drafts(), 1);
        app.apply(saved(&actions, 9));
        assert_eq!(app.open.as_ref().unwrap().review.drafts[0].id, Some(9));
        assert_eq!(app.unsaved_drafts(), 0);
    }

    #[test]
    fn option_arrows_jump_words_in_the_compose_box_whatever_the_terminal_sends() {
        let mut app = with_review();
        on_line(&mut app);
        press(&mut app, "c");
        press(&mut app, "fix the bug");
        let alt = |code| KeyEvent::new(code, KeyModifiers::ALT);
        app.handle_key(alt(KeyCode::Left));
        app.handle_key(alt(KeyCode::Char('b')));
        press(&mut app, "x");
        assert_eq!(app.buffer.text(), "fix xthe bug", "⌥← and esc-b both jump a word back, and type nothing");
        app.handle_key(alt(KeyCode::Right));
        app.handle_key(alt(KeyCode::Char('f')));
        app.handle_key(alt(KeyCode::Backspace));
        assert_eq!(app.buffer.text(), "fix xthe ", "⌥→ and esc-f jump a word on, ⌥⌫ deletes the word before");
    }

    #[test]
    fn the_compose_box_edits_in_place_keeps_its_text_on_esc_and_takes_newlines() {
        let mut app = with_review();
        on_line(&mut app);
        press(&mut app, "cab");
        assert_eq!(app.focus, Focus::Side, "the box lives in the pane");
        assert_eq!(
            app.open.as_ref().unwrap().pane.as_ref().map(|p| &p.place),
            Some(&Place::Line { file: 0, new: Some(12), old: Some(12) })
        );
        app.handle_key(code(KeyCode::Left));
        press(&mut app, "x");
        assert_eq!(app.buffer.text(), "axb");
        app.handle_key(ctrl('a'));
        app.handle_key(code(KeyCode::Delete));
        assert_eq!(app.buffer.text(), "xb");
        app.handle_key(ctrl('e'));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
        press(&mut app, "y");
        assert_eq!(app.buffer.text(), "xb\ny", "alt-enter is a newline, not a send");
        app.handle_key(code(KeyCode::Esc));
        assert!(app.input.is_none());
        assert_eq!(app.focus, Focus::Review, "back where the box was opened from");
        press(&mut app, "j");
        press(&mut app, "c");
        assert_eq!(app.buffer.text(), "", "another line starts empty");
        assert_eq!(app.handle_key(code(KeyCode::Enter)), vec![], "an empty comment is dropped");
        press(&mut app, "k");
        press(&mut app, "c");
        assert_eq!(app.buffer.text(), "xb\ny", "the text left on line 12 comes back");
        let actions = app.handle_key(code(KeyCode::Enter));
        assert!(matches!(actions.as_slice(), [Action::SaveDraft { draft, .. }] if draft.body == "xb\ny"), "{actions:?}");
    }

    fn cmd_enter() -> KeyEvent {
        KeyEvent::new(KeyCode::Enter, KeyModifiers::SUPER)
    }

    #[test]
    fn cmd_enter_posts_a_new_thread_at_once_and_ctrl_s_posts_a_reply() {
        let mut app = with_review();
        on_line(&mut app);
        press(&mut app, "c");
        press(&mut app, " nit ");
        let actions = app.handle_key(cmd_enter());
        let [Action::Post { key, to: Post::Thread(position), body }] = actions.as_slice() else { panic!("{actions:?}") };
        assert_eq!((key, position.line.new, body.as_str()), (&mr_key(), Some(12), "nit"));
        assert!(app.input.is_none(), "the box closes while the forge answers");
        assert!(app.open.as_ref().unwrap().review.drafts.is_empty(), "no draft on the way");
        press(&mut app, "]N");
        app.handle_key(code(KeyCode::Enter));
        press(&mut app, "r");
        press(&mut app, "agreed");
        let actions = app.handle_key(ctrl('s'));
        assert_eq!(actions, vec![Action::Post { key: mr_key(), to: Post::Reply("c0ffee00c0ffee00".into()), body: "agreed".into() }]);
    }

    #[test]
    fn a_posted_comment_clears_its_text_and_refreshes_the_threads() {
        let mut app = with_review();
        on_line(&mut app);
        press(&mut app, "c");
        press(&mut app, "nit");
        let actions = app.handle_key(cmd_enter());
        let [Action::Post { to, .. }] = actions.as_slice() else { panic!("{actions:?}") };
        app.poll.discussions_due = None;
        app.apply(Incoming::Posted { key: mr_key(), to: to.clone() });
        assert_eq!(app.live_toast().map(|t| t.text.as_str()), Some("posted"));
        assert!(app.take_actions().contains(&Action::RefreshMr(mr_key())), "the new note is read back at once");
        press(&mut app, "c");
        assert_eq!(app.buffer.text(), "", "nothing left to send on the line");
    }

    #[test]
    fn a_refused_post_opens_the_box_again_with_its_text() {
        let mut app = with_review();
        on_line(&mut app);
        press(&mut app, "c");
        press(&mut app, "nit");
        let actions = app.handle_key(cmd_enter());
        let [Action::Post { key, to, .. }] = actions.as_slice() else { panic!("{actions:?}") };
        app.apply(Incoming::Failed { what: Failure::Post { key: key.clone(), to: to.clone() }, message: "HTTP 403".into() });
        assert!(app.live_toast().unwrap().danger);
        assert_eq!((app.input_label().as_str(), app.buffer.text()), ("new thread · charge.rs:12", "nit"));
    }

    #[test]
    fn cmd_enter_on_an_empty_box_does_nothing_and_saves_other_boxes_as_enter_does() {
        let mut app = with_review();
        on_line(&mut app);
        press(&mut app, "c");
        press(&mut app, "  ");
        assert_eq!(app.handle_key(cmd_enter()), vec![]);
        assert!(app.input.is_some(), "the box stays open");
        let mut app = with_saved_draft();
        press(&mut app, "le");
        let actions = app.handle_key(ctrl('s'));
        assert!(matches!(actions.as_slice(), [Action::UpdateDraft { .. }]), "an edited draft stays a draft: {actions:?}");
    }

    #[test]
    fn up_and_down_in_the_compose_box_move_between_its_lines() {
        let mut app = with_review();
        on_line(&mut app);
        press(&mut app, "cab");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
        press(&mut app, "cd");
        app.handle_key(code(KeyCode::Up));
        press(&mut app, "x");
        app.handle_key(code(KeyCode::Down));
        press(&mut app, "y");
        assert_eq!(app.buffer.text(), "abx\ncdy");
    }
}
