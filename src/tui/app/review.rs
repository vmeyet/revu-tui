use super::{Action, App, Focus, MrKey};
use crate::diff::fold::FoldState;
use crate::forge::{Kind, LineRef};
use crate::review::{Review, Row};
use std::time::{Duration, Instant};

/// The MR on screen: the review plus where the reader is in it.
#[derive(Clone, Debug, PartialEq)]
pub struct Open {
    pub key: MrKey,
    pub review: Review,
    pub rows: Vec<Row>,
    pub selected: usize,
    pub scroll: usize,
    pub thread: Option<String>,
    pub thread_scroll: usize,
    /// How old the painted data was when it came from the cache, and when that was.
    pub cached: Option<(Instant, Duration)>,
    /// Where `V` started; the selection runs from there to the cursor.
    pub select_from: Option<usize>,
}

impl Open {
    pub fn new(key: MrKey, review: Review) -> Self {
        let rows = review.rows();
        let selected = first_selectable(&rows);
        Self { key, review, rows, selected, scroll: 0, thread: None, thread_scroll: 0, cached: None, select_from: None }
    }

    /// The rows the selection covers, the cursor included; just the cursor without `V`.
    pub fn selection(&self) -> std::ops::RangeInclusive<usize> {
        let from = self.select_from.unwrap_or(self.selected);
        from.min(self.selected)..=from.max(self.selected)
    }

    pub fn is_selected(&self, index: usize) -> bool {
        self.select_from.is_some() && self.selection().contains(&index)
    }

    /// Fresh data under the same cursor: the row it was on is found again, else the index is kept.
    pub fn with_review(&self, review: Review) -> Self {
        let rows = review.rows();
        let selected = self
            .row()
            .and_then(|row| rows.iter().position(|r| same_place(r, row)))
            .unwrap_or(self.selected)
            .min(rows.len().saturating_sub(1));
        let thread = self.thread.clone().filter(|id| review.thread(id).is_some());
        Self { review, rows, selected, thread, ..self.clone() }
    }

    pub fn row(&self) -> Option<&Row> {
        self.rows.get(self.selected)
    }

    /// The age of what is on screen, for the `offline` line.
    pub fn staleness(&self, now: Instant) -> Option<Duration> {
        self.cached.map(|(at, age)| age + now.saturating_duration_since(at))
    }

    fn file_of(&self, row: &Row) -> Option<usize> {
        match row {
            Row::File { index, .. } | Row::Outdated { file: index } => Some(*index),
            Row::Hunk { file, .. } | Row::Line { file, .. } => Some(*file),
            Row::Thread { id } => {
                let path = self.review.thread(id)?.anchor.as_ref()?.path.clone();
                self.review.files.iter().position(|f| f.new_path == path || f.old_path == path)
            }
            Row::Draft { index } => {
                let path = self.review.drafts.get(*index)?.anchor.as_ref()?.path.clone();
                self.review.files.iter().position(|f| f.new_path == path || f.old_path == path)
            }
            Row::Header | Row::Gap => None,
        }
    }

    fn move_to(&self, index: usize) -> Self {
        Self { selected: index.min(self.rows.len().saturating_sub(1)), ..self.clone() }
    }

    fn move_by(&self, delta: isize) -> Self {
        let selectable = |i: usize| is_selectable(&self.rows[i]);
        let mut at = self.selected as isize;
        let mut left = delta.abs();
        let step = delta.signum();
        while left > 0 {
            let next = at + step;
            if next < 0 || next >= self.rows.len() as isize {
                break;
            }
            at = next;
            if selectable(at as usize) {
                left -= 1;
            }
        }
        self.move_to(at as usize)
    }

    /// The next row after the cursor that `wanted` accepts, wrapping around; None when there is none.
    fn seek(&self, forward: bool, wanted: impl Fn(&Row) -> bool) -> Option<usize> {
        let len = self.rows.len();
        (1..len).map(|k| if forward { (self.selected + k) % len } else { (self.selected + len - k) % len }).find(|&i| wanted(&self.rows[i]))
    }

    fn fold_target(&self) -> Option<(String, Option<usize>)> {
        let row = self.row()?;
        let file = &self.review.files[self.file_of(row)?];
        let hunk = match row {
            Row::Hunk { index, .. } => Some(*index),
            Row::Line { hunk, .. } => Some(*hunk),
            _ => None,
        };
        Some((file.new_path.clone(), hunk))
    }

    fn with_fold(&self, fold: FoldState) -> Self {
        let review = self.review.with_fold(fold);
        let rows = review.rows();
        let anchor = self.row().cloned();
        let selected =
            anchor.and_then(|row| rows.iter().position(|r| same_place(r, &row))).unwrap_or(self.selected).min(rows.len().saturating_sub(1));
        Self { review, rows, selected, ..self.clone() }
    }

    /// The web page of the line under the cursor, the MR's page anywhere else.
    pub fn line_url(&self, kind: Kind) -> String {
        let mr = &self.review.mr;
        let Some(Row::Line { file, hunk, index }) = self.row() else { return mr.web_url.clone() };
        let file = &self.review.files[*file];
        let line = &file.hunks[*hunk].lines[*index];
        kind.line_url(&mr.web_url, &file.new_path, LineRef { old: line.old, new: line.new })
    }
}

/// The header is drawn above the rows and gaps are air: the cursor lands on neither.
fn is_selectable(row: &Row) -> bool {
    !matches!(row, Row::Header | Row::Gap)
}

fn first_selectable(rows: &[Row]) -> usize {
    rows.iter().position(is_selectable).unwrap_or(0)
}

/// A row still names the same thing once folds changed, even if its `open` flag flipped.
fn same_place(before: &Row, after: &Row) -> bool {
    match (before, after) {
        (Row::File { index: was, .. }, Row::File { index: is, .. }) => was == is,
        (Row::Hunk { file: file_was, index: was, .. }, Row::Hunk { file: file_is, index: is, .. }) => file_was == file_is && was == is,
        _ => before == after,
    }
}

impl App {
    pub(super) fn review_move(&mut self, delta: isize) {
        if let Some(open) = &self.open {
            self.open = Some(open.move_by(delta));
        }
    }

    pub(super) fn review_jump(&mut self, forward: bool, wanted: impl Fn(&Row) -> bool) {
        let Some(open) = &self.open else { return };
        match open.seek(forward, wanted) {
            Some(index) => self.open = Some(open.move_to(index)),
            None => self.toast("nothing to jump to"),
        }
    }

    pub(super) fn review_first(&mut self) {
        if let Some(open) = &self.open {
            self.open = Some(open.move_to(first_selectable(&open.rows)));
        }
    }

    pub(super) fn review_last(&mut self) {
        if let Some(open) = &self.open {
            self.open = Some(open.move_to(open.rows.len().saturating_sub(1)));
        }
    }

    /// Indexes of the files carrying an unresolved thread, for `]f` and `[f`.
    pub(super) fn files_with_unresolved(&self) -> Vec<usize> {
        let Some(open) = &self.open else { return vec![] };
        let anchored = |path: &str| {
            open.review.threads.iter().any(|t| t.resolvable && !t.resolved && t.anchor.as_ref().is_some_and(|a| a.path == path))
        };
        (0..open.review.files.len())
            .filter(|&i| anchored(&open.review.files[i].new_path) || anchored(&open.review.files[i].old_path))
            .collect()
    }

    /// `za`: the file or hunk under the cursor; `zo` and `zc` only move in one direction.
    pub(super) fn fold_at_cursor(&mut self, want_open: Option<bool>) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let Some((path, hunk)) = open.fold_target() else { return vec![] };
        let fold = &open.review.fold;
        let is_open = match hunk {
            Some(index) => fold.hunk_is_open(&path, index),
            None => fold.file_is_open(&path),
        };
        if want_open == Some(is_open) {
            return vec![];
        }
        let next = match hunk {
            Some(index) => fold.toggle_hunk(&path, index),
            None => fold.toggle_file(&path),
        };
        self.apply_fold(next)
    }

    pub(super) fn fold_all(&mut self, closed: bool) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let paths: Vec<String> = open.review.files.iter().map(|f| f.new_path.clone()).collect();
        let next = if closed { open.review.fold.fold_all(&paths) } else { FoldState::default() };
        self.apply_fold(next)
    }

    fn apply_fold(&mut self, fold: FoldState) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let next = open.with_fold(fold);
        let action = Action::SaveState { key: next.key.clone(), fold: next.review.fold.clone(), viewed: next.review.viewed.clone() };
        self.open = Some(next);
        vec![action]
    }

    pub(super) fn enter_review_row(&mut self) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        match open.row().cloned() {
            Some(Row::Thread { id }) => {
                self.open = Some(Open { thread: Some(id), thread_scroll: 0, ..open.clone() });
                self.focus = Focus::Side;
                vec![]
            }
            Some(Row::Outdated { file }) => {
                let first = open.review.outdated(&open.review.files[file].new_path).first().map(|t| t.id.clone());
                self.open = Some(Open { thread: first, thread_scroll: 0, ..open.clone() });
                self.focus = Focus::Side;
                vec![]
            }
            Some(Row::File { .. } | Row::Hunk { .. }) => self.fold_at_cursor(None),
            Some(Row::Draft { .. }) => {
                self.edit_draft_here();
                vec![]
            }
            _ => vec![],
        }
    }

    pub(super) fn close_thread(&mut self) {
        if let Some(open) = &self.open {
            self.open = Some(Open { thread: None, thread_scroll: 0, ..open.clone() });
        }
        self.focus = Focus::Review;
    }

    pub(super) fn thread_scroll(&mut self, delta: isize) {
        if let Some(open) = &self.open {
            let scroll = (open.thread_scroll as isize + delta).max(0) as usize;
            self.open = Some(Open { thread_scroll: scroll, ..open.clone() });
        }
    }

    /// The first `http` link in the open thread, for `u`.
    pub(super) fn thread_link(&self) -> Option<String> {
        let open = self.open.as_ref()?;
        let thread = open.review.thread(open.thread.as_ref()?)?;
        thread.notes.iter().find_map(|n| first_link(&n.body))
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
