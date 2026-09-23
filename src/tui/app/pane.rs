//! The right pane: the conversations of one place, opened from a marked line and following the cursor.
use super::{Action, App, Focus, Open};
use crate::review::{Conversation, Place, Review, Row};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::BTreeSet;

const HALF_PAGE: usize = 5;

/// What the pane shows and where the reader is in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pane {
    pub place: Place,
    /// The entry under the cursor bar, counted over every conversation in order.
    pub note: usize,
    pub scroll: usize,
    /// Resolved threads the reader unfolded, by id; the others show their first note only.
    pub unfolded: BTreeSet<String>,
}

impl Pane {
    pub fn at(place: Place) -> Self {
        Self { place, note: 0, scroll: 0, unfolded: BTreeSet::new() }
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
        let conversations = self.review.conversations(&pane.place);
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

    fn with_pane(&self, pane: Option<Pane>) -> Self {
        Self { pane, ..self.clone() }
    }

    /// Whether the row's line carries a thread or a draft, or the header conversations on the MR.
    pub fn is_marked(&self, row: &Row) -> bool {
        self.is_marked_in(&self.review.markers(), row)
    }

    fn is_marked_in(&self, markers: &crate::review::Markers, row: &Row) -> bool {
        match row {
            Row::Header => !self.review.conversations(&Place::Mr).is_empty(),
            _ => self.review.marker_of(markers, row).is_some(),
        }
    }
}

impl App {
    /// The pane on `place`, focused, in place of the file tree.
    pub(super) fn open_pane(&mut self, place: Place) {
        let Some(open) = &self.open else { return };
        self.open = Some(Open { tree: None, answer: None, ..open.with_pane(Some(Pane::at(place))) });
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

    pub(super) fn close_pane(&mut self) {
        if let Some(open) = &self.open {
            self.open = Some(open.with_pane(None));
        }
        self.focus = Focus::Review;
    }

    /// The pane follows the cursor onto another marked line; an unmarked line keeps what it shows.
    pub(super) fn follow_cursor(&mut self) {
        let Some(open) = &self.open else { return };
        let Some(pane) = &open.pane else { return };
        let Some(row) = open.row().filter(|row| open.is_marked(row)) else { return };
        let Some(place) = open.review.place_of(row).filter(|place| *place != pane.place) else { return };
        self.open = Some(open.with_pane(Some(Pane::at(place))));
    }

    /// `]n` `[n`: the next marked line, in any open file; the pane follows when open.
    pub(super) fn jump_to_marked(&mut self, forward: bool) {
        let Some(open) = &self.open else { return };
        let markers = open.review.markers();
        let marked: Vec<bool> = open.rows.iter().map(|row| open.is_marked_in(&markers, row)).collect();
        self.review_jump_where(forward, |index| marked[index]);
        self.follow_cursor();
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
            KeyCode::Enter => self.unfold_focused(),
            KeyCode::Char('u') => {
                let Some(url) = self.thread_link() else {
                    self.toast("no link in this thread");
                    return vec![];
                };
                return vec![Action::OpenUrl(url)];
            }
            KeyCode::Char('o') => return self.thread_url().map(|u| vec![Action::OpenUrl(u)]).unwrap_or_default(),
            KeyCode::Char('y') => return self.thread_url().map(|u| vec![Action::Yank(u)]).unwrap_or_default(),
            KeyCode::Char('v') => return self.view_thread(),
            KeyCode::Char('r') => self.reply_here(),
            KeyCode::Char('R') => return self.toggle_resolved(),
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
