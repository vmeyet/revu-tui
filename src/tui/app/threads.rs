//! Threads in the diff: in zen and in a window too narrow for the pane beside it, the conversations of a line hang
//! under it. A thread is one row until `enter` unfolds it into a row per note, and the thread keys act on the row
//! under the cursor as they do in the pane. A comment written from the diff is written there too, in a box under
//! the row it will land after.
use super::{App, Focus, Input, Open};
use crate::review::{Anchor, Conversation, Entry, EntryKind, Place, Row, anchor_of};
use std::collections::BTreeSet;

/// What `enter` folds or unfolds: the thread of the row under the cursor, or every thread of its line.
enum Here {
    Thread(String),
    Line(BTreeSet<String>),
}

impl Open {
    /// On a thread row: the conversations of its line, their cursor stops in the diff, and the row's own.
    pub fn row_view(&self) -> Option<(Vec<Conversation>, Vec<Entry>, Option<Entry>)> {
        let row @ Row::Thread { stop, .. } = self.row()? else { return None };
        let conversations = self.review.conversations(&self.review.place_of(row)?);
        let entries = self.review.stops(&conversations);
        let focused = entries.get(*stop).copied();
        Some((conversations, entries, focused))
    }

    /// Whether the pane shows `place`: not a line whose threads read in the diff.
    pub(super) fn pane_takes(&self, place: &Place) -> bool {
        !(self.review.reads_threads_in_diff() && matches!(place, Place::Line { .. }))
    }

    /// What `enter` folds or unfolds with the cursor at `place`; nothing on the row of my new draft.
    fn threads_here(&self, place: &Place) -> Option<Here> {
        match self.row_view() {
            Some(view) => super::pane::stop_of(view)?.thread.map(Here::Thread),
            None => Some(Here::Line(self.review.conversations(place).into_iter().filter_map(|c| c.thread).collect())),
        }
    }

    /// The row showing the first note of thread `id` under the line at `place`.
    fn first_row_of(&self, place: &Place, id: &str) -> Option<usize> {
        let conversations = self.review.conversations(place);
        let starts = |entry: &Entry| entry.kind == EntryKind::Note(0) && conversations[entry.conversation].thread.as_deref() == Some(id);
        let first = self.review.stops(&conversations).iter().position(starts)?;
        let shows =
            |row: &Row| matches!(row, Row::Thread { stop, .. } if *stop == first) && self.review.place_of(row).as_ref() == Some(place);
        self.rows.iter().position(shows)
    }

    /// The row of the line `anchor` names, with the conversations hung under it and their cursor stops: the rows
    /// right after it, in order.
    fn hung_at(&self, anchor: &Anchor) -> Option<(usize, Vec<Conversation>, Vec<Entry>)> {
        let line = self.rows.iter().position(|row| self.review.row_holds(row, anchor))?;
        let conversations = self.review.conversations(&self.review.place_of(&self.rows[line])?);
        let entries = self.review.stops(&conversations);
        Some((line, conversations, entries))
    }

    /// The row showing my draft `index`, where threads read in the diff.
    fn draft_row(&self, index: usize) -> Option<usize> {
        let review = self.review.reads_threads_in_diff().then_some(&self.review)?;
        let draft = review.drafts.get(index)?;
        let anchor = draft.anchor.as_ref().or_else(|| review.thread(draft.reply_to.as_deref()?)?.anchor.as_ref())?;
        let (line, _, entries) = self.hung_at(anchor)?;
        let stop = entries.iter().position(|entry| entry.kind == EntryKind::Draft(index))?;
        Some(line + 1 + stop)
    }

    /// The row the text of `input` will land after, where threads read in the diff: the last row of the thread
    /// a reply answers, the last row under the line of a new thread, the row of an edited draft.
    pub(super) fn compose_row(&self, input: &Input) -> Option<usize> {
        let review = self.review.reads_threads_in_diff().then_some(&self.review)?;
        match input {
            Input::Comment { position } => {
                let (line, _, entries) = self.hung_at(&anchor_of(position)?)?;
                Some(line + entries.len())
            }
            Input::Reply { thread } => {
                let (line, conversations, entries) = self.hung_at(review.thread(thread)?.anchor.as_ref()?)?;
                let last = entries.iter().rposition(|entry| conversations[entry.conversation].thread.as_deref() == Some(thread))?;
                Some(line + 1 + last)
            }
            Input::EditDraft { draft } => self.draft_row(review.drafts.iter().position(|d| d.is(*draft))?),
            Input::Ask { .. } | Input::FollowUp => None,
        }
    }

    /// The thread `input` writes in, unfolded so the notes it answers show.
    fn with_thread_shown(self, input: &Input) -> Self {
        let thread = match input {
            Input::Reply { thread } => Some(thread.clone()),
            Input::EditDraft { draft } => self.review.drafts.iter().find(|d| d.is(*draft)).and_then(|d| d.reply_to.clone()),
            Input::Comment { .. } | Input::Ask { .. } | Input::FollowUp => None,
        };
        match thread.filter(|id| !self.review.unfolded.contains(id)) {
            Some(id) => self.relaid(|review| {
                let unfolded = review.unfolded.iter().cloned().chain([id]).collect();
                review.with_unfolded(unfolded)
            }),
            None => self,
        }
    }
}

impl App {
    /// The diff has the keys and its cursor is on a thread row.
    pub(super) fn on_thread_row(&self) -> bool {
        let row = self.open.as_ref().and_then(|open| open.row());
        self.focus == Focus::Review && matches!(row, Some(Row::Thread { .. }))
    }

    /// The row the compose box hangs under in the diff; none while the pane holds the box, or the diff has no
    /// row for what is written.
    pub fn compose_row(&self) -> Option<usize> {
        let from_the_diff = self.compose_from == Focus::Review;
        self.open.as_ref()?.compose_row(self.input.as_ref().filter(|_| from_the_diff)?)
    }

    /// A comment is being written and the diff does not hold its box.
    pub fn composes_outside_the_diff(&self) -> bool {
        self.input.is_some() && self.compose_row().is_none()
    }

    /// Written from the diff where threads read in it, `input` gets its box under the row it will land after:
    /// its thread unfolds and the cursor goes to that row, to come back once the box closes.
    pub(super) fn compose_in_diff(&mut self, input: &Input) -> bool {
        let Some(open) = self.open.take_if(|open| self.focus == Focus::Review && open.review.reads_threads_in_diff()) else {
            return false;
        };
        let left = open.row().cloned();
        let shown = open.with_thread_shown(input);
        let row = shown.compose_row(input);
        self.open = Some(match row {
            Some(index) => shown.move_to(index),
            None => shown,
        });
        self.compose_left = left.filter(|_| row.is_some());
        row.is_some()
    }

    /// The keys go back where the compose box was opened from, and the diff's cursor to the row it left for it.
    pub(super) fn back_from_compose(&mut self) {
        self.focus = self.compose_from;
        let Some(left) = self.compose_left.take() else { return };
        self.update_open(|open| match open.rows.iter().position(|row| super::review::same_place(row, &left)) {
            Some(index) => open.move_to(index),
            None => open,
        });
    }

    /// Where threads read in the diff, the cursor rests on the row of my draft `index`.
    pub(super) fn rest_on_draft(&mut self, index: usize) {
        self.update_open(|open| match open.draft_row(index) {
            Some(row) => open.move_to(row),
            None => open,
        });
    }

    /// Threads read under their lines in zen and in a window too narrow for the pane beside the diff, in the pane
    /// otherwise; a pane on a line closes once the diff shows its threads.
    pub(crate) fn fit_threads(&mut self) {
        let in_diff = self.zen || self.narrow;
        if let Some(open) = self.open.take_if(|o| o.review.threads_in_diff != in_diff) {
            self.open = Some(open.relaid(|review| review.with_threads_in_diff(in_diff)));
        }
        let on_a_line = |open: &Open| open.pane.as_ref().is_some_and(|pane| !open.pane_takes(&pane.place));
        if self.open.as_ref().is_some_and(on_a_line) {
            self.close_pane();
        }
    }

    /// `enter` where threads read in the diff: on a thread row that thread unfolds, or folds; on a line all its
    /// threads do, unfolding while one of them is folded.
    pub(super) fn toggle_threads_here(&mut self) -> bool {
        let Some(open) = &self.open else { return false };
        let here = open.row().and_then(|row| open.review.place_of(row)).filter(|place| !open.pane_takes(place));
        let Some(place) = here else { return false };
        let Some(here) = open.threads_here(&place) else { return true };
        let threads = match &here {
            Here::Thread(id) => BTreeSet::from([id.clone()]),
            Here::Line(ids) => ids.clone(),
        };
        let unfolded = &open.review.unfolded;
        let unfolded = if threads.is_subset(unfolded) { unfolded - &threads } else { unfolded | &threads };
        self.update_open(|open| {
            let next = open.relaid(|review| review.with_unfolded(unfolded));
            let Here::Thread(id) = &here else { return next };
            match next.first_row_of(&place, id) {
                Some(index) => next.move_to(index),
                None => next,
            }
        });
        true
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crate::tui::app::test_support::*;

    /// The removed line 13 of `charge.rs`, where the fixture's resolved thread hangs.
    const REMOVED_LINE_13: Row = Row::Line { file: 0, hunk: 0, index: 1 };
    const RESOLVED: &str = "│ ✓ resolved · you · Why drop the plain client?";

    fn row_of(app: &App) -> Row {
        app.open.as_ref().unwrap().row().cloned().unwrap()
    }

    fn stop_of(app: &App) -> Option<usize> {
        if let Row::Thread { stop, .. } = row_of(app) { Some(stop) } else { None }
    }

    fn enter(app: &mut App) -> Vec<Action> {
        app.handle_key(code(KeyCode::Enter))
    }

    fn thread_rows(app: &App) -> usize {
        app.open.as_ref().unwrap().rows.iter().filter(|row| matches!(row, Row::Thread { .. })).count()
    }

    /// The screen rows right under the one showing `code`, trimmed.
    fn under(screen: &str, code: &str, rows: usize) -> Vec<String> {
        let lines: Vec<&str> = screen.lines().collect();
        let at = lines.iter().position(|line| line.contains(code)).unwrap_or_else(|| panic!("{code} on screen:\n{screen}"));
        lines[at + 1..=at + rows].iter().map(|line| line.trim().to_owned()).collect()
    }

    #[test]
    fn in_zen_a_thread_is_one_row_under_its_line_and_enter_unfolds_it_in_place() {
        let mut app = with_review();
        press(&mut app, "zz]N");
        assert_eq!(row_of(&app), REMOVED_LINE_13);
        let folded = render(&mut app, 138, 40);
        assert_eq!(under(&folded, "-    let client = Client::new();", 1), [RESOLVED]);
        enter(&mut app);
        assert_eq!((listed_place(&app), app.focus, row_of(&app)), (None, Focus::Review, REMOVED_LINE_13), "no pane, the cursor stays");
        let unfolded = render(&mut app, 138, 40);
        let notes = under(&unfolded, "-    let client = Client::new();", 3);
        assert_eq!(notes, ["│ ✓ resolved", "│ you · 2h", "│ Why drop the plain client?"]);
        let column = |text: &str| unfolded.lines().find_map(|line| line.find(text).map(|byte| line[..byte].chars().count()));
        assert_eq!(column("Why drop"), column("    let client = Client::new();"), "notes start where the code does");
        enter(&mut app);
        assert!(render(&mut app, 138, 40).contains(RESOLVED), "enter again folds it");
    }

    #[test]
    fn each_thread_on_a_line_takes_its_row_and_counts_its_replies() {
        let mut app = zen_with_threads(vec![thread_on_line_13("aaa", 3), thread_on_line_13("bbb", 1)]);
        let screen = render(&mut app, 138, 40);
        let rows = under(&screen, "+    let client = Client::with_key", 2);
        assert_eq!(rows, ["│ ◆ unresolved · lea · note 0 of aaa · 2 replies", "│ ◆ unresolved · lea · note 0 of bbb"]);
    }

    #[test]
    fn a_folded_thread_marks_my_reply_waiting_in_it_and_my_new_draft_shows_whole() {
        let mut app = zen_with_threads(vec![thread_on_line_13("aaa", 1)]);
        let open = app.open.clone().unwrap();
        let line_13 = crate::review::Anchor { path: "src/pay/charge.rs".into(), side: crate::review::Side::New, line: 13 };
        let drafts = vec![crate::review::Draft::reply("aaa", "agreed"), crate::review::Draft::new(Some(line_13), "nit")];
        app.open = Some(open.relaid(|review| review.with_drafts(drafts)));
        let screen = render(&mut app, 138, 40);
        let rows = under(&screen, "+    let client = Client::with_key", 4);
        assert_eq!(rows, ["│ ◆ unresolved · lea · note 0 of aaa ◇", "│ ◇ draft", "│ you · unsaved ◇", "│ nit"]);
    }

    #[test]
    fn j_and_k_walk_one_row_of_a_folded_thread_and_every_note_of_an_unfolded_one() {
        let mut app = zen_with_threads(vec![thread_on_line_13("aaa", 3)]);
        press(&mut app, "j");
        assert_eq!(stop_of(&app), Some(0));
        press(&mut app, "j");
        assert_eq!(row_of(&app), Row::Line { file: 0, hunk: 0, index: 3 }, "one row, then the code again");
        press(&mut app, "kk");
        enter(&mut app);
        let walked: Vec<Option<usize>> = (0..4)
            .map(|_| {
                press(&mut app, "j");
                stop_of(&app)
            })
            .collect();
        assert_eq!(walked, [Some(0), Some(1), Some(2), None], "note by note, then past the thread");
        press(&mut app, "k");
        assert_eq!(stop_of(&app), Some(2), "and back into it from below");
    }

    #[test]
    fn enter_on_a_thread_row_folds_that_thread_alone_and_rests_on_its_row() {
        let mut app = zen_with_threads(vec![thread_on_line_13("aaa", 3), thread_on_line_13("bbb", 2)]);
        enter(&mut app);
        assert_eq!(thread_rows(&app), 1 + 3 + 2, "the fixture's folded thread, then both unfolded");
        press(&mut app, "jjjjj");
        assert_eq!(stop_of(&app), Some(4), "the second note of bbb");
        enter(&mut app);
        assert_eq!((thread_rows(&app), stop_of(&app)), (1 + 3 + 1, Some(3)), "bbb is one row again, the cursor on it");
        enter(&mut app);
        assert_eq!((thread_rows(&app), stop_of(&app)), (1 + 3 + 2, Some(3)), "and unfolds from its row");
        press(&mut app, "kkkk");
        assert_eq!(row_of(&app), ADDED_LINE_13);
        enter(&mut app);
        assert_eq!(thread_rows(&app), 1 + 1 + 1, "enter on the line folds them all once none is folded");
    }

    #[test]
    fn a_thread_longer_than_the_screen_scrolls_note_by_note_under_the_cursor() {
        let mut app = zen_with_threads(vec![thread_on_line_13("aaa", 20)]);
        enter(&mut app);
        for note in 0..20 {
            press(&mut app, "j");
            let screen = render(&mut app, 138, 16);
            let on_cursor = |line: &str| line.trim_start().starts_with('▎') && line.contains(&format!("note {note} of aaa"));
            assert!(screen.lines().any(on_cursor), "note {note} under the cursor:\n{screen}");
        }
        press(&mut app, "j");
        assert_eq!(row_of(&app), Row::Line { file: 0, hunk: 0, index: 3 });
        assert!(render(&mut app, 138, 16).contains("let attempt = Attempt::first();"));
    }

    #[test]
    fn the_thread_keys_act_on_the_thread_row_under_the_diffs_cursor() {
        let mut app = zen_with_threads(vec![thread_on_line_13("aaa", 3)]);
        enter(&mut app);
        press(&mut app, "jj");
        let url = "https://gitlab.com/acme/widgets/-/merge_requests/42#note_7000";
        assert_eq!(press(&mut app, "o"), vec![Action::OpenUrl(url.into())]);
        assert_eq!(press(&mut app, "y"), vec![Action::Copy { text: url.into(), done: format!("copied {url}") }]);
        press(&mut app, "+");
        assert_eq!(app.react.as_ref().map(|pick| (pick.thread.as_str(), pick.note)), Some(("aaa", 1)), "the note under the cursor");
        app.handle_key(code(KeyCode::Esc));
        assert_eq!(press(&mut app, "R"), vec![Action::Resolve { key: mr_key(), thread: "aaa".into(), resolved: true }]);
        press(&mut app, "r");
        assert_eq!((app.input_label().as_str(), listed_place(&app), app.focus), ("reply to lea", None, Focus::Review));
        let actions = type_text(&mut app, "agreed");
        let [Action::SaveDraft { draft, .. }] = actions.as_slice() else { panic!("{actions:?}") };
        assert_eq!(draft.reply_to.as_deref(), Some("aaa"));
        assert_eq!((stop_of(&app), app.draft_here()), (Some(3), Some(0)), "the cursor rests on my reply, the thread's last row");
        press(&mut app, "e");
        assert_eq!((app.input_label().as_str(), app.buffer.text()), ("edit draft", "agreed"));
        app.handle_key(code(KeyCode::Esc));
        press(&mut app, "d");
        assert_eq!((app.draft_count(), stop_of(&app)), (0, Some(2)), "the cursor rests on the note before the deleted draft");
    }

    #[test]
    fn off_a_thread_row_the_diff_keeps_its_own_keys() {
        let mut app = zen_with_threads(vec![thread_on_line_13("aaa", 1)]);
        assert_eq!(press(&mut app, "r"), vec![Action::RefreshMr(mr_key())], "r refreshes on a line");
        press(&mut app, "j");
        assert_eq!(press(&mut app, "r"), vec![], "and replies on a thread row");
        assert_eq!(app.input_label(), "reply to lea");
    }

    /// The screen row the compose box opens on, and the two rows around it: the one above its top, the one under its foot.
    fn around_the_box(screen: &str, label: &str) -> (String, String) {
        let lines: Vec<&str> = screen.lines().collect();
        let top = lines.iter().position(|line| line.contains(&format!("╭ {label}"))).unwrap_or_else(|| panic!("a box:\n{screen}"));
        let foot = lines.iter().position(|line| line.contains("╰ enter save")).unwrap_or_else(|| panic!("its foot:\n{screen}"));
        (lines[top - 1].trim().to_owned(), lines[foot + 1].trim().to_owned())
    }

    /// What the compose box is open on, once `keys` were pressed; the box closes again.
    fn written_with(app: &mut App, keys: &str) -> Option<Input> {
        press(app, keys);
        let input = app.input.clone();
        app.handle_key(code(KeyCode::Esc));
        input
    }

    fn reply_to(thread: &str) -> Input {
        Input::Reply { thread: thread.into() }
    }

    #[test]
    fn c_replies_on_a_thread_row_as_r_does_and_still_starts_a_thread_on_a_line() {
        let mut app = zen_with_threads(vec![thread_on_line_13("aaa", 2)]);
        assert!(matches!(written_with(&mut app, "c"), Some(Input::Comment { .. })), "a new thread on a code line");
        press(&mut app, "j");
        assert_eq!((written_with(&mut app, "c"), written_with(&mut app, "r")), (Some(reply_to("aaa")), Some(reply_to("aaa"))));
        assert_eq!(written_with(&mut app, "C"), None, "C only ever starts a thread, on the old side of a pair");
    }

    #[test]
    fn c_replies_in_the_pane_as_r_does_and_big_c_writes_nothing_there() {
        let mut app = with_review();
        press(&mut app, "]N");
        enter(&mut app);
        assert_eq!(app.focus, Focus::Side);
        let nina = "c0ffee00c0ffee00";
        assert_eq!((written_with(&mut app, "c"), written_with(&mut app, "r")), (Some(reply_to(nina)), Some(reply_to(nina))));
        assert_eq!(written_with(&mut app, "C"), None);
        let mut list = zen_with_threads(vec![]);
        press(&mut list, "T");
        assert_eq!(written_with(&mut list, "c"), Some(reply_to("6a9c1750b2d6e4f0")), "under zen's diff too");
    }

    #[test]
    fn a_key_bound_to_comment_replies_on_a_thread_and_comments_on_a_line() {
        let mut app = zen_with_threads(vec![thread_on_line_13("aaa", 2)]);
        app.keymap = keymap(r#"bind = { comment = "F" }"#);
        assert!(matches!(written_with(&mut app, "F"), Some(Input::Comment { .. })));
        press(&mut app, "j");
        assert_eq!(written_with(&mut app, "F"), Some(reply_to("aaa")));
        press(&mut app, "T");
        assert_eq!(written_with(&mut app, "F"), Some(reply_to("6a9c1750b2d6e4f0")), "in the pane as well");
    }

    #[test]
    fn c_on_a_thread_counts_as_a_reply_and_on_a_line_as_a_comment() {
        let mut app = zen_with_threads(vec![thread_on_line_13("aaa", 2)]);
        app.usage = Some(crate::usage::Tally::default());
        written_with(&mut app, "c");
        press(&mut app, "j");
        written_with(&mut app, "c");
        written_with(&mut app, "r");
        press(&mut app, "T");
        written_with(&mut app, "c");
        let counts = app.take_usage().unwrap();
        assert_eq!((counts.actions.get("comment"), counts.actions.get("reply"), counts.actions.get("refresh")), (Some(&1), Some(&3), None));
    }

    #[test]
    fn a_new_comment_is_written_under_its_line_and_its_draft_takes_the_cursor() {
        let mut app = with_review();
        press(&mut app, "zz");
        on_line(&mut app);
        press(&mut app, "cnit");
        assert_eq!((listed_place(&app), app.focus, app.input_label().as_str()), (None, Focus::Review, "new thread · charge.rs:12"));
        let screen = render(&mut app, 138, 40);
        let (above, below) = around_the_box(&screen, "new thread · charge.rs:12");
        assert!(above.contains("pub async fn charge(card") && below.contains("-    let client = Client::new();"), "{screen}");
        let column = |text: &str| screen.lines().find_map(|line| line.find(text).map(|byte| line[..byte].chars().count()));
        assert_eq!(column("nit"), column("pub async fn charge(card"), "the text is typed where the code starts");
        let actions = app.handle_key(code(KeyCode::Enter));
        assert!(matches!(actions.as_slice(), [Action::SaveDraft { .. }]), "{actions:?}");
        assert_eq!((stop_of(&app), app.draft_here()), (Some(0), Some(0)), "the cursor rests on the new draft");
        let screen = render(&mut app, 138, 40);
        let draft = under(&screen, "pub async fn charge(card", 3);
        assert_eq!(draft, ["▎           │ ◇ draft", "▎           │ you · unsaved ◇", "▎           │ nit"]);
    }

    #[test]
    fn a_new_comment_on_a_line_with_threads_is_written_after_them() {
        let mut app = zen_with_threads(vec![thread_on_line_13("aaa", 3), thread_on_line_13("bbb", 2)]);
        press(&mut app, "c");
        let (above, below) = around_the_box(&render(&mut app, 138, 40), "new thread · charge.rs:13");
        assert_eq!(above, "▎           │ ◆ unresolved · lea · note 0 of bbb · 1 reply", "under the line's last thread");
        assert!(below.contains("let attempt = Attempt::first();"), "{below}");
        app.handle_key(code(KeyCode::Esc));
        assert_eq!(row_of(&app), ADDED_LINE_13, "esc takes the cursor back to the line");
    }

    #[test]
    fn a_reply_unfolds_its_thread_and_is_written_under_its_last_note() {
        for (width, bar) in [(138, "▎           │"), (100, "▎      │")] {
            let mut app = zen_with_threads(vec![thread_on_line_13("aaa", 3), thread_on_line_13("bbb", 2)]);
            press(&mut app, "jr");
            assert_eq!((app.input_label().as_str(), thread_rows(&app)), ("reply to lea", 1 + 3 + 1), "aaa unfolds for the reply");
            let (above, below) = around_the_box(&render(&mut app, width, 40), "reply to lea");
            assert_eq!(above, format!("{bar} note 2 of aaa"), "{width} columns: under the last note");
            assert!(below.ends_with("◆ unresolved · lea · note 0 of bbb · 1 reply"), "{width} columns: before the next thread: {below}");
            app.handle_key(code(KeyCode::Esc));
            assert_eq!(stop_of(&app), Some(0), "esc takes the cursor back to the row r was pressed on");
            press(&mut app, "ragreed");
            app.handle_key(code(KeyCode::Enter));
            assert_eq!((stop_of(&app), app.draft_here()), (Some(3), Some(0)), "enter leaves it on the reply");
        }
    }

    #[test]
    fn an_edited_draft_is_written_under_its_own_row_in_a_narrow_window() {
        let mut app = with_saved_draft();
        render(&mut app, 100, 30);
        press(&mut app, "je");
        assert_eq!((app.input_label().as_str(), app.buffer.text(), listed_place(&app)), ("edit draft", "nit", None));
        let (above, below) = around_the_box(&render(&mut app, 100, 30), "edit draft");
        assert!(above.contains("▎      │ nit") && below.contains("-    let client = Client::new();"), "{above}\n{below}");
        press(&mut app, "!");
        app.handle_key(code(KeyCode::Enter));
        assert_eq!((stop_of(&app), app.open.as_ref().unwrap().review.drafts[0].body.as_str()), (Some(0), "nit!"));
    }

    #[test]
    fn the_diff_scrolls_so_a_growing_box_stays_whole_on_screen() {
        let mut app = zen_with_threads(vec![thread_on_line_13("aaa", 12)]);
        enter(&mut app);
        press(&mut app, "jr");
        for line in ["one", "two", "three", "four"] {
            press(&mut app, line);
            let screen = render(&mut app, 138, 18);
            let shows = |text: &str| screen.contains(text);
            assert!(shows("╭ reply to lea") && shows(line) && shows("╰ enter save") && shows("note 11 of aaa"), "{line}:\n{screen}");
            app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
        }
    }

    #[test]
    fn a_draft_whose_line_is_folded_away_is_edited_in_a_box_at_the_foot_of_the_diff() {
        let mut app = with_saved_draft();
        press(&mut app, "zz");
        app.review_jump_to(|row| matches!(row, Row::Hunk { index: 0, .. }));
        press(&mut app, "zaPe");
        assert_eq!((app.input_label().as_str(), app.compose_row(), listed_place(&app)), ("edit draft", None, None));
        let screen = render(&mut app, 138, 40);
        let lines: Vec<&str> = screen.lines().collect();
        let top = lines.iter().position(|line| line.contains("╭ edit draft")).unwrap_or_else(|| panic!("a box:\n{screen}"));
        assert!(lines[..top].iter().any(|line| line.contains("Cargo.lock")), "under every row of the diff:\n{screen}");
    }

    #[test]
    fn a_drag_under_the_box_still_copies_the_line_it_is_on() {
        let mut app = zen_with_threads(vec![thread_on_line_13("aaa", 1)]);
        press(&mut app, "c");
        let (x, y) = spot(&mut app, "let attempt = Attempt::first();", 138, 40);
        let actions = drag(&mut app, (x, y), (x + 10, y), 138, 40);
        assert_eq!(actions, vec![Action::Copy { text: "let attempt".into(), done: "copied 1 line".into() }]);
    }

    #[test]
    fn written_from_the_pane_or_side_by_side_the_box_stays_in_the_pane() {
        let mut list = zen_with_threads(vec![thread_on_line_13("aaa", 2)]);
        press(&mut list, "TJr");
        assert_eq!((list.input_label().as_str(), list.focus, list.compose_row()), ("reply to lea", Focus::Side, None));
        let screen = render(&mut list, 138, 40);
        let rule = screen.lines().position(|line| line.contains("─ the whole MR")).unwrap();
        let top = screen.lines().position(|line| line.contains("╭ reply to lea")).unwrap();
        assert!(top > rule, "the box sits in the list under the diff:\n{screen}");
        let mut beside = with_review();
        press(&mut beside, "zzD");
        render(&mut beside, 200, 40);
        beside.review_jump_to(|row| matches!(row, Row::Pair { .. }));
        press(&mut beside, "c");
        assert_eq!((beside.compose_row(), beside.focus), (None, Focus::Side));
        assert!(matches!(listed_place(&beside), Some(Place::Line { .. })), "side by side writes in the pane of the line");
    }

    #[test]
    fn in_a_narrow_window_threads_read_in_the_diff_and_a_wide_one_gives_them_back_to_the_pane() {
        let mut app = with_review();
        press(&mut app, "]N");
        assert!(render(&mut app, 100, 30).contains(RESOLVED));
        enter(&mut app);
        assert_eq!((listed_place(&app), app.focus), (None, Focus::Review), "no page takes the diff's place");
        assert!(render(&mut app, 100, 30).contains("│ Why drop the plain client?"));
        let wide = render(&mut app, 160, 30);
        assert!(thread_rows(&app) == 0 && !wide.contains("Why drop"), "the diff shows code alone:\n{wide}");
        enter(&mut app);
        assert_eq!((listed_place(&app), app.focus), (Some(Place::Line { file: 0, new: None, old: Some(13) }), Focus::Side));
        render(&mut app, 100, 30);
        assert_eq!((listed_place(&app), app.focus, thread_rows(&app)), (None, Focus::Review, 1), "narrow again: the pane closes");
    }

    #[test]
    fn a_set_narrow_width_moves_where_threads_read_in_the_diff() {
        let mut app = with_review();
        app.widths = crate::config::Widths { narrow: 100, wide: 150 };
        render(&mut app, 110, 30);
        assert_eq!(thread_rows(&app), 0, "110 columns are no longer narrow");
        render(&mut app, 99, 30);
        assert_eq!(thread_rows(&app), 1);
    }

    #[test]
    fn thread_rows_start_where_the_code_does_under_both_gutters_and_under_one() {
        let mut app = with_review();
        press(&mut app, "zz]N");
        enter(&mut app);
        for (width, numbers) in [(138, "13      -"), (100, "13 -")] {
            let screen = render(&mut app, width, 40);
            let column = |text: &str| screen.lines().find_map(|line| line.find(text).map(|byte| line[..byte].chars().count()));
            assert!(screen.contains(&format!("{numbers}    let client = Client::new();")), "{width} columns:\n{screen}");
            assert_eq!(column("Why drop"), column("    let client = Client::new();"), "{width} columns: the note under the code");
            assert_eq!(column("│ Why drop"), column(" -    let client = Client::new();"), "{width} columns: the rule before the sign");
        }
    }

    #[test]
    fn side_by_side_keeps_its_threads_in_the_pane_in_zen_too() {
        let mut app = with_review();
        press(&mut app, "zzD]N");
        let screen = render(&mut app, 200, 30);
        assert!(app.open.as_ref().unwrap().review.shows_side_by_side() && thread_rows(&app) == 0, "{screen}");
        enter(&mut app);
        assert_eq!(app.focus, Focus::Side);
        assert!(matches!(listed_place(&app), Some(Place::Line { .. })));
        assert!(render(&mut app, 200, 40).contains("─ charge.rs:13 · 1 thread"), "under the diff, as before");
    }

    #[test]
    fn jumps_stop_on_marked_lines_never_on_thread_rows_and_a_folded_hunk_hides_them() {
        let mut app = zen_with_threads(vec![thread_on_line_13("aaa", 2)]);
        enter(&mut app);
        press(&mut app, "g");
        let stops: Vec<Row> = (0..3)
            .map(|_| {
                press(&mut app, "]N");
                row_of(&app)
            })
            .collect();
        assert_eq!(stops, [REMOVED_LINE_13, ADDED_LINE_13, Row::Header]);
        app.review_jump_to(|row| matches!(row, Row::Thread { stop: 1, .. }));
        press(&mut app, "]c");
        assert!(matches!(row_of(&app), Row::Hunk { index: 1, .. }), "]c leaves the thread for the next hunk");
        app.review_jump_to(|row| matches!(row, Row::Thread { stop: 1, .. }));
        press(&mut app, "za");
        assert_eq!((row_of(&app), thread_rows(&app)), (Row::Hunk { file: 0, index: 0, open: false }, 0));
        press(&mut app, "za/attempt");
        app.handle_key(code(KeyCode::Enter));
        assert_eq!(row_of(&app), Row::Line { file: 0, hunk: 0, index: 3 }, "search lands on code past the thread rows");
    }

    #[test]
    fn a_refresh_keeps_what_was_unfolded_and_every_thread_still_lists_under_the_diff() {
        let mut app = zen_with_threads(vec![thread_on_line_13("aaa", 2)]);
        enter(&mut app);
        let fresh = Review::new(mr(), &diffs(), discussions().into_iter().chain([thread_on_line_13("aaa", 3)]).collect(), &[]);
        app.apply(Incoming::Review { key: mr_key(), review: Box::new(fresh), cached: None });
        assert_eq!(thread_rows(&app), 1 + 3, "the new note joins the unfolded thread");
        press(&mut app, "T");
        assert_eq!((listed_place(&app), app.focus), (Some(Place::All), Focus::Side));
        let screen = render(&mut app, 138, 40);
        assert!(screen.contains("─ the whole MR · 4 threads") && screen.contains("│ note 2 of aaa"), "{screen}");
        press(&mut app, "m");
        assert!(render(&mut app, 138, 40).contains("the whole MR · mine · 1 of 4 threads"));
    }

    #[test]
    fn the_wheel_walks_thread_rows_and_a_drag_across_one_copies_code_alone() {
        let mut app = with_review();
        press(&mut app, "zz]N");
        let from = spot(&mut app, "let client = Client::new()", 138, 40);
        let to = spot(&mut app, "let client = Client::with_key", 138, 40);
        assert_eq!(to.1, from.1 + 2, "the thread's row sits between the two lines");
        wheel(&mut app, true, from.0, from.1);
        assert_eq!(row_of(&app), Row::Line { file: 0, hunk: 0, index: 3 }, "a notch is three rows, the thread's one of them");
        let actions = drag(&mut app, from, (to.0 + 3, to.1), 138, 40);
        let Some(Action::Copy { text, .. }) = actions.first() else { panic!("a copy: {actions:?}") };
        assert_eq!(text.lines().count(), 2, "{text}");
        assert!(text.starts_with("let client = Client::new();") && !text.contains("Why drop"), "{text}");
    }

    #[test]
    fn snapshot_threads_in_the_diff() {
        let mut app = zen_with_threads(vec![thread_on_line_13("aaa", 3), thread_on_line_13("bbb", 2)]);
        press(&mut app, "j");
        enter(&mut app);
        press(&mut app, "jr");
        press(&mut app, "agreed");
        insta::assert_snapshot!("zen_threads_in_diff", render(&mut app, 138, 34));
        app.handle_key(code(KeyCode::Esc));
        press(&mut app, "zz");
        insta::assert_snapshot!("narrow_threads_in_diff", render(&mut app, 100, 30));
        press(&mut app, "c");
        press(&mut app, "one more thing");
        insta::assert_snapshot!("narrow_compose_in_diff", render(&mut app, 100, 30));
    }
}
