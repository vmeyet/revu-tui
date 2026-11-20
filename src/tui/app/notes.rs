//! My published comments: `e` on one edits it in the compose box, `d` deletes it from the forge after a `y`.
//! Either changes the screen only once the forge answered.
use super::input::target_key;
use super::pane::Stop;
use super::{Action, App, Confirm, EntryKind, Input, MrKey, Open};
use crate::forge::{Me, Note, NoteKind, ThreadId};
use crate::review::Thread;

impl App {
    /// `d`: my draft under the cursor goes at once; my published comment waits for a `y`.
    pub(super) fn delete_here(&mut self) -> Vec<Action> {
        if let Some(index) = self.draft_here() {
            return self.delete_draft(index);
        }
        let Some(open) = &self.open else { return vec![] };
        let Some(stop) = self.stop_here() else { return vec![] };
        match Mine::at(open, &stop, self.me.on(&open.key)).and_then(Mine::deletable) {
            Ok(mine) => {
                let (key, thread, note) = (open.key.clone(), mine.thread.id.clone(), Box::new(mine.note().clone()));
                self.confirm = Some(Confirm::DeleteNote { key, thread, note });
            }
            Err(reason) => self.toast(reason),
        }
        vec![]
    }

    /// `e`: my draft under the cursor, or my published comment, opens in the compose box with its text.
    pub(super) fn edit_here(&mut self) {
        if self.edit_draft_here() {
            return;
        }
        let mine = self.open.as_ref().zip(self.stop_here()).map(|(open, stop)| Mine::at(open, &stop, self.me.on(&open.key)));
        match mine {
            Some(Ok(mine)) => {
                let input = Input::EditNote { thread: mine.thread.id.clone(), note: Box::new(mine.note().clone()) };
                let body = if self.is_unsent(&input) { String::new() } else { mine.note().body.clone() };
                self.open_input(input, &body);
            }
            Some(Err(reason)) => self.toast(reason),
            None => self.toast("e edits your draft or your comment"),
        }
    }

    /// A finished edit goes to the forge at once, its text waiting under the note until the forge takes it; the
    /// text the note already has sends nothing.
    pub(super) fn change_note(&mut self, thread: ThreadId, note: Box<Note>, text: String) -> Vec<Action> {
        let Some(key) = self.open.as_ref().map(|o| o.key.clone()) else { return vec![] };
        if text.trim() == note.body.trim() {
            self.toast("nothing changed");
            return vec![];
        }
        self.unsent.insert(target_key(&Input::EditNote { thread: thread.clone(), note: note.clone() }), text.clone());
        vec![Action::EditNote { key, thread, note, body: text }]
    }

    /// The forge took the edit: the note reads as the forge holds it, and the MR is read again.
    pub(super) fn note_edited(&mut self, key: &MrKey, thread: ThreadId, note: Box<Note>, body: &str) -> Vec<Action> {
        self.update_open_of(key, |open| open.relaid(|review| review.with_note_body(&thread, note.id, body)));
        self.unsent.remove(&target_key(&Input::EditNote { thread, note }));
        self.toast("comment edited");
        self.reread(key)
    }

    /// The forge deleted it: the note leaves the screen, and the MR is read again for what else changed.
    pub(super) fn note_deleted(&mut self, key: &MrKey, thread: &ThreadId, note: u64) -> Vec<Action> {
        self.update_open_of(key, |open| open.relaid(|review| review.without_note(thread, note)));
        self.close_pane_left_empty();
        self.toast("comment deleted");
        self.reread(key)
    }

    /// The pane of a place whose last conversation went closes, unless something is being written in it.
    fn close_pane_left_empty(&mut self) {
        let lists_nothing = |open: &Open| !open.lists_every_thread() && open.pane_view().is_some_and(|view| view.entries.is_empty());
        if self.input.is_none() && self.open.as_ref().is_some_and(lists_nothing) {
            self.close_pane();
        }
    }
}

/// A published note I wrote: note `index` of `thread`.
struct Mine<'a> {
    thread: &'a Thread,
    index: usize,
}

impl<'a> Mine<'a> {
    /// The note at `stop` when I wrote it, else why `e` and `d` leave it alone.
    fn at(open: &'a Open, stop: &Stop, me: &Me) -> Result<Self, &'static str> {
        const NO_NOTE: &str = "no comment here";
        let EntryKind::Note(index) = stop.kind else { return Err(NO_NOTE) };
        let thread = stop.thread.as_ref().and_then(|id| open.review.thread(id)).ok_or(NO_NOTE)?;
        let note = thread.notes.get(index).ok_or(NO_NOTE)?;
        if !note.written_by(me) {
            return Err("not your comment: e and d change only your own");
        }
        Ok(Self { thread, index })
    }

    fn note(&self) -> &'a Note {
        &self.thread.notes[self.index]
    }

    /// Itself, or why `d` leaves it alone. The first note of a thread stays while it has replies: no forge says
    /// what becomes of them, and they would answer nothing.
    fn deletable(self) -> Result<Self, &'static str> {
        if self.note().kind == Some(NoteKind::Summary) {
            return Err("the forge keeps the summary of a review: e edits it");
        }
        if self.index == 0 && self.thread.notes.len() > 1 {
            return Err("this comment has replies: it stays while they answer it");
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use crate::forge::{Identities, Note, NoteKind, User};
    use crate::tui::app::Confirm;
    use crate::tui::app::test_support::*;

    const LINE_13: Place = Place::Line { file: 0, new: Some(13), old: None };

    /// `thread` with its notes `mine` written by nina, who reads.
    fn with_mine(thread: Discussion, mine: &[usize]) -> Discussion {
        let by_nina = |note: Note| Note { author: User { username: "nina".into(), ..note.author }, ..note };
        let notes = thread.notes.into_iter().enumerate().map(|(i, note)| if mine.contains(&i) { by_nina(note) } else { note });
        Discussion { notes: notes.collect(), ..thread }
    }

    /// The pane on line 13 of `charge.rs`, `thread` the only one of the MR, the cursor on its note `note`.
    fn on_note(thread: Discussion, note: usize) -> App {
        let mut app = with_review();
        app.apply(Incoming::Discussions { key: mr_key(), discussions: vec![thread] });
        app.open_pane(LINE_13);
        press(&mut app, &"j".repeat(note));
        app
    }

    fn on_my_only_note() -> App {
        on_note(with_mine(thread_on_line_13("aaa", 1), &[0]), 0)
    }

    /// The same note on an MR of github.com, where I am `nina-gh`, written by `author`.
    fn on_a_note_on_github_by(author: &str) -> App {
        let key = MrKey { host: Some("github.com".into()), ..mr_key() };
        let mut app = App::new(settings());
        app.me = Identities::from("nina").with_host("github.com", "nina-gh".into());
        app.start_on(key.clone());
        app.apply(Incoming::Review { key: key.clone(), review: Box::new(review()), cached: None });
        let thread = thread_on_line_13("aaa", 1);
        let notes = thread.notes.into_iter().map(|note| Note { author: User { username: author.into(), ..note.author }, ..note });
        app.apply(Incoming::Discussions { key, discussions: vec![Discussion { notes: notes.collect(), ..thread }] });
        app.open_pane(LINE_13);
        app
    }

    #[test]
    fn my_comment_is_mine_once_the_forge_corrected_a_stale_name() {
        let mut app = on_my_only_note();
        app.me = "nina-then".into();
        press(&mut app, "e");
        assert_eq!(toast(&app), "not your comment: e and d change only your own");
        app.apply(queue_answer(None, sections(), false));
        press(&mut app, "e");
        assert_eq!(app.input_label(), "edit comment");
    }

    #[test]
    fn a_name_in_another_case_is_still_mine() {
        let mut app = on_my_only_note();
        app.me = "Nina".into();
        press(&mut app, "e");
        assert_eq!(app.input_label(), "edit comment");
    }

    #[test]
    fn e_and_d_on_another_host_take_my_name_there() {
        let mut app = on_a_note_on_github_by("nina-gh");
        press(&mut app, "d");
        assert!(matches!(app.confirm, Some(Confirm::DeleteNote { .. })));
        let mut app = on_a_note_on_github_by("nina");
        press(&mut app, "e");
        assert_eq!(toast(&app), "not your comment: e and d change only your own");
    }

    #[test]
    fn a_comment_on_another_host_reads_you_under_my_name_there() {
        let screen = render(&mut on_a_note_on_github_by("nina-gh"), 120, 20);
        assert!(screen.contains("you ·") && !screen.contains("nina-gh"), "{screen}");
    }

    /// The one edit `actions` send, as the forge answers once it holds the new text.
    fn edited(actions: &[Action]) -> Incoming {
        let [Action::EditNote { key, thread, note, body }] = actions else { panic!("{actions:?}") };
        Incoming::NoteEdited { key: key.clone(), thread: thread.clone(), note: note.clone(), body: body.clone() }
    }

    fn body_of_the_note(app: &App) -> String {
        app.open.as_ref().unwrap().review.thread(&"aaa".into()).unwrap().notes[0].body.clone()
    }

    #[test]
    fn e_on_my_published_comment_opens_the_box_with_its_text_and_enter_publishes_the_edit() {
        let mut app = on_my_only_note();
        assert_eq!(press(&mut app, "e"), vec![]);
        assert_eq!((app.input_label().as_str(), app.buffer.text()), ("edit comment", "note 0 of aaa"));
        press(&mut app, ", fixed");
        insta::assert_snapshot!("edit_comment_box", render(&mut app, 120, 14));
        let actions = app.handle_key(code(KeyCode::Enter));
        let [Action::EditNote { thread, note, body, .. }] = actions.as_slice() else { panic!("{actions:?}") };
        assert_eq!((thread.as_str(), note.id, body.as_str()), ("aaa", 7000, "note 0 of aaa, fixed"));
        assert_eq!((body_of_the_note(&app).as_str(), app.draft_count()), ("note 0 of aaa", 0), "no draft, and no guess on screen");
    }

    #[test]
    fn the_box_of_a_published_comment_says_enter_publishes_and_a_reply_box_that_it_saves() {
        let mut app = on_my_only_note();
        press(&mut app, "e");
        assert_eq!(app.input_legend().keys, "enter publish · esc cancel");
        app.handle_key(code(KeyCode::Esc));
        press(&mut app, "r");
        assert_eq!(app.input_legend().keys, "enter save · ⌘enter post · ⌥enter newline");
    }

    #[test]
    fn an_edit_left_with_esc_or_sent_unchanged_asks_nothing_of_the_forge() {
        let mut app = on_my_only_note();
        press(&mut app, "ex");
        assert_eq!(app.handle_key(code(KeyCode::Esc)), vec![]);
        let mut app = on_my_only_note();
        press(&mut app, "e");
        assert_eq!(app.handle_key(code(KeyCode::Enter)), vec![]);
        assert_eq!(toast(&app), "nothing changed");
    }

    #[test]
    fn e_on_a_comment_of_someone_else_opens_no_box_and_says_why() {
        let mut app = on_note(thread_on_line_13("aaa", 1), 0);
        press(&mut app, "e");
        assert_eq!((app.input.as_ref(), toast(&app)), (None, "not your comment: e and d change only your own"));
    }

    #[test]
    fn an_edit_the_forge_took_reads_as_the_forge_holds_it_and_the_mr_is_read_again() {
        let mut app = on_my_only_note();
        press(&mut app, "e");
        let sent = type_text(&mut app, "!");
        app.apply(edited(&sent));
        assert_eq!((body_of_the_note(&app).as_str(), toast(&app)), ("note 0 of aaa!", "comment edited"));
        assert!(app.take_actions().contains(&Action::RefreshMr(mr_key())));
        press(&mut app, "e");
        assert_eq!(app.buffer.text(), "note 0 of aaa!", "nothing waits unsent");
    }

    #[test]
    fn an_edit_the_forge_refused_keeps_the_old_text_on_screen_and_the_new_one_in_the_box() {
        let mut app = on_my_only_note();
        press(&mut app, "e");
        let sent = type_text(&mut app, "!");
        let [Action::EditNote { key, thread, note, .. }] = sent.as_slice() else { panic!("{sent:?}") };
        let what = Failure::EditNote { key: key.clone(), thread: thread.clone(), note: note.clone() };
        app.apply(Incoming::Failed { what, message: "offline".into() });
        assert_eq!(toast(&app), "comment not changed: offline · your text waits in the box");
        assert_eq!((body_of_the_note(&app).as_str(), app.buffer.text()), ("note 0 of aaa", "note 0 of aaa!"));
        assert!(matches!(app.handle_key(code(KeyCode::Enter)).as_slice(), [Action::EditNote { body, .. }] if body == "note 0 of aaa!"));
    }

    #[test]
    fn in_zen_the_edit_box_opens_in_the_diff_under_the_thread() {
        let mut app = zen_with_threads(vec![with_mine(thread_on_line_13("aaa", 1), &[0])]);
        press(&mut app, "je");
        assert_eq!((app.buffer.text(), app.focus, listed_place(&app)), ("note 0 of aaa", Focus::Review, None));
        assert!(app.compose_row().is_some(), "the diff holds the box");
    }

    #[test]
    fn an_edit_finished_in_the_editor_is_published_as_one_typed_in_the_box() {
        let mut app = on_my_only_note();
        press(&mut app, "e");
        let handed = app.handle_key(ctrl('o'));
        let [Action::Compose { input, .. }] = handed.as_slice() else { panic!("{handed:?}") };
        app.apply(Incoming::Composed { input: input.clone(), edited: Edited::Done("rewritten".into()) });
        assert!(matches!(app.take_actions().as_slice(), [Action::EditNote { body, .. }] if body == "rewritten"));
    }

    #[test]
    fn d_on_my_published_comment_asks_first_quoting_it_and_deletes_only_on_y() {
        let mut app = on_my_only_note();
        assert_eq!(press(&mut app, "d"), vec![]);
        insta::assert_snapshot!("delete_comment_confirm", render(&mut app, 120, 12));
        assert_eq!(press(&mut app, "n"), vec![], "any other key cancels");
        assert_eq!(toast(&app), "not deleted");
        press(&mut app, "d");
        let actions = press(&mut app, "y");
        assert!(
            matches!(actions.as_slice(), [Action::DeleteNote { key, thread, note }] if (key, thread.as_str(), note.id) == (&mr_key(), "aaa", 7000)),
            "{actions:?}"
        );
    }

    #[test]
    fn d_on_a_comment_of_someone_else_asks_nothing_and_says_why() {
        let mut app = on_note(thread_on_line_13("aaa", 1), 0);
        assert_eq!(press(&mut app, "d"), vec![]);
        assert_eq!((app.confirm.as_ref(), toast(&app)), (None, "not your comment: e and d change only your own"));
    }

    #[test]
    fn d_on_my_first_comment_keeps_it_while_it_has_replies() {
        let mut app = on_note(with_mine(thread_on_line_13("aaa", 2), &[0]), 0);
        press(&mut app, "d");
        assert_eq!((app.confirm.as_ref(), toast(&app)), (None, "this comment has replies: it stays while they answer it"));
    }

    #[test]
    fn d_on_my_reply_asks_to_delete_that_reply_alone() {
        let mut app = on_note(with_mine(thread_on_line_13("aaa", 3), &[0, 1]), 1);
        press(&mut app, "d");
        assert!(matches!(&app.confirm, Some(Confirm::DeleteNote { note, .. }) if note.id == 7001), "{:?}", app.confirm);
    }

    #[test]
    fn d_on_the_summary_of_my_review_says_the_forge_keeps_it() {
        let mut thread = with_mine(thread_on_line_13("aaa", 1), &[0]);
        thread.notes[0].kind = Some(NoteKind::Summary);
        let mut app = on_note(thread, 0);
        press(&mut app, "d");
        assert_eq!((app.confirm.as_ref(), toast(&app)), (None, "the forge keeps the summary of a review: e edits it"));
    }

    #[test]
    fn d_on_a_thread_row_of_the_diff_asks_as_the_pane_does() {
        let mut app = zen_with_threads(vec![with_mine(thread_on_line_13("aaa", 1), &[0])]);
        press(&mut app, "jd");
        assert!(matches!(&app.confirm, Some(Confirm::DeleteNote { note, .. }) if note.id == 7000), "{:?}", app.confirm);
    }

    #[test]
    fn a_deleted_comment_leaves_the_screen_the_pane_closes_once_empty_and_the_mr_is_read_again() {
        let mut app = on_my_only_note();
        app.apply(Incoming::NoteDeleted { key: mr_key(), thread: "aaa".into(), note: 7000 });
        assert_eq!(toast(&app), "comment deleted");
        assert_eq!((listed_place(&app), app.focus), (None, Focus::Review), "nothing left to list on the line");
        assert!(app.take_actions().contains(&Action::RefreshMr(mr_key())), "the forge then says what else changed");
    }

    #[test]
    fn a_deleted_reply_leaves_the_rest_of_its_thread_and_the_cursor_where_it_was() {
        let mut app = on_note(with_mine(thread_on_line_13("aaa", 3), &[1]), 1);
        app.apply(Incoming::NoteDeleted { key: mr_key(), thread: "aaa".into(), note: 7001 });
        let left: Vec<u64> = app.open.as_ref().unwrap().review.thread(&"aaa".into()).unwrap().notes.iter().map(|n| n.id).collect();
        assert_eq!((left, listed_place(&app), app.focus), (vec![7000, 7002], Some(LINE_13), Focus::Side));
        assert_eq!(app.stop_here().map(|stop| stop.kind), Some(EntryKind::Note(1)), "on the note that followed it");
    }

    #[test]
    fn a_delete_the_forge_refused_keeps_the_comment_and_says_why() {
        let mut app = on_my_only_note();
        app.apply(Incoming::Failed { what: Failure::DeleteNote, message: "HTTP 500".into() });
        let warning = app.live_toast().unwrap();
        assert_eq!((warning.text.as_str(), warning.danger), ("comment not deleted: HTTP 500", true));
        assert!(app.open.as_ref().unwrap().review.thread(&"aaa".into()).is_some());
    }
}
