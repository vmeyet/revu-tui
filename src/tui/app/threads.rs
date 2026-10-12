//! Threads in the diff: in zen and in a window too narrow for the pane beside it, the conversations of a line hang
//! under it. A thread is one row until `enter` unfolds it into a row per note, and the thread keys act on the row
//! under the cursor as they do in the pane.
use super::{App, Open};
use crate::review::{Conversation, Entry, EntryKind, Place, Row};
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
}

impl App {
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
        assert_eq!(press(&mut app, "o"), vec![Action::OpenUrl("https://gitlab.com/acme/widgets/-/merge_requests/42#note_7000".into())]);
        press(&mut app, "+");
        assert_eq!(app.react.as_ref().map(|pick| (pick.thread.as_str(), pick.note)), Some(("aaa", 1)), "the note under the cursor");
        app.handle_key(code(KeyCode::Esc));
        assert_eq!(press(&mut app, "R"), vec![Action::Resolve { key: mr_key(), thread: "aaa".into(), resolved: true }]);
        press(&mut app, "r");
        assert_eq!((app.input_label().as_str(), listed_place(&app), app.focus), ("reply to lea", None, Focus::Review));
        let actions = type_text(&mut app, "agreed");
        let [Action::SaveDraft { draft, .. }] = actions.as_slice() else { panic!("{actions:?}") };
        assert_eq!(draft.reply_to.as_deref(), Some("aaa"));
        press(&mut app, "jj");
        assert_eq!((stop_of(&app), app.draft_here()), (Some(3), Some(0)), "my reply closes the thread");
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

    #[test]
    fn a_comment_is_written_in_a_box_under_the_diff_and_its_draft_hangs_under_the_line() {
        let mut app = with_review();
        press(&mut app, "zz");
        on_line(&mut app);
        press(&mut app, "cnit");
        assert_eq!((listed_place(&app), app.focus, app.input_label().as_str()), (None, Focus::Review, "new thread · charge.rs:12"));
        let screen = render(&mut app, 138, 40);
        let rows: Vec<&str> = screen.lines().collect();
        let top = rows.iter().position(|row| row.contains("╭ new thread · charge.rs:12")).unwrap_or_else(|| panic!("a box:\n{screen}"));
        assert!(rows[top + 1].contains("nit") && rows[..top].iter().any(|row| row.contains("pub async fn charge")), "{screen}");
        let actions = app.handle_key(code(KeyCode::Enter));
        assert!(matches!(actions.as_slice(), [Action::SaveDraft { .. }]), "{actions:?}");
        press(&mut app, "j");
        assert_eq!((stop_of(&app), app.draft_here()), (Some(0), Some(0)));
        let screen = render(&mut app, 138, 40);
        let draft = under(&screen, "pub async fn charge(card", 3);
        assert_eq!(draft, ["▎           │ ◇ draft", "▎           │ you · unsaved ◇", "▎           │ nit"]);
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
    }
}
