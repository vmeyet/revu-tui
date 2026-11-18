//! My published comments: `d` on one deletes it from the forge, after a `y`.
use super::pane::Stop;
use super::{Action, App, Confirm, EntryKind, MrKey, Open};
use crate::forge::{Note, NoteKind, ThreadId};
use crate::review::Thread;

impl App {
    /// `d`: my draft under the cursor goes at once; my published comment waits for a `y`.
    pub(super) fn delete_here(&mut self) -> Vec<Action> {
        if let Some(index) = self.draft_here() {
            return self.delete_draft(index);
        }
        let Some(open) = &self.open else { return vec![] };
        let Some(stop) = self.stop_here() else { return vec![] };
        match note_to_delete(open, &stop, &self.me) {
            Ok((thread, note)) => {
                let (key, thread, note) = (open.key.clone(), thread.id.clone(), Box::new(note.clone()));
                self.confirm = Some(Confirm::DeleteNote { key, thread, note });
            }
            Err(reason) => self.toast(reason),
        }
        vec![]
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

/// My published note at `stop`, or why `d` leaves it alone. The first note of a thread stays while it has
/// replies: no forge says what becomes of them, and they would answer nothing.
fn note_to_delete<'a>(open: &'a Open, stop: &Stop, me: &str) -> Result<(&'a Thread, &'a Note), &'static str> {
    const NO_NOTE: &str = "no comment here";
    let EntryKind::Note(index) = stop.kind else { return Err(NO_NOTE) };
    let thread = stop.thread.as_ref().and_then(|id| open.review.thread(id)).ok_or(NO_NOTE)?;
    let note = thread.notes.get(index).ok_or(NO_NOTE)?;
    if !note.written_by(me) {
        return Err("not your comment: d deletes only your own");
    }
    if note.kind == Some(NoteKind::Summary) {
        return Err("the forge keeps the summary of a review: it cannot be deleted");
    }
    if index == 0 && thread.notes.len() > 1 {
        return Err("this comment has replies: it stays while they answer it");
    }
    Ok((thread, note))
}

#[cfg(test)]
mod tests {
    use crate::forge::{Note, NoteKind, User};
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

    fn toast(app: &App) -> &str {
        &app.live_toast().unwrap().text
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
        assert_eq!((app.confirm.as_ref(), toast(&app)), (None, "not your comment: d deletes only your own"));
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
        assert_eq!((app.confirm.as_ref(), toast(&app)), (None, "the forge keeps the summary of a review: it cannot be deleted"));
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
