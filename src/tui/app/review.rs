use super::{Action, App, MrKey};
use crate::diff::fold::FoldState;
use crate::forge::{Kind, LineRef, Sha};
use crate::review::{Draft, Review, Row, Side};
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
    /// Where the branch was deployed; `None` until the forge was asked.
    pub deployments: Option<Vec<crate::forge::Deployment>>,
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
            deployments: None,
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
    /// The reader's inline or side by side choice outlives the refresh; the lines read around hunks only while the head stays.
    pub fn with_review(self, review: Review) -> Self {
        let same_head = review.mr.refs.head == self.review.mr.refs.head;
        self.relaid(|old| Review {
            side_by_side: old.side_by_side,
            wide: old.wide,
            peek: old.peek,
            quiet_whitespace: old.quiet_whitespace,
            context: if same_head { old.context } else { crate::review::Context::default() },
            ..review
        })
    }

    /// The right pane holds something: threads, the tree, an answer or the pipeline.
    pub fn side_open(&self) -> bool {
        self.pane.is_some() || self.tree.is_some() || self.answer.is_some() || self.pipeline.is_some()
    }

    /// The right pane shows threads: no file tree, pipeline or answer took their place.
    pub fn shows_threads(&self) -> bool {
        self.pane.is_some() && self.tree.is_none() && self.answer.is_none() && self.pipeline.is_none()
    }

    pub fn row(&self) -> Option<&Row> {
        self.rows.get(self.selected)
    }

    /// The age of what is on screen, for the `offline` line.
    pub fn staleness(&self, now: Instant) -> Option<Duration> {
        self.cached.map(|(at, age)| age + now.saturating_duration_since(at))
    }

    pub(super) fn move_to(self, index: usize) -> Self {
        Self { selected: index.min(self.rows.len().saturating_sub(1)), ..self }
    }

    fn move_by(self, delta: isize) -> Self {
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

    pub(super) fn with_fold(self, fold: FoldState) -> Self {
        self.relaid(|review| review.with_fold(fold))
    }

    fn with_quiet_whitespace(self, quiet: bool) -> Self {
        self.relaid(|review| review.with_quiet_whitespace(quiet))
    }

    /// The review changed by `change` and laid out again, the cursor kept on what it pointed at.
    pub(super) fn relaid(self, change: impl FnOnce(Review) -> Review) -> Self {
        let anchor = self.row().cloned();
        let review = change(self.review);
        let rows = review.rows();
        let selected = anchor.and_then(|row| find_again(&rows, &row)).unwrap_or(self.selected).min(rows.len().saturating_sub(1));
        Self { review, rows, selected, ..self }
    }

    /// The drafts `change` makes of the current ones, laid out again.
    pub(super) fn with_drafts_changed(self, change: impl FnOnce(Vec<Draft>) -> Vec<Draft>) -> Self {
        self.relaid(|review| Review { drafts: change(review.drafts), ..review })
    }

    pub(super) fn with_draft_replaced(self, index: usize, draft: Draft) -> Self {
        self.with_drafts_changed(|mut drafts| {
            drafts[index] = draft;
            drafts
        })
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
pub(super) fn first_selectable(rows: &[Row]) -> usize {
    rows.iter().position(|row| is_selectable(row, false)).unwrap_or(0)
}

/// A row still names the same thing once folds changed, even if its `open` flag flipped.
pub(super) fn same_place(before: &Row, after: &Row) -> bool {
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

/// Where `row` sits in `rows`: itself or its pair, else the hunk it sat in.
fn find_again(rows: &[Row], row: &Row) -> Option<usize> {
    let hunk = row.hunk();
    let in_hunk = |r: &Row| hunk.is_some() && matches!(r, Row::Hunk { .. }) && r.hunk() == hunk;
    rows.iter().position(|r| same_place(r, row)).or_else(|| rows.iter().position(in_hunk))
}

/// The side `>` shows after `peek`, or `<` when not `forward`: diff, after, before, and round.
fn next_peek(peek: Option<Side>, forward: bool) -> Option<Side> {
    match (peek, forward) {
        (None, true) | (Some(Side::Old), false) => Some(Side::New),
        (Some(Side::New), true) | (None, false) => Some(Side::Old),
        (Some(Side::Old), true) | (Some(Side::New), false) => None,
    }
}

/// How many lines `+` adds on each side of a hunk.
const CONTEXT_STEP: u32 = 10;
const TOO_NARROW: &str = "side by side needs a wider window";

impl App {
    /// The open MR replaced by what `change` makes of it; nothing happens when none is open.
    pub(super) fn update_open(&mut self, change: impl FnOnce(Open) -> Open) {
        self.open = self.open.take().map(change);
    }

    /// The open MR replaced by what `change` makes of it, only while it is `key`.
    pub(super) fn update_open_of(&mut self, key: &MrKey, change: impl FnOnce(Open) -> Open) {
        self.open = self.open.take().map(|open| if open.key == *key { change(open) } else { open });
    }

    pub(super) fn review_move(&mut self, delta: isize) {
        self.update_open(|open| open.move_by(delta));
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
            Some(index) => self.update_open(|open| open.move_to(index)),
            None => self.toast("nothing to jump to"),
        }
    }

    /// Moves to the first row `wanted` accepts, from the top.
    pub(super) fn review_jump_to(&mut self, wanted: impl Fn(&Row) -> bool) {
        let Some(open) = &self.open else { return };
        if let Some(index) = open.rows.iter().position(wanted) {
            self.update_open(|open| open.move_to(index));
        }
    }

    pub(super) fn review_first(&mut self) {
        self.update_open(|open| {
            let first = first_selectable(&open.rows);
            open.move_to(first)
        });
    }

    pub(super) fn review_last(&mut self) {
        self.update_open(|open| {
            let last = open.rows.len().saturating_sub(1);
            open.move_to(last)
        });
    }

    /// Indexes of the files carrying an unresolved thread, for `]f` and `[f`.
    pub(super) fn files_with_unresolved(&self) -> Vec<usize> {
        let Some(open) = &self.open else { return vec![] };
        let anchored = |path: &str| open.review.threads.iter().any(|t| t.unresolved() && t.anchor.as_ref().is_some_and(|a| a.path == path));
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
        let at = self.open.as_ref()?.rows.iter().position(|r| matches!(r, Row::File { index, .. } if *index == file))?;
        self.update_open(|open| Open { pinned_file: None, ..open.move_to(at) });
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
        let Some(open) = self.open.take() else { return vec![] };
        self.keep(open.with_fold(fold))
    }

    /// `D`: changed words inline, or the old file beside the new one.
    pub(super) fn toggle_side_by_side(&mut self) -> Vec<Action> {
        let Some(open) = self.open.take() else { return vec![] };
        let next = open.relaid(|review| {
            let side_by_side = !review.side_by_side;
            review.with_side_by_side(side_by_side)
        });
        self.toast(match (next.review.side_by_side, next.review.wide) {
            (true, true) => "side by side",
            (true, false) => TOO_NARROW,
            (false, _) => "inline diff",
        });
        self.keep(next)
    }

    /// `>` and `<`: the diff, the code after, the code before, each key the other way round.
    pub(super) fn cycle_peek(&mut self, forward: bool) -> Vec<Action> {
        let Some(open) = self.open.take() else { return vec![] };
        let next = open.relaid(|review| {
            let peek = next_peek(review.peek, forward);
            review.with_peek(peek)
        });
        self.toast(match next.review.peek {
            Some(Side::Old) => "before",
            Some(Side::New) => "after",
            None => "diff",
        });
        self.open = Some(next);
        vec![]
    }

    /// The diff area is drawn wide enough for two sides, or not: side by side falls back to inline
    /// while it is narrow, saying so once, and comes back when it widens.
    pub(crate) fn fit_diff(&mut self, wide: bool) {
        let Some(open) = self.open.take_if(|o| o.review.wide != wide) else { return };
        let next = open.relaid(|review| review.with_wide(wide));
        if next.review.side_by_side && !wide {
            self.toast(TOO_NARROW);
        }
        self.open = Some(next);
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
        self.update_open(|open| open.relaid(|review| review.with_context(context)));
        load.into_iter().collect()
    }

    /// A file read whole arrived: the hunks waiting on it show their extra lines, unless a push moved the head since it was asked.
    pub(super) fn apply_file(&mut self, key: &MrKey, path: String, sha: &Sha, text: &str) {
        let Some(open) = self.open.take_if(|o| &o.key == key && &o.review.mr.refs.head == sha) else { return };
        let lines = std::sync::Arc::new(text.lines().map(str::to_owned).collect());
        self.open = Some(open.relaid(|review| {
            let mut context = review.context.clone();
            context.texts.insert(path, lines);
            review.with_context(context)
        }));
    }

    /// `W`: lines that changed only in whitespace read as one quiet row, or show as they are.
    pub(super) fn toggle_whitespace(&mut self) {
        let Some(open) = self.open.take() else { return };
        let quiet = !open.review.quiet_whitespace;
        let next = open.with_quiet_whitespace(quiet);
        self.toast(if next.review.quiet_whitespace { "whitespace-only changes hidden" } else { "whitespace changes shown" });
        self.open = Some(next);
    }

    /// Shows `next` and saves what the reader chose in it: folds, viewed files, side by side.
    pub(super) fn keep(&mut self, next: Open) -> Vec<Action> {
        self.count_viewed(&next.key, &next.review);
        self.open = Some(next);
        self.save_state(None).into_iter().collect()
    }

    /// What the reader chose in the open MR and, when given, where the cursor rests.
    pub(super) fn save_state(&mut self, spot: Option<super::Spot>) -> Option<Action> {
        self.next_save += 1;
        let open = self.open.as_ref()?;
        let review = &open.review;
        Some(Action::SaveState {
            key: open.key.clone(),
            order: self.next_save,
            fold: review.fold.clone(),
            viewed: review.viewed_fingerprints(),
            auto_folded: review.auto_folded.clone(),
            side_by_side: review.side_by_side,
            spot,
        })
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crate::tui::app::test_support::*;

    #[test]
    fn tab_and_brackets_jump_between_files_hunks_and_threads() {
        let mut app = with_review();
        press(&mut app, "]c");
        assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::Hunk { index: 0, .. })));
        press(&mut app, "]c");
        assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::Hunk { index: 1, .. })));
        press(&mut app, "]N");
        assert_eq!(app.open.as_ref().unwrap().row(), Some(&Row::Header), "wraps to the thread on the MR");
        press(&mut app, "]N");
        assert_eq!(app.open.as_ref().unwrap().row(), Some(&Row::Line { file: 0, hunk: 0, index: 1 }), "then the marked removed line");
        app.handle_key(code(KeyCode::Tab));
        assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::File { index: 1, .. })));
        app.handle_key(code(KeyCode::BackTab));
        assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::File { index: 0, .. })));
    }

    #[test]
    fn bracket_f_finds_files_with_unresolved_threads() {
        let mut app = with_review();
        press(&mut app, "G");
        press(&mut app, "]f");
        assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::File { index: 0, .. })));
        press(&mut app, "]f");
        assert!(app.live_toast().is_some(), "only one such file: nothing to jump to");
    }

    #[test]
    fn folds_save_state_and_keep_the_cursor_on_the_same_place() {
        let mut app = with_review();
        let actions = press(&mut app, "za");
        assert!(matches!(actions.as_slice(), [Action::SaveState { key, .. }] if *key == mr_key()));
        let open = app.open.as_ref().unwrap();
        assert_eq!(open.row(), Some(&Row::File { index: 0, open: false }));
        assert_eq!(open.rows.len(), 5, "header, gap, file, gap, file");
        assert_eq!(press(&mut app, "zc"), vec![], "already closed");
        press(&mut app, "zo");
        assert!(app.open.as_ref().unwrap().rows.len() > 5);
        press(&mut app, "zM");
        assert_eq!(app.open.as_ref().unwrap().rows.len(), 5);
        press(&mut app, "zR");
        assert!(app.open.as_ref().unwrap().review.fold.file_is_open("Cargo.lock"));
    }

    #[test]
    fn o_and_y_on_a_line_use_the_line_url() {
        let mut app = with_review();
        press(&mut app, "]cj");
        let url = match press(&mut app, "y").as_slice() {
            [Action::Yank(url)] => url.clone(),
            other => panic!("{other:?}"),
        };
        let digest = sha1_smol::Sha1::from("src/pay/charge.rs".as_bytes()).digest().to_string();
        assert_eq!(url, format!("https://gitlab.com/acme/widgets/-/merge_requests/42/diffs#{digest}_12_12"));
    }

    #[test]
    fn a_one_word_change_reads_as_one_row_with_its_thread_under_it() {
        let app = with_sum_review();
        let rows = &app.open.as_ref().unwrap().rows;
        let pair = rows.iter().position(|r| matches!(r, Row::Pair { removed: 2, added: 3, .. })).expect("the b line pairs up");
        let open = app.open.as_ref().unwrap();
        assert!(open.review.marker_of(&open.review.markers(), &rows[pair]).is_some(), "the thread on the added line marks the pair");
        let lines: Vec<usize> = rows.iter().filter_map(|r| if let Row::Line { index, .. } = r { Some(*index) } else { None }).collect();
        assert_eq!(lines, vec![0, 1, 4, 5, 6, 7], "the rewritten d line stays on two rows");
    }

    fn row_of(app: &App) -> Option<Row> {
        app.open.as_ref().unwrap().row().cloned()
    }

    #[test]
    fn big_d_sets_the_old_file_beside_the_new_and_back_keeping_the_cursor_and_saving_the_choice() {
        let mut app = with_sum_review();
        walk_to(&mut app, |r| matches!(r, Row::Line { index: 6, .. }));
        let actions = press(&mut app, "D");
        assert!(matches!(actions.as_slice(), [Action::SaveState { side_by_side: true, .. }]), "{actions:?}");
        assert_eq!(app.live_toast().map(|t| t.text.as_str()), Some("side by side"));
        assert!(matches!(row_of(&app), Some(Row::Pair { removed: 5, added: 6, .. })), "the rewritten d line sits beside its old text");
        let actions = press(&mut app, "D");
        assert!(matches!(actions.as_slice(), [Action::SaveState { side_by_side: false, .. }]));
        assert!(matches!(row_of(&app), Some(Row::Line { index: 5, .. })), "{:?}", row_of(&app));
    }

    fn peeked(app: &App) -> Vec<usize> {
        app.open.as_ref().unwrap().rows.iter().filter_map(|r| if let Row::Line { index, .. } = r { Some(*index) } else { None }).collect()
    }

    #[test]
    fn angle_brackets_cycle_the_diff_through_after_and_before_each_the_other_way() {
        let mut app = with_sum_review();
        let diff = app.open.as_ref().unwrap().rows.clone();
        press(&mut app, ">");
        assert_eq!((peeked(&app), app.live_toast().map(|t| t.text.clone())), (vec![0, 1, 3, 4, 6, 7], Some("after".to_owned())));
        press(&mut app, ">");
        assert_eq!((peeked(&app), app.live_toast().map(|t| t.text.clone())), (vec![0, 1, 2, 4, 5, 7], Some("before".to_owned())));
        press(&mut app, ">");
        assert_eq!(app.open.as_ref().unwrap().rows, diff, "back to the diff");
        press(&mut app, "<");
        assert_eq!(peeked(&app), vec![0, 1, 2, 4, 5, 7], "< goes to before first");
        press(&mut app, "<");
        assert_eq!(peeked(&app), vec![0, 1, 3, 4, 6, 7]);
    }

    #[test]
    fn a_peek_keeps_the_cursor_on_its_line_or_on_its_hunk_when_the_line_hides() {
        let mut app = with_sum_review();
        walk_to(&mut app, |r| matches!(r, Row::Line { index: 6, .. }));
        press(&mut app, ">");
        assert!(matches!(row_of(&app), Some(Row::Line { index: 6, .. })), "{:?}", row_of(&app));
        press(&mut app, ">");
        assert!(matches!(row_of(&app), Some(Row::Hunk { index: 0, .. })), "{:?}", row_of(&app));
    }

    #[test]
    fn a_refresh_keeps_the_side_by_side_choice() {
        let mut app = with_sum_review();
        press(&mut app, "D");
        app.apply(Incoming::Review { key: mr_key(), review: Box::new(sum_review()), cached: None });
        assert!(app.open.as_ref().unwrap().review.shows_side_by_side());
    }

    #[test]
    fn side_by_side_falls_back_to_inline_in_a_narrow_diff_saying_so_once() {
        let mut app = with_sum_review();
        press(&mut app, "D");
        app.fit_diff(false);
        let open = app.open.as_ref().unwrap();
        assert!(open.review.side_by_side && !open.rows.iter().any(|r| matches!(r, Row::Pair { removed: 5, .. })), "inline rows");
        assert_eq!(app.live_toast().map(|t| t.text.as_str()), Some("side by side needs a wider window"));
        app.toast("seen");
        app.fit_diff(false);
        assert_eq!(app.live_toast().map(|t| t.text.as_str()), Some("seen"), "no second toast while it stays narrow");
        app.fit_diff(true);
        assert!(app.open.as_ref().unwrap().rows.iter().any(|r| matches!(r, Row::Pair { removed: 5, .. })), "back side by side");
    }

    #[test]
    fn c_on_a_side_by_side_row_comments_the_new_side_and_big_c_the_old_side() {
        let mut app = with_sum_review();
        press(&mut app, "D");
        walk_to(&mut app, |r| matches!(r, Row::Pair { removed: 5, .. }));
        press(&mut app, "c");
        assert_eq!(app.input_label(), "new thread · sum.rs:5");
        app.handle_key(code(KeyCode::Esc));
        press(&mut app, "C");
        assert_eq!(app.input_label(), "new thread · sum.rs:-5");
    }

    #[test]
    fn c_on_a_pair_comments_the_new_side_and_big_c_the_old_side() {
        let mut app = with_sum_review();
        on_pair(&mut app);
        press(&mut app, "c");
        assert_eq!(app.input_label(), "new thread · sum.rs:3");
        app.handle_key(code(KeyCode::Esc));
        press(&mut app, "C");
        assert_eq!(app.input_label(), "new thread · sum.rs:-3");
        app.handle_key(code(KeyCode::Esc));
        press(&mut app, "k");
        press(&mut app, "C");
        assert_eq!(app.input_label(), "", "C only means something on a pair");
    }

    #[test]
    fn v_treats_a_pair_as_its_added_line() {
        let mut app = with_sum_review();
        walk_to(&mut app, |r| matches!(r, Row::Line { index: 1, .. }));
        press(&mut app, "Vjjc");
        let Some(Input::Comment { position }) = &app.input else { panic!("no comment input") };
        let start = position.start.expect("a range");
        assert_eq!((start.new, position.line.new), (Some(2), Some(4)), "from line 2 through the pair to line 4");
    }

    #[test]
    fn moving_and_jumping_step_over_pair_rows_like_lines() {
        let mut app = with_sum_review();
        on_pair(&mut app);
        press(&mut app, "j");
        assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::Line { .. })), "no thread row after the pair");
        press(&mut app, "kk");
        assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::Line { index: 1, .. })));
        press(&mut app, "[c");
        assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::Hunk { .. })));
    }

    #[test]
    fn big_w_hides_whitespace_only_changes_and_back() {
        let mut app = with_review();
        press(&mut app, "W");
        assert!(app.open.as_ref().unwrap().review.quiet_whitespace);
        assert!(app.live_toast().unwrap().text.contains("hidden"));
        press(&mut app, "W");
        assert!(!app.open.as_ref().unwrap().review.quiet_whitespace);
    }

    #[test]
    fn equals_loads_the_file_once_and_shows_ten_more_lines_around_the_hunk() {
        let mut app = with_review();
        press(&mut app, "]cj");
        let actions = press(&mut app, "=");
        let open = app.open.clone().unwrap();
        let path = open.review.files[0].new_path.clone();
        assert!(
            matches!(actions.as_slice(), [Action::LoadFile { path: p, sha, .. }] if *p == path && *sha == open.review.mr.refs.head),
            "{actions:?}"
        );
        let text: String = (1..=40).map(|n| format!("line {n}\n")).collect();
        let contexts = |app: &App| app.open.as_ref().unwrap().rows.iter().filter(|r| matches!(r, Row::Context { .. })).count();
        app.apply(Incoming::File { key: mr_key(), path: path.clone(), sha: "old".into(), text: text.clone() });
        assert_eq!(contexts(&app), 0, "a file read at another head is dropped");
        app.apply(Incoming::File { key: mr_key(), path: path.clone(), sha: "bbbb".into(), text });
        assert_eq!(contexts(&app), 20, "ten above, ten below");
        assert_eq!(press(&mut app, "="), vec![], "the file is read once");
        let screen = render(&mut app, 120, 40);
        assert!(screen.contains("line 2"), "{screen}");
        let pushed = Mr { refs: crate::forge::Refs { head: "cccc".into(), ..mr().refs }, ..mr() };
        app.apply(Incoming::Review { key: mr_key(), review: Box::new(Review::new(pushed, &diffs(), discussions(), &[])), cached: None });
        assert_eq!(contexts(&app), 0, "after a push the lines around hunks are read again");
    }
}
