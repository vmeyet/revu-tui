use super::{Action, App, MrKey};
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
    /// The conversations of one place, when the right pane holds them.
    pub pane: Option<super::pane::Pane>,
    /// How old the painted data was when it came from the cache, and when that was.
    pub cached: Option<(Instant, Duration)>,
    /// Where `V` started; the selection runs from there to the cursor.
    pub select_from: Option<usize>,
    /// The file tree, when it holds the right pane.
    pub tree: Option<super::Tree>,
    /// Claude's answer, when it holds the right pane.
    pub answer: Option<super::ask::Answer>,
    /// The CI run, when it holds the right pane.
    pub pipeline: Option<super::Pipeline>,
    /// The file row pinned above the diff at the last draw, so `za` and `zc` fold that file.
    pub pinned_file: Option<usize>,
}

impl Open {
    pub fn new(key: MrKey, review: Review) -> Self {
        let rows = review.rows();
        let selected = first_selectable(&rows);
        Self {
            key,
            review,
            rows,
            selected,
            scroll: 0,
            pane: None,
            cached: None,
            select_from: None,
            tree: None,
            answer: None,
            pipeline: None,
            pinned_file: None,
        }
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
    /// The reader's inline or split choice outlives the refresh.
    pub fn with_review(&self, review: Review) -> Self {
        let review = Review {
            split: self.review.split,
            quiet_whitespace: self.review.quiet_whitespace,
            context: self.review.context.clone(),
            ..review
        };
        let rows = review.rows();
        let selected = self
            .row()
            .and_then(|row| rows.iter().position(|r| same_place(r, row)))
            .unwrap_or(self.selected)
            .min(rows.len().saturating_sub(1));
        Self { review, rows, selected, ..self.clone() }
    }

    pub fn row(&self) -> Option<&Row> {
        self.rows.get(self.selected)
    }

    /// The age of what is on screen, for the `offline` line.
    pub fn staleness(&self, now: Instant) -> Option<Duration> {
        self.cached.map(|(at, age)| age + now.saturating_duration_since(at))
    }

    pub(super) fn move_to(&self, index: usize) -> Self {
        Self { selected: index.min(self.rows.len().saturating_sub(1)), ..self.clone() }
    }

    fn move_by(&self, delta: isize) -> Self {
        let on_the_mr = !self.review.conversations(&crate::review::Place::Mr).is_empty();
        let selectable = |i: usize| is_selectable(&self.rows[i], on_the_mr);
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

    /// The next row index after the cursor that `wanted` accepts, wrapping around; None when there is none.
    fn seek(&self, forward: bool, wanted: impl Fn(usize) -> bool) -> Option<usize> {
        let len = self.rows.len();
        (1..len).map(|k| if forward { (self.selected + k) % len } else { (self.selected + len - k) % len }).find(|&i| wanted(i))
    }

    fn fold_target(&self) -> Option<(String, Option<usize>)> {
        let row = self.row()?;
        let file = &self.review.files[row.file()?];
        let hunk = match row {
            Row::Hunk { index, .. } => Some(*index),
            Row::Line { hunk, .. } | Row::Pair { hunk, .. } | Row::Context { hunk, .. } => Some(*hunk),
            _ => None,
        };
        Some((file.new_path.clone(), hunk))
    }

    fn with_fold(&self, fold: FoldState) -> Self {
        self.relaid(self.review.with_fold(fold))
    }

    fn with_split(&self, split: bool) -> Self {
        self.relaid(self.review.with_split(split))
    }

    fn with_quiet_whitespace(&self, quiet: bool) -> Self {
        self.relaid(self.review.with_quiet_whitespace(quiet))
    }

    /// The same review laid out again, the cursor kept on what it pointed at.
    pub(super) fn relaid(&self, review: Review) -> Self {
        let rows = review.rows();
        let anchor = self.row().cloned();
        let selected =
            anchor.and_then(|row| rows.iter().position(|r| same_place(r, &row))).unwrap_or(self.selected).min(rows.len().saturating_sub(1));
        Self { review, rows, selected, ..self.clone() }
    }

    /// The web page of the line under the cursor, the MR's page anywhere else.
    pub fn line_url(&self, kind: Kind) -> String {
        let mr = &self.review.mr;
        let (file, hunk, index) = match self.row() {
            Some(Row::Line { file, hunk, index } | Row::Pair { file, hunk, added: index, .. }) => (*file, *hunk, *index),
            _ => return mr.web_url.clone(),
        };
        let file = &self.review.files[file];
        let line = &file.hunks[hunk].lines[index];
        kind.line_url(&mr.web_url, &file.new_path, LineRef { old: line.old, new: line.new })
    }
}

/// Gaps are air, and the header row only holds something when the MR itself has conversations.
fn is_selectable(row: &Row, on_the_mr: bool) -> bool {
    match row {
        Row::Gap => false,
        Row::Header => on_the_mr,
        _ => true,
    }
}

/// Where the cursor starts: the first file, even when the header holds conversations.
fn first_selectable(rows: &[Row]) -> usize {
    rows.iter().position(|row| is_selectable(row, false)).unwrap_or(0)
}

/// A row still names the same thing once folds changed, even if its `open` flag flipped.
fn same_place(before: &Row, after: &Row) -> bool {
    match (before, after) {
        (Row::File { index: was, .. }, Row::File { index: is, .. }) => was == is,
        (Row::Hunk { file: file_was, index: was, .. }, Row::Hunk { file: file_is, index: is, .. }) => file_was == file_is && was == is,
        (Row::Pair { file, hunk, removed, added }, Row::Line { file: line_file, hunk: line_hunk, index })
        | (Row::Line { file: line_file, hunk: line_hunk, index }, Row::Pair { file, hunk, removed, added }) => {
            file == line_file && hunk == line_hunk && (index == removed || index == added)
        }
        _ => before == after,
    }
}

/// How many lines `+` adds on each side of a hunk.
const CONTEXT_STEP: u32 = 10;

impl App {
    pub(super) fn review_move(&mut self, delta: isize) {
        if let Some(open) = &self.open {
            self.open = Some(open.move_by(delta));
        }
    }

    pub(super) fn review_jump(&mut self, forward: bool, wanted: impl Fn(&Row) -> bool) {
        let Some(open) = &self.open else { return };
        let rows = open.rows.clone();
        self.review_jump_where(forward, |index| wanted(&rows[index]));
    }

    /// Moves to the next row index `wanted` accepts, wrapping around.
    pub(super) fn review_jump_where(&mut self, forward: bool, wanted: impl Fn(usize) -> bool) {
        let Some(open) = &self.open else { return };
        match open.seek(forward, wanted) {
            Some(index) => self.open = Some(open.move_to(index)),
            None => self.toast("nothing to jump to"),
        }
    }

    /// Moves to the first row `wanted` accepts, from the top.
    pub(super) fn review_jump_to(&mut self, wanted: impl Fn(&Row) -> bool) {
        let Some(open) = &self.open else { return };
        if let Some(index) = open.rows.iter().position(wanted) {
            self.open = Some(open.move_to(index));
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
    /// While a file header is pinned above the diff, `za` and `zc` fold that file instead.
    pub(super) fn fold_at_cursor(&mut self, want_open: Option<bool>) -> Vec<Action> {
        if want_open != Some(true)
            && let Some(actions) = self.fold_pinned_file()
        {
            return actions;
        }
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

    /// Folds the pinned file and puts the cursor on its row, when the cursor sits inside it on a
    /// line; on a hunk row the hunk still folds, as without a pin.
    fn fold_pinned_file(&mut self) -> Option<Vec<Action>> {
        let open = self.open.as_ref()?;
        let Row::File { index: file, .. } = open.rows.get(open.pinned_file?)? else { return None };
        let file = *file;
        let row = open.row()?;
        if !matches!(row, Row::Line { .. } | Row::Pair { .. } | Row::Context { .. }) || row.file() != Some(file) {
            return None;
        }
        let path = open.review.files[file].new_path.clone();
        let actions = self.set_file_fold(&path, crate::diff::fold::Fold::Closed);
        let open = self.open.as_ref()?;
        let at = open.rows.iter().position(|r| matches!(r, Row::File { index, .. } if *index == file))?;
        self.open = Some(Open { pinned_file: None, ..open.move_to(at) });
        Some(actions)
    }

    pub(super) fn fold_all(&mut self, closed: bool) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let paths: Vec<String> = open.review.files.iter().map(|f| f.new_path.clone()).collect();
        let next = if closed { open.review.fold.fold_all(&paths) } else { FoldState::default() };
        self.apply_fold(next)
    }

    /// One file open or folded, whatever it was, and saved.
    pub(super) fn set_file_fold(&mut self, path: &str, fold: crate::diff::fold::Fold) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let current = &open.review.fold;
        let is_open = current.file_is_open(path);
        let wanted_open = fold == crate::diff::fold::Fold::Open;
        let next = if is_open == wanted_open { current.clone() } else { current.toggle_file(path) };
        self.apply_fold(next)
    }

    fn apply_fold(&mut self, fold: FoldState) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        self.keep(open.with_fold(fold))
    }

    /// `D`: changed words inline, or every changed line on its own row.
    pub(super) fn toggle_split(&mut self) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let next = open.with_split(!open.review.split);
        self.toast(if next.review.split { "split diff" } else { "inline diff" });
        self.keep(next)
    }

    /// `+`: ten more unchanged lines above and below the hunk under the cursor, read from the whole file.
    pub(super) fn expand_context(&mut self) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let target = match open.row() {
            Some(
                Row::Hunk { file, index: hunk, .. }
                | Row::Line { file, hunk, .. }
                | Row::Pair { file, hunk, .. }
                | Row::Context { file, hunk, .. },
            ) => Some((*file, *hunk)),
            _ => None,
        };
        let Some((file, hunk)) = target else {
            self.toast("move into a hunk first");
            return vec![];
        };
        let mut context = open.review.context.clone();
        *context.around.entry((file, hunk)).or_insert(0) += CONTEXT_STEP;
        let path = open.review.files[file].new_path.clone();
        let load = (!context.texts.contains_key(&path)).then(|| Action::LoadFile {
            key: open.key.clone(),
            path,
            sha: open.review.mr.refs.head.clone(),
        });
        self.open = Some(open.relaid(open.review.with_context(context)));
        load.into_iter().collect()
    }

    /// A file read whole arrived: the hunks waiting on it show their extra lines.
    pub(super) fn apply_file(&mut self, key: &MrKey, path: String, text: &str) {
        let Some(open) = self.open.as_ref().filter(|o| &o.key == key) else { return };
        let mut context = open.review.context.clone();
        context.texts.insert(path, std::sync::Arc::new(text.lines().map(str::to_owned).collect()));
        self.open = Some(open.relaid(open.review.with_context(context)));
    }

    /// `W`: lines that changed only in whitespace read as one quiet row, or show as they are.
    pub(super) fn toggle_whitespace(&mut self) {
        let Some(open) = &self.open else { return };
        let next = open.with_quiet_whitespace(!open.review.quiet_whitespace);
        self.toast(if next.review.quiet_whitespace { "whitespace-only changes hidden" } else { "whitespace changes shown" });
        self.open = Some(next);
    }

    /// Shows `next` and saves what the reader chose in it: folds, viewed files, split.
    fn keep(&mut self, next: Open) -> Vec<Action> {
        let review = &next.review;
        let action = Action::SaveState {
            key: next.key.clone(),
            fold: review.fold.clone(),
            viewed: review.viewed_fingerprints(),
            split: review.split,
        };
        self.open = Some(next);
        vec![action]
    }

    pub(super) fn enter_review_row(&mut self) -> Vec<Action> {
        if self.open_pane_here() {
            return vec![];
        }
        match self.open.as_ref().and_then(|o| o.row().cloned()) {
            Some(Row::File { .. } | Row::Hunk { .. }) => self.fold_at_cursor(None),
            _ => vec![],
        }
    }
}
