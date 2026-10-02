//! The right pane: the conversations of one place, opened from a marked line and following the cursor,
//! or every conversation of the MR, opened with `T`.
use super::{Action, App, Focus, Open};
use crate::review::{Conversation, Mark, Markers, Place, Review, Row, Spot};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::BTreeSet;

const HALF_PAGE: usize = 5;

/// Which conversations `]n` stops on: open ones, or every one, resolved included (`]N`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Threads {
    Open,
    Every,
}

impl Threads {
    /// A line whose threads are all resolved is not open; one of my drafts keeps it open.
    fn stops_at(self, mark: Mark) -> bool {
        self == Self::Every || mark > Mark::Resolved
    }
}

/// What the pane shows and where the reader is in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pane {
    pub place: Place,
    /// The entry under the cursor bar, counted over every conversation in order.
    pub note: usize,
    pub scroll: usize,
    /// Resolved threads the reader unfolded, by id; the others show their first note only.
    pub unfolded: BTreeSet<String>,
    /// `m` in the list of every thread: only the conversations this user takes part in.
    pub only_with: Option<String>,
}

impl Pane {
    pub fn at(place: Place) -> Self {
        Self { place, note: 0, scroll: 0, unfolded: BTreeSet::new(), only_with: None }
    }
}

/// One stop of the cursor bar: a note of a thread, or one of my drafts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub conversation: usize,
    pub kind: EntryKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    /// The note at this index of the thread.
    Note(usize),
    /// The draft at this index of `Review::drafts`.
    Draft(usize),
}

/// Every stop of the cursor bar at `place`: all notes of an open thread and my replies after
/// them, the first note alone of a folded resolved thread, my new draft on its own.
pub fn entries(review: &Review, conversations: &[Conversation], unfolded: &BTreeSet<String>) -> Vec<Entry> {
    let mut entries = vec![];
    for (index, conversation) in conversations.iter().enumerate() {
        if let Some(thread) = conversation.thread.as_deref().and_then(|id| review.thread(id)) {
            let shown = if thread.resolved && !unfolded.contains(&thread.id) { 1 } else { thread.notes.len() };
            entries.extend((0..shown).map(|note| Entry { conversation: index, kind: EntryKind::Note(note) }));
        }
        entries.extend(conversation.drafts.iter().map(|&draft| Entry { conversation: index, kind: EntryKind::Draft(draft) }));
    }
    entries
}

impl Open {
    /// The conversations the pane lists, its cursor stops, and the one under the cursor.
    pub fn pane_view(&self) -> Option<(Vec<Conversation>, Vec<Entry>, Option<Entry>)> {
        let pane = self.pane.as_ref()?;
        let conversations: Vec<Conversation> = self
            .review
            .conversations(&pane.place)
            .into_iter()
            .filter(|conversation| pane.only_with.as_deref().is_none_or(|user| self.review.takes_part(conversation, user)))
            .collect();
        let entries = entries(&self.review, &conversations, &pane.unfolded);
        let focused = entries.get(pane.note.min(entries.len().saturating_sub(1))).copied();
        Some((conversations, entries, focused))
    }

    /// The thread under the pane's cursor.
    pub fn focused_thread(&self) -> Option<String> {
        let (conversations, _, focused) = self.pane_view()?;
        conversations.get(focused?.conversation)?.thread.clone()
    }

    /// My draft under the pane's cursor.
    pub fn focused_draft(&self) -> Option<usize> {
        match self.pane_view()?.2?.kind {
            EntryKind::Draft(index) => Some(index),
            EntryKind::Note(_) => None,
        }
    }

    /// The pane lists every conversation of the MR, not those of one place.
    pub fn lists_every_thread(&self) -> bool {
        self.pane.as_ref().is_some_and(|pane| pane.place == Place::All)
    }

    fn with_pane(self, pane: Option<Pane>) -> Self {
        Self { pane, ..self }
    }

    /// Whether the row's line carries a thread or a draft, or the header conversations on the MR.
    pub fn is_marked(&self, row: &Row) -> bool {
        self.mark_in(&self.review.markers(), row).is_some()
    }

    fn mark_in(&self, markers: &Markers, row: &Row) -> Option<Mark> {
        match row {
            Row::Header => self.review.mr_mark(),
            _ => self.review.marker_of(markers, row).map(|marker| marker.mark),
        }
    }
}

impl App {
    /// The pane on `place`, focused, in place of the file tree.
    pub(super) fn open_pane(&mut self, place: Place) {
        self.update_open(|open| Open { tree: None, answer: None, pipeline: None, outline: None, ..open.with_pane(Some(Pane::at(place))) });
        self.focus = Focus::Side;
    }

    /// `enter` or `l` on a marked line, or the header with conversations on the MR.
    pub(super) fn open_pane_here(&mut self) -> bool {
        let Some(open) = &self.open else { return false };
        let Some(row) = open.row().filter(|row| open.is_marked(row)).cloned() else { return false };
        let Some(place) = open.review.place_of(&row) else { return false };
        self.open_pane(place);
        true
    }

    /// `l` on a file row with outdated threads opens them.
    pub(super) fn open_outdated_here(&mut self) -> bool {
        let Some(open) = &self.open else { return false };
        let Some(Row::File { index, .. }) = open.row() else { return false };
        if open.review.outdated(&open.review.files[*index].new_path).is_empty() {
            return false;
        }
        self.open_pane(Place::Outdated { file: *index });
        true
    }

    /// `T`: every conversation of the MR, or the pane closed when it already lists them; in zen,
    /// while the diff hides the list, the list shown again.
    pub(super) fn toggle_every_thread(&mut self) {
        let Some(open) = &self.open else { return };
        if self.every_thread_hidden() {
            self.focus = Focus::Side;
        } else if open.lists_every_thread() {
            self.close_pane();
        } else if open.review.conversations(&Place::All).is_empty() {
            self.toast("no thread on this MR yet");
        } else {
            self.open_pane(Place::All);
        }
    }

    /// The list of every conversation is open, but zen shows the diff in its place.
    fn every_thread_hidden(&self) -> bool {
        self.zen && self.focus == Focus::Review && !self.areas.both_shown() && self.open.as_ref().is_some_and(Open::lists_every_thread)
    }

    /// `m` in the list of every conversation: only those I take part in, or every one again.
    fn toggle_mine(&mut self) {
        let Some(pane) = self.open.as_ref().and_then(|o| o.pane.clone()) else { return };
        let only_with = if pane.only_with.is_some() { None } else { Some(self.me.clone()) };
        self.update_open(|open| open.with_pane(Some(Pane { note: 0, scroll: 0, only_with, ..pane })));
    }

    pub(super) fn close_pane(&mut self) {
        self.update_open(|open| open.with_pane(None));
        self.focus = Focus::Review;
    }

    /// The pane follows the cursor onto another marked line; an unmarked line, a comment being
    /// written in the pane, or the list of every conversation keeps what it shows.
    pub(super) fn follow_cursor(&mut self) {
        if self.input.is_some() {
            return;
        }
        let Some(open) = self.open.as_ref().filter(|open| !open.lists_every_thread()) else { return };
        let Some(pane) = &open.pane else { return };
        let Some(row) = open.row().filter(|row| open.is_marked(row)) else { return };
        let Some(place) = open.review.place_of(row).filter(|place| *place != pane.place) else { return };
        self.update_open(|open| open.with_pane(Some(Pane::at(place))));
    }

    /// `]n` `[n`, `]N` `[N`: the next conversation in file then line order, folded or not; a folded
    /// file or hunk on the way opens, and stays open like one opened by hand. The pane follows when open.
    pub(super) fn jump_to_marked(&mut self, forward: bool, threads: Threads) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let Some(target) = next_marked(open, forward, threads) else {
            let text = match threads {
                Threads::Open => format!("no open thread · {} for every thread", self.keymap.label("]N")),
                Threads::Every => "nothing to jump to".to_owned(),
            };
            self.toast(text);
            return vec![];
        };
        self.jump_to_row(&target)
    }

    /// `enter` in the list of every conversation: the diff's cursor onto the focused one's line;
    /// zen shows the diff there when the list takes its place.
    fn jump_to_focused(&mut self) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let Some((conversations, _, Some(focused))) = open.pane_view() else { return vec![] };
        let Some(target) = row_of(&open.review, open.review.spot(&conversations[focused.conversation])) else {
            self.toast("its line is not in the diff");
            return vec![];
        };
        let actions = self.jump_to_row(&target);
        if self.zen && !self.areas.both_shown() {
            self.focus = Focus::Review;
        }
        actions
    }

    /// The cursor on `target`; its folded file or hunk opens, and stays open like one opened by hand.
    pub(super) fn jump_to_row(&mut self, target: &Row) -> Vec<Action> {
        let Some(open) = self.open.take() else { return vec![] };
        let fold = unfolded_for(&open, target);
        let changed = fold != open.review.fold;
        let laid = if changed { open.with_fold(fold) } else { open };
        let index = laid.rows.iter().position(|row| super::review::same_place(row, target)).unwrap_or(laid.selected);
        let next = laid.move_to(index);
        let actions = if changed {
            self.keep(next)
        } else {
            self.open = Some(next);
            vec![]
        };
        self.follow_cursor();
        actions
    }

    pub(super) fn handle_pane_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.pane_move(1),
            KeyCode::Char('k') | KeyCode::Up => self.pane_move(-1),
            KeyCode::Char('d') if ctrl => self.pane_move(HALF_PAGE as isize),
            KeyCode::Char('u') if ctrl => self.pane_move(-(HALF_PAGE as isize)),
            KeyCode::Char('g') => self.pane_move(isize::MIN / 2),
            KeyCode::Char('G') => self.pane_move(isize::MAX / 2),
            KeyCode::Char('J') => self.pane_jump(true),
            KeyCode::Char('K') => self.pane_jump(false),
            KeyCode::Enter if self.open.as_ref().is_some_and(Open::lists_every_thread) => {
                self.unfold_focused();
                return self.jump_to_focused();
            }
            KeyCode::Enter => self.unfold_focused(),
            KeyCode::Char('T') => self.toggle_every_thread(),
            KeyCode::Char('m') if self.open.as_ref().is_some_and(Open::lists_every_thread) => self.toggle_mine(),
            KeyCode::Char('u') => {
                let Some(url) = self.thread_link() else {
                    self.toast("no link in this thread");
                    return vec![];
                };
                return vec![Action::OpenUrl(url)];
            }
            KeyCode::Char('o') => return self.thread_url().map(|u| vec![Action::OpenUrl(u)]).unwrap_or_default(),
            KeyCode::Char('y') => return self.thread_url().map(|u| vec![Action::Yank(u)]).unwrap_or_default(),
            KeyCode::Char('v') if ctrl => return self.view_thread(),
            KeyCode::Char('r') => self.reply_here(),
            KeyCode::Char('R') => return self.toggle_resolved(),
            KeyCode::Char('S') => self.apply_here(),
            KeyCode::Char('+') => self.open_react(),
            KeyCode::Char('e') => {
                if !self.edit_draft_here() {
                    self.toast("e edits one of your drafts");
                }
            }
            KeyCode::Char('d') => return self.delete_draft_here(),
            KeyCode::Char('E') => return self.compose_draft_here(),
            KeyCode::Char('P') => self.open_publish(),
            KeyCode::Esc | KeyCode::Char('x') => self.close_pane(),
            _ => {}
        }
        vec![]
    }

    fn pane_move(&mut self, delta: isize) {
        let Some(open) = &self.open else { return };
        let Some((_, entries, _)) = open.pane_view() else { return };
        let Some(pane) = &open.pane else { return };
        let last = entries.len().saturating_sub(1) as isize;
        let note = (pane.note as isize).saturating_add(delta).clamp(0, last) as usize;
        let pane = Pane { note, ..pane.clone() };
        self.update_open(|open| open.with_pane(Some(pane)));
    }

    fn pane_jump(&mut self, forward: bool) {
        let Some(open) = &self.open else { return };
        let Some((_, entries, Some(focused))) = open.pane_view() else { return };
        let Some(pane) = &open.pane else { return };
        let target = if forward {
            entries.iter().position(|e| e.conversation > focused.conversation)
        } else {
            let previous = focused.conversation.checked_sub(1);
            previous.and_then(|c| entries.iter().position(|e| e.conversation == c))
        };
        let Some(note) = target else {
            self.toast(if forward { "last thread on this line" } else { "first thread on this line" });
            return;
        };
        let pane = Pane { note, ..pane.clone() };
        self.update_open(|open| open.with_pane(Some(pane)));
    }

    /// `enter` on a folded resolved thread shows all its notes, and folds it again.
    fn unfold_focused(&mut self) {
        let Some(open) = &self.open else { return };
        let (Some(id), Some(pane)) = (open.focused_thread(), &open.pane) else { return };
        if !open.review.thread(&id).is_some_and(|t| t.resolved) {
            return;
        }
        let mut unfolded = pane.unfolded.clone();
        if !unfolded.remove(&id) {
            unfolded.insert(id);
        }
        let pane = Pane { unfolded, ..pane.clone() };
        self.update_open(|open| open.with_pane(Some(pane)));
    }

    /// The first `http` link in the focused thread, for `u`.
    pub(super) fn thread_link(&self) -> Option<String> {
        let open = self.open.as_ref()?;
        let thread = open.review.thread(&open.focused_thread()?)?;
        thread.notes.iter().find_map(|n| first_link(&n.body))
    }

    /// The focused thread's page on the forge.
    fn thread_url(&self) -> Option<String> {
        let open = self.open.as_ref()?;
        let thread = open.review.thread(&open.focused_thread()?)?;
        Some(format!("{}#note_{}", open.review.mr.web_url, thread.first().id))
    }
}

fn first_link(text: &str) -> Option<String> {
    let start = text.find("http://").or_else(|| text.find("https://"))?;
    let rest = &text[start..];
    let end = rest.find(|c: char| c.is_whitespace() || matches!(c, ')' | '>' | ']')).unwrap_or(rest.len());
    Some(rest[..end].to_owned())
}

/// The next row `threads` stops on after the cursor, wrapping, among the rows the review shows with every fold open.
fn next_marked(open: &Open, forward: bool, threads: Threads) -> Option<Row> {
    let every = open.review.clone().with_fold(crate::diff::fold::FoldState::default()).rows();
    let markers = open.review.markers();
    let here = open.row().and_then(|row| every.iter().position(|r| super::review::same_place(r, row))).unwrap_or(0);
    let len = every.len();
    (1..=len)
        .map(|k| if forward { (here + k) % len } else { (here + len - k % len) % len })
        .find(|&i| i != here && open.mark_in(&markers, &every[i]).is_some_and(|mark| threads.stops_at(mark)))
        .map(|i| every[i].clone())
}

/// The row showing where a conversation hangs, among the rows the review shows with every fold open:
/// the header for the MR, the file row for an outdated line.
fn row_of(review: &Review, spot: Spot) -> Option<Row> {
    match spot {
        Spot::Mr => Some(Row::Header),
        Spot::Outdated(anchor) => review.file_of(anchor).map(|index| Row::File { index, open: true }),
        Spot::Line(anchor) => {
            review.clone().with_fold(crate::diff::fold::FoldState::default()).rows().into_iter().find(|row| review.row_holds(row, anchor))
        }
    }
}

/// The fold state with the target's file and hunk open; the rest as the reader left it.
fn unfolded_for(open: &Open, target: &Row) -> crate::diff::fold::FoldState {
    let fold = open.review.fold.clone();
    let Some(file) = target.file() else { return fold };
    let path = &open.review.files[file].new_path;
    let fold = if fold.file_is_open(path) { fold } else { fold.toggle_file(path) };
    let hunk = match target {
        Row::Hunk { index, .. } => Some(*index),
        Row::Line { hunk, .. } | Row::Pair { hunk, .. } | Row::Context { hunk, .. } => Some(*hunk),
        _ => None,
    };
    match hunk {
        Some(hunk) if !fold.hunk_is_open(path, hunk) => fold.toggle_hunk(path, hunk),
        _ => fold,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::tui::app::test_support::*;

    #[test]
    fn first_link_stops_at_whitespace_and_brackets() {
        assert_eq!(first_link("see https://a.b/c) now"), Some("https://a.b/c".into()));
        assert_eq!(first_link("[x](http://a.b/c?d=1)"), Some("http://a.b/c?d=1".into()));
        assert_eq!(first_link("no link"), None);
    }

    #[test]
    fn enter_toggles_a_hunk_and_opens_the_pane_on_a_marked_line() {
        let mut app = with_review();
        press(&mut app, "]c");
        app.handle_key(code(KeyCode::Enter));
        assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::Hunk { open: false, .. })));
        app.handle_key(code(KeyCode::Enter));
        press(&mut app, "]N");
        app.handle_key(code(KeyCode::Enter));
        assert_eq!(app.focus, Focus::Side);
        let open = app.open.as_ref().unwrap();
        assert_eq!(open.pane.as_ref().map(|p| &p.place), Some(&Place::Line { file: 0, new: None, old: Some(13) }));
        assert_eq!(open.focused_thread().as_deref(), Some("c0ffee00c0ffee00"));
        assert_eq!(press(&mut app, "u"), vec![], "no link in that thread");
        assert!(app.live_toast().is_some());
        app.handle_key(code(KeyCode::Esc));
        assert_eq!(app.focus, Focus::Review);
        assert_eq!(app.open.as_ref().unwrap().pane, None);
    }

    #[test]
    fn l_on_a_file_opens_its_outdated_threads_and_the_header_its_mr_threads() {
        let mut app = with_review();
        assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::File { index: 0, .. })));
        press(&mut app, "l");
        let open = app.open.as_ref().unwrap();
        assert_eq!(open.pane.as_ref().map(|p| &p.place), Some(&Place::Outdated { file: 0 }));
        assert_eq!(open.focused_thread().as_deref(), Some("9f2c0aa1d4e5b6c7"));
        press(&mut app, "x");
        assert_eq!(app.open.as_ref().unwrap().pane, None);
        press(&mut app, "k");
        assert_eq!(app.open.as_ref().unwrap().row(), Some(&Row::Header), "the header holds the thread on the MR");
        app.handle_key(code(KeyCode::Enter));
        assert_eq!(app.open.as_ref().unwrap().pane.as_ref().map(|p| &p.place), Some(&Place::Mr));
    }

    #[test]
    fn the_pane_follows_the_cursor_onto_marked_lines_only() {
        let mut app = with_review();
        press(&mut app, "]N");
        press(&mut app, "l");
        press(&mut app, "h");
        press(&mut app, "k");
        let open = app.open.as_ref().unwrap();
        assert_eq!(
            open.pane.as_ref().map(|p| &p.place),
            Some(&Place::Line { file: 0, new: None, old: Some(13) }),
            "an unmarked line keeps the thread"
        );
        assert!(render(&mut app, 140, 24).contains("↑ line -13"), "and says where it belongs");
        press(&mut app, "x");
        assert_eq!(app.open.as_ref().unwrap().pane, None);
    }

    #[test]
    fn a_range_comment_marks_its_other_lines_while_the_pane_is_on_it() {
        let mut app = with_review();
        on_line(&mut app);
        press(&mut app, "Vjjc");
        type_text(&mut app, "these three lines");
        let screen = render(&mut app, 160, 24);
        let line_12 = screen.lines().find(|l| l.contains("12   12")).expect("line 12 on screen");
        assert!(line_12.contains("│   12"), "the first line of the range shows the bar: {line_12}");
        let line_13 = screen.lines().find(|l| l.contains("13 +    let client")).expect("line 13 on screen");
        assert!(line_13.contains("◇"), "the last line carries the draft mark: {line_13}");
    }

    #[test]
    fn below_120_columns_the_pane_is_a_page_and_h_goes_back_to_the_diff() {
        let mut app = with_review();
        press(&mut app, "]N");
        press(&mut app, "l");
        let page = render(&mut app, 100, 20);
        assert!(page.contains("charge.rs:-13") && !page.contains("Queue"), "the pane alone:\n{page}");
        press(&mut app, "h");
        let diff = render(&mut app, 100, 20);
        assert!(diff.contains("let client = Client::new()") && !diff.contains("charge.rs:-13 ·"), "the diff alone:\n{diff}");
        assert!(app.open.as_ref().unwrap().pane.is_some(), "the pane waits for l");
    }

    /// Where `keys` stops, one press at a time, from the top.
    fn thread_stops(app: &mut App, keys: &str, presses: usize) -> Vec<Row> {
        (0..presses)
            .map(|_| {
                press(app, keys);
                app.open.as_ref().unwrap().row().cloned().unwrap()
            })
            .collect()
    }

    fn is_saved_fold(actions: &[Action]) -> bool {
        actions.iter().any(|a| matches!(a, Action::SaveState { .. }))
    }

    /// The review with the fixture's thread on `charge.rs` line 13 open again.
    fn with_open_thread() -> App {
        let mut app = with_review();
        let open = app.open.clone().unwrap();
        app.open = Some(open.relaid(|review| review.with_resolved("c0ffee00c0ffee00", false)));
        app
    }

    const LINE_13: Row = Row::Line { file: 0, hunk: 0, index: 1 };

    #[test]
    fn bracket_n_skips_resolved_threads_and_says_how_to_reach_them() {
        let mut app = with_review();
        assert_eq!(thread_stops(&mut app, "]n", 2), [Row::Header, Row::Header], "the resolved line 13 is skipped");
        assert_eq!(app.live_toast().unwrap().text, "no open thread · ]N for every thread");
        assert_eq!(thread_stops(&mut app, "]N", 2), [LINE_13, Row::Header], "]N stops on it and wraps");
        assert_eq!(thread_stops(&mut app, "[N", 1), [LINE_13]);
    }

    #[test]
    fn bracket_n_stops_on_my_draft_reply_to_a_resolved_thread() {
        let mut app = with_review();
        let open = app.open.clone().unwrap();
        let reply = crate::review::Draft::reply("c0ffee00c0ffee00", "agreed");
        app.open = Some(open.relaid(|review| review.with_drafts(vec![reply])));
        assert_eq!(thread_stops(&mut app, "]n", 2), [LINE_13, Row::Header]);
    }

    #[test]
    fn bracket_n_skips_the_header_once_the_mr_thread_is_resolved() {
        let mut app = with_open_thread();
        let open = app.open.clone().unwrap();
        app.open = Some(open.relaid(|review| review.with_resolved("6a9c1750b2d6e4f0", true)));
        assert_eq!(thread_stops(&mut app, "]n", 1), [LINE_13]);
        assert_eq!(thread_stops(&mut app, "]N", 1), [Row::Header], "]N still stops there");
    }

    #[test]
    fn bracket_n_opens_a_folded_file_and_lands_on_its_thread() {
        let mut app = with_open_thread();
        press(&mut app, "zM");
        assert!(!app.open.as_ref().unwrap().review.fold.file_is_open("src/pay/charge.rs"));
        press(&mut app, "g");
        let actions = press(&mut app, "]n");
        let open = app.open.as_ref().unwrap();
        assert!(open.review.fold.file_is_open("src/pay/charge.rs"), "the file opened on the way");
        assert_eq!(open.row(), Some(&LINE_13));
        assert!(is_saved_fold(&actions), "the unfold is kept like one done by hand");
    }

    #[test]
    fn bracket_big_n_opens_a_folded_file_on_the_way_to_a_resolved_thread() {
        let mut app = with_review();
        press(&mut app, "zMg");
        let actions = press(&mut app, "]N");
        assert_eq!(app.open.as_ref().unwrap().row(), Some(&LINE_13));
        assert!(is_saved_fold(&actions));
    }

    #[test]
    fn bracket_n_opens_a_folded_hunk_of_an_open_file() {
        let mut app = with_open_thread();
        let Row::Line { hunk, .. } = LINE_13 else { unreachable!() };
        app.review_jump_to(|r| matches!(r, Row::Hunk { file: 0, index, .. } if *index == hunk));
        press(&mut app, "za");
        assert!(!app.open.as_ref().unwrap().review.fold.hunk_is_open("src/pay/charge.rs", hunk));
        assert_eq!(thread_stops(&mut app, "]n", 1), [LINE_13], "the hunk opened and the cursor is on the thread");
        assert!(app.open.as_ref().unwrap().review.fold.hunk_is_open("src/pay/charge.rs", hunk));
    }

    #[test]
    fn bracket_n_order_is_unchanged_when_nothing_is_folded() {
        let mut open_app = with_open_thread();
        press(&mut open_app, "zRg");
        let stops = thread_stops(&mut open_app, "]n", 4);
        let mut folded = with_open_thread();
        press(&mut folded, "zMg");
        assert_eq!(thread_stops(&mut folded, "]n", 4), stops, "folds change nothing about where ]n stops");
        assert_eq!(stops[..2], stops[2..], "it wraps around in the same order");
    }

    #[test]
    fn bracket_n_backwards_opens_a_fold_too() {
        let mut app = with_open_thread();
        press(&mut app, "zMG");
        let actions = press(&mut app, "[n");
        assert_eq!(app.open.as_ref().unwrap().row(), Some(&LINE_13));
        assert!(is_saved_fold(&actions));
    }

    #[test]
    fn the_pane_keeps_its_thread_while_a_reply_is_written() {
        let mut app = with_review();
        press(&mut app, "]Nlr");
        assert!(app.input.is_some());
        let open = app.open.clone().unwrap();
        let shown = open.pane.clone().unwrap().place;
        let other = open.rows.iter().position(|row| open.is_marked(row) && open.review.place_of(row).is_some_and(|p| p != shown)).unwrap();
        app.open = Some(Open { selected: other, ..open });
        app.follow_cursor();
        assert_eq!(app.open.as_ref().unwrap().pane.as_ref().map(|p| &p.place), Some(&shown));
        app.handle_key(code(KeyCode::Esc));
        app.follow_cursor();
        assert_ne!(app.open.as_ref().unwrap().pane.as_ref().map(|p| &p.place), Some(&shown), "without the box it follows again");
    }

    #[test]
    fn t_lists_every_thread_and_t_esc_or_q_close_it() {
        let mut app = with_review();
        press(&mut app, "T");
        assert_eq!((listed_place(&app), app.focus), (Some(Place::All), Focus::Side));
        assert_eq!(focused_thread(&app).as_deref(), Some("6a9c1750b2d6e4f0"), "the MR's own thread comes first");
        press(&mut app, "T");
        assert_eq!((listed_place(&app), app.focus), (None, Focus::Review));
        for close in [code(KeyCode::Esc), key('q')] {
            press(&mut app, "T");
            app.handle_key(close);
            assert_eq!(listed_place(&app), None);
        }
        press(&mut app, ":threads");
        app.handle_key(code(KeyCode::Enter));
        assert_eq!(listed_place(&app), Some(Place::All), "the palette opens it too");
    }

    #[test]
    fn capital_j_k_walk_every_thread_and_r_and_capital_r_act_on_the_one_under_the_cursor() {
        let mut app = with_review();
        press(&mut app, "TJ");
        assert_eq!(focused_thread(&app).as_deref(), Some("9f2c0aa1d4e5b6c7"), "the outdated thread, still open");
        press(&mut app, "J");
        assert_eq!(focused_thread(&app).as_deref(), Some("c0ffee00c0ffee00"), "the resolved one last");
        press(&mut app, "K");
        assert_eq!(press(&mut app, "R"), vec![Action::Resolve { key: mr_key(), thread: "9f2c0aa1d4e5b6c7".into(), resolved: true }]);
        press(&mut app, "gJ");
        let mut actions = press(&mut app, "r");
        actions.extend(type_text(&mut app, "agreed"));
        let [Action::SaveDraft { draft, .. }] = actions.as_slice() else { panic!("{actions:?}") };
        assert_eq!(draft.reply_to.as_deref(), Some("c0ffee00c0ffee00"), "resolving moved the outdated thread down");
        assert_eq!(listed_place(&app), Some(Place::All));
    }

    #[test]
    fn e_and_d_in_every_thread_act_on_my_draft_and_keep_the_list() {
        let mut app = with_saved_draft();
        press(&mut app, "TJJ");
        assert_eq!(app.open.as_ref().unwrap().focused_draft(), Some(0));
        press(&mut app, "e");
        assert_eq!((app.input_label(), listed_place(&app)), ("edit draft".to_owned(), Some(Place::All)));
        app.handle_key(code(KeyCode::Esc));
        assert_eq!(press(&mut app, "d"), vec![Action::DeleteDraft { key: mr_key(), id: 9 }]);
    }

    #[test]
    fn enter_in_every_thread_takes_the_diff_to_the_line_and_opens_its_fold() {
        let mut app = with_review();
        press(&mut app, "zMTJJ");
        assert!(!app.open.as_ref().unwrap().review.fold.file_is_open("src/pay/charge.rs"));
        app.handle_key(code(KeyCode::Enter));
        let open = app.open.as_ref().unwrap();
        assert!(open.review.fold.file_is_open("src/pay/charge.rs"));
        assert_eq!(open.row().and_then(|row| open.review.place_of(row)), Some(Place::Line { file: 0, new: None, old: Some(13) }));
        assert!(open.pane.as_ref().unwrap().unfolded.contains("c0ffee00c0ffee00"), "a resolved thread unfolds as well");
        assert_eq!((listed_place(&app), app.focus), (Some(Place::All), Focus::Side));
        press(&mut app, "K");
        app.handle_key(code(KeyCode::Enter));
        assert_eq!(app.open.as_ref().unwrap().row(), Some(&Row::File { index: 0, open: true }), "an outdated thread goes to its file");
        press(&mut app, "g");
        app.handle_key(code(KeyCode::Enter));
        assert_eq!(app.open.as_ref().unwrap().row(), Some(&Row::Header), "the MR's own thread goes to the header");
    }

    #[test]
    fn moving_in_the_diff_leaves_every_thread_in_the_pane() {
        let mut app = with_review();
        press(&mut app, "Th]N");
        assert_eq!(app.focus, Focus::Review);
        let open = app.open.as_ref().unwrap();
        assert!(open.is_marked(open.row().unwrap()), "on a marked line");
        press(&mut app, "jk");
        assert_eq!(listed_place(&app), Some(Place::All));
    }

    #[test]
    fn in_zen_enter_in_every_thread_moves_the_diff_above_and_keeps_the_list() {
        let mut app = with_review();
        press(&mut app, "zzT");
        let list = render(&mut app, 138, 40);
        assert!(list.contains("the whole MR · 3 threads") && list.contains("@@ -12,4"), "the diff over the list:\n{list}");
        press(&mut app, "JJ");
        app.handle_key(code(KeyCode::Enter));
        let open = app.open.as_ref().unwrap();
        assert_eq!(open.row().and_then(|row| open.review.place_of(row)), Some(Place::Line { file: 0, new: None, old: Some(13) }));
        assert_eq!((listed_place(&app), app.focus), (Some(Place::All), Focus::Side), "the list keeps the keys");
        assert!(render(&mut app, 138, 40).contains("▎✓   13      -    let client = Client::new();"));
        press(&mut app, "hT");
        assert_eq!((listed_place(&app), app.focus, app.zen), (None, Focus::Review, true), "T from the diff closes the list it shows");
    }

    #[test]
    fn m_in_every_thread_keeps_mine_until_the_list_closes_and_the_keys_act_on_them() {
        let mut app = with_review();
        press(&mut app, "Tm");
        assert_eq!(focused_thread(&app).as_deref(), Some("c0ffee00c0ffee00"), "nina started the resolved thread only");
        press(&mut app, "J");
        assert_eq!(focused_thread(&app).as_deref(), Some("c0ffee00c0ffee00"), "nothing after it");
        assert_eq!(press(&mut app, "R"), vec![Action::Resolve { key: mr_key(), thread: "c0ffee00c0ffee00".into(), resolved: false }]);
        app.handle_key(code(KeyCode::Enter));
        assert_eq!(app.open.as_ref().unwrap().row(), Some(&Row::Line { file: 0, hunk: 0, index: 1 }));
        press(&mut app, "hl");
        assert_eq!(app.open.as_ref().unwrap().pane.as_ref().unwrap().only_with.as_deref(), Some("nina"), "kept while the list is open");
        let mut actions = press(&mut app, "r");
        actions.extend(type_text(&mut app, "agreed"));
        let [Action::SaveDraft { draft, .. }] = actions.as_slice() else { panic!("{actions:?}") };
        assert_eq!(draft.reply_to.as_deref(), Some("c0ffee00c0ffee00"));
        press(&mut app, "m");
        assert_eq!(focused_thread(&app).as_deref(), Some("6a9c1750b2d6e4f0"), "m again lists every thread");
        press(&mut app, "mTT");
        assert_eq!(app.open.as_ref().unwrap().pane.as_ref().unwrap().only_with, None, "a new list shows every thread");
    }
}
