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

    fn with_pane(&self, pane: Option<Pane>) -> Self {
        Self { pane, ..self.clone() }
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
        let Some(open) = &self.open else { return };
        self.open = Some(Open { tree: None, answer: None, pipeline: None, ..open.with_pane(Some(Pane::at(place))) });
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
        self.zen && self.focus == Focus::Review && self.open.as_ref().is_some_and(Open::lists_every_thread)
    }

    /// `m` in the list of every conversation: only those I take part in, or every one again.
    fn toggle_mine(&mut self) {
        let Some(open) = &self.open else { return };
        let Some(pane) = &open.pane else { return };
        let only_with = if pane.only_with.is_some() { None } else { Some(self.me.clone()) };
        self.open = Some(open.with_pane(Some(Pane { note: 0, scroll: 0, only_with, ..pane.clone() })));
    }

    pub(super) fn close_pane(&mut self) {
        if let Some(open) = &self.open {
            self.open = Some(open.with_pane(None));
        }
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
        self.open = Some(open.with_pane(Some(Pane::at(place))));
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
    /// zen shows the diff there, since the list takes its place.
    fn jump_to_focused(&mut self) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let Some((conversations, _, Some(focused))) = open.pane_view() else { return vec![] };
        let Some(target) = row_of(&open.review, open.review.spot(&conversations[focused.conversation])) else {
            self.toast("its line is not in the diff");
            return vec![];
        };
        let actions = self.jump_to_row(&target);
        if self.zen {
            self.focus = Focus::Review;
        }
        actions
    }

    /// The cursor on `target`; its folded file or hunk opens, and stays open like one opened by hand.
    fn jump_to_row(&mut self, target: &Row) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let fold = unfolded_for(open, target);
        let changed = fold != open.review.fold;
        let laid = if changed { open.relaid(open.review.with_fold(fold)) } else { open.clone() };
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
        self.open = Some(open.with_pane(Some(Pane { note, ..pane.clone() })));
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
        self.open = Some(open.with_pane(Some(Pane { note, ..pane.clone() })));
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
        self.open = Some(open.with_pane(Some(Pane { unfolded, ..pane.clone() })));
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
    let every = open.review.with_fold(crate::diff::fold::FoldState::default()).rows();
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
            review.with_fold(crate::diff::fold::FoldState::default()).rows().into_iter().find(|row| review.row_holds(row, anchor))
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

    #[test]
    fn first_link_stops_at_whitespace_and_brackets() {
        assert_eq!(first_link("see https://a.b/c) now"), Some("https://a.b/c".into()));
        assert_eq!(first_link("[x](http://a.b/c?d=1)"), Some("http://a.b/c?d=1".into()));
        assert_eq!(first_link("no link"), None);
    }
}
