//! The rows of the review pane in reading order, after folds, pairs, peeks and context.
use super::{File, Markers, Place, Review, Side};
use crate::diff::words;
use crate::diff::{self, Hunk, LineKind};

/// One row of the review pane, in reading order, after folds. Indexes point into `Review`, so a row is tiny.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Row {
    Header,
    File {
        index: usize,
        open: bool,
    },
    Hunk {
        file: usize,
        index: usize,
        open: bool,
    },
    Line {
        file: usize,
        hunk: usize,
        index: usize,
    },
    /// An unchanged line outside the hunk, read from the whole file: `new` is its line on the new side.
    Context {
        file: usize,
        hunk: usize,
        old: u32,
        new: u32,
    },
    /// A removed line and its added twin read as one row, the changed words side by side.
    Pair {
        file: usize,
        hunk: usize,
        removed: usize,
        added: usize,
    },
    /// A conversation under its line, where threads read in the diff: `new` and `old` number the line, `stop` counts
    /// the cursor stops of its conversations.
    Thread {
        file: usize,
        hunk: usize,
        new: Option<u32>,
        old: Option<u32>,
        stop: usize,
    },
    Gap,
}

impl Row {
    /// The file a row belongs to; the header and gaps belong to none.
    pub(crate) fn file(&self) -> Option<usize> {
        match self {
            Row::File { index, .. } => Some(*index),
            Row::Hunk { file, .. }
            | Row::Line { file, .. }
            | Row::Pair { file, .. }
            | Row::Context { file, .. }
            | Row::Thread { file, .. } => Some(*file),
            Row::Header | Row::Gap => None,
        }
    }

    /// The file and hunk a row sits in, a hunk row its own.
    pub(crate) fn hunk(&self) -> Option<(usize, usize)> {
        match self {
            Row::Hunk { file, index: hunk, .. }
            | Row::Line { file, hunk, .. }
            | Row::Pair { file, hunk, .. }
            | Row::Context { file, hunk, .. }
            | Row::Thread { file, hunk, .. } => Some((*file, *hunk)),
            Row::Header | Row::File { .. } | Row::Gap => None,
        }
    }
}

impl Review {
    pub(crate) fn shows_line(&self, line: &diff::Line) -> bool {
        match self.peek {
            None => true,
            Some(Side::Old) => line.kind != LineKind::Added,
            Some(Side::New) => line.kind != LineKind::Removed,
        }
    }

    /// The removed and added lines of `hunk` read as one row; none while a peek shows one side.
    fn pairs(&self, hunk: &Hunk) -> Vec<(usize, usize)> {
        if self.peek.is_some() {
            return vec![];
        }
        let mut pairs = if self.shows_side_by_side() { words::side_by_side_pairs(hunk) } else { words::inline_pairs(hunk, self.inline) };
        if self.quiet_whitespace {
            let quiet: Vec<(usize, usize)> = words::whitespace_pairs(hunk).into_iter().filter(|pair| !pairs.contains(pair)).collect();
            pairs.extend(quiet);
        }
        pairs
    }

    pub(crate) fn rows(&self) -> Vec<Row> {
        let rows = self.code_rows();
        if !self.reads_threads_in_diff() {
            return rows;
        }
        let markers = self.markers();
        let with_threads = |row: Row| {
            let threads = self.thread_rows(&markers, &row);
            std::iter::once(row).chain(threads)
        };
        rows.into_iter().flat_map(with_threads).collect()
    }

    /// The conversations of a marked line, a row for each of their cursor stops.
    fn thread_rows(&self, markers: &Markers, row: &Row) -> Vec<Row> {
        let (Some((file, hunk)), Some(_)) = (row.hunk(), self.marker_of(markers, row)) else { return vec![] };
        let Some(place @ Place::Line { new, old, .. }) = self.place_of(row) else { return vec![] };
        let stops = self.stops(&self.conversations(&place)).len();
        (0..stops).map(|stop| Row::Thread { file, hunk, new, old, stop }).collect()
    }

    fn code_rows(&self) -> Vec<Row> {
        let mut rows = vec![Row::Header];
        for (index, file) in self.files.iter().enumerate() {
            rows.push(Row::Gap);
            let open = self.fold.file_is_open(&file.new_path);
            rows.push(Row::File { index, open });
            if open {
                self.push_file_rows(&mut rows, index, file);
            }
        }
        rows
    }

    fn push_file_rows(&self, rows: &mut Vec<Row>, index: usize, file: &File) {
        let text = self.context.texts.get(&file.new_path);
        let mut shown_until = 0;
        for (hunk_index, hunk) in file.hunks.iter().enumerate() {
            let open = self.fold.hunk_is_open(&file.new_path, hunk_index);
            rows.push(Row::Hunk { file: index, index: hunk_index, open });
            if !open {
                continue;
            }
            let extra = self.context.around.get(&(index, hunk_index)).copied().unwrap_or(0);
            if text.is_some() {
                let from = hunk.new_start.saturating_sub(extra).max(shown_until + 1).max(1);
                let offset = i64::from(hunk.old_start) - i64::from(hunk.new_start);
                rows.extend((from..hunk.new_start).map(|new| context_row(index, hunk_index, offset, new)));
            }
            let pairs = self.pairs(hunk);
            for (line_index, line) in hunk.lines.iter().enumerate() {
                if !self.shows_line(line) || pairs.iter().any(|&(_, added)| added == line_index) {
                    continue;
                }
                if let Some(&(removed, added)) = pairs.iter().find(|&&(removed, _)| removed == line_index) {
                    rows.push(Row::Pair { file: index, hunk: hunk_index, removed, added });
                } else {
                    rows.push(Row::Line { file: index, hunk: hunk_index, index: line_index });
                }
            }
            let last_new = hunk.lines.iter().filter_map(|l| l.new).max().unwrap_or(hunk.new_start.saturating_sub(1));
            let last_old = hunk.lines.iter().filter_map(|l| l.old).max().unwrap_or(hunk.old_start.saturating_sub(1));
            shown_until = last_new;
            if let Some(text) = text {
                let next_start = file.hunks.get(hunk_index + 1).map_or(u32::MAX, |h| h.new_start);
                let until = (last_new + extra).min(text.len() as u32).min(next_start.saturating_sub(1));
                let offset = i64::from(last_old) - i64::from(last_new);
                rows.extend((last_new + 1..=until).map(|new| context_row(index, hunk_index, offset, new)));
                shown_until = shown_until.max(until);
            }
        }
    }
}

/// Line `new` of the file; `offset` is how far the old side's numbering sits from the new one's there.
fn context_row(file: usize, hunk: usize, offset: i64, new: u32) -> Row {
    let old = (i64::from(new) + offset).max(0) as u32;
    Row::Context { file, hunk, old, new }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{mr, review};
    use super::super::{Anchor, Context, Draft};
    use super::*;
    use crate::forge::DiffFile;
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::Arc;

    #[test]
    fn rows_follow_folds() {
        let review = review();
        let rows = review.rows();
        assert_eq!(rows[0], Row::Header);
        assert_eq!(rows[2], Row::File { index: 0, open: true });
        assert_eq!(rows[3], Row::Hunk { file: 0, index: 0, open: true });
        let lines = review.files[0].hunks.iter().map(|h| h.lines.len()).sum::<usize>();
        assert_eq!(rows.len(), 1 + 2 + 2 + 2 + lines, "header, gaps, files, hunks and lines only: conversations live in the pane");
        assert_eq!(rows.last(), Some(&Row::File { index: 1, open: false }), "the lock file starts folded");
        assert_eq!(rows.iter().filter(|r| matches!(r, Row::Gap)).count(), 2);
    }

    fn thread_stops(review: &Review) -> Vec<(Option<u32>, Option<u32>, usize)> {
        let stop = |row: &Row| if let Row::Thread { new, old, stop, .. } = row { Some((*new, *old, *stop)) } else { None };
        review.rows().iter().filter_map(stop).collect()
    }

    #[test]
    fn where_threads_read_in_the_diff_a_marked_line_is_followed_by_a_row_per_cursor_stop() {
        let anchor = Anchor { path: "src/pay/charge.rs".into(), side: Side::Old, line: 13 };
        let drafts = vec![Draft::reply("c0ffee00c0ffee00".into(), "agreed"), Draft::new(Some(anchor), "nit")];
        let review = review().with_drafts(drafts).with_threads_in_diff(true);
        let rows = review.rows();
        let line = rows.iter().position(|row| *row == Row::Line { file: 0, hunk: 0, index: 1 }).unwrap();
        let under = |stop| Row::Thread { file: 0, hunk: 0, new: None, old: Some(13), stop };
        assert_eq!(rows[line + 1..line + 4], [under(0), under(1), Row::Line { file: 0, hunk: 0, index: 2 }], "my draft, then the thread");
        assert_eq!(review.place_of(&under(1)), review.place_of(&rows[line]), "a thread row stands for its line");
        let unfolded = review.clone().with_unfolded(BTreeSet::from(["c0ffee00c0ffee00".into()]));
        assert_eq!(thread_stops(&unfolded), [(None, Some(13), 0), (None, Some(13), 1), (None, Some(13), 2)], "its note and my reply");
        let folded_hunk = review.clone().with_fold(review.fold.toggle_hunk("src/pay/charge.rs", 0));
        assert_eq!(thread_stops(&folded_hunk), [], "a folded hunk hides its threads with its lines");
    }

    #[test]
    fn threads_stay_out_of_the_rows_in_the_pane_layouts_and_side_by_side() {
        assert_eq!(thread_stops(&review()), [], "the pane holds them");
        let side_by_side = review().with_threads_in_diff(true).with_side_by_side(true);
        assert!(side_by_side.shows_side_by_side() && thread_stops(&side_by_side).is_empty());
        assert_eq!(thread_stops(&side_by_side.with_wide(false)).len(), 1, "too narrow for two sides: inline, threads under their lines");
    }

    #[test]
    fn a_closed_hunk_keeps_its_row_and_hides_its_lines() {
        let review = review();
        let folded = review.clone().with_fold(review.fold.toggle_hunk("src/pay/charge.rs", 0));
        let rows = folded.rows();
        assert_eq!(rows[3], Row::Hunk { file: 0, index: 0, open: false });
        assert_eq!(rows[4], Row::Hunk { file: 0, index: 1, open: true });
        assert!(rows.contains(&Row::Line { file: 0, hunk: 1, index: 0 }));
    }

    fn review_of_one(diff: &str) -> Review {
        let diff = DiffFile { diff: diff.into(), new_path: "a.rs".into(), old_path: "a.rs".into(), ..DiffFile::default() };
        Review::new(mr(), &[diff], vec![], &[])
    }

    /// The diff rows of the first hunk as `(removed, added)` for pairs and `(index, index)` for lines.
    fn line_rows(review: &Review) -> Vec<(usize, usize)> {
        let pick = |r: &Row| match r {
            Row::Line { index, .. } => Some((*index, *index)),
            Row::Pair { removed, added, .. } => Some((*removed, *added)),
            _ => None,
        };
        review.rows().iter().filter_map(pick).collect()
    }

    #[test]
    fn quiet_whitespace_pairs_a_reindented_line_the_inline_rule_leaves_apart() {
        let review = review_of_one("@@ -1 +1 @@\n-\t\t\tone();\n+            one();\n");
        assert_eq!(line_rows(&review), [(0, 0), (1, 1)], "too much changed to read inline");
        assert_eq!(line_rows(&review.with_quiet_whitespace(true)), [(0, 1)], "W reads them as one row");
    }

    #[test]
    fn side_by_side_sets_each_removed_line_beside_an_added_one_and_the_longer_run_alone() {
        let review = review_of_one("@@ -1,7 +1,6 @@\n a\n-b\n-c\n-d\n+B\n e\n-f\n+F\n+G\n+H\n").with_side_by_side(true);
        assert_eq!(line_rows(&review), [(0, 0), (1, 4), (2, 2), (3, 3), (5, 5), (6, 7), (8, 8), (9, 9)]);
        let folded = review.clone().with_fold(review.fold.toggle_hunk("a.rs", 0));
        assert!(line_rows(&folded).is_empty(), "a folded hunk hides its rows side by side too");
    }

    #[test]
    fn a_peek_keeps_one_side_of_every_hunk_as_plain_lines() {
        let review = review_of_one("@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n").with_side_by_side(true);
        assert_eq!(line_rows(&review.clone().with_peek(Some(Side::Old))), [(0, 0), (1, 1), (3, 3)], "before: no added line, no pair");
        assert_eq!(line_rows(&review.clone().with_peek(Some(Side::New))), [(0, 0), (2, 2), (3, 3)], "after: no removed line");
        assert!(!review.with_peek(Some(Side::New)).shows_side_by_side(), "a peek reads one side, never two");
    }

    #[test]
    fn side_by_side_falls_back_to_the_inline_rows_while_the_diff_is_narrow() {
        let review = review_of_one("@@ -1,2 +1,1 @@\n-let b = 2;\n-gone\n+let b = 20;\n");
        let narrow = review.clone().with_side_by_side(true).with_wide(false);
        assert!(narrow.side_by_side && !narrow.shows_side_by_side(), "the choice stays");
        assert_eq!(line_rows(&narrow), line_rows(&review));
        assert_eq!(line_rows(&narrow.with_wide(true)), [(0, 2), (1, 1)]);
    }

    fn contexts(review: &Review) -> Vec<(usize, u32, u32)> {
        review
            .rows()
            .iter()
            .filter_map(|r| if let Row::Context { hunk, old, new, .. } = r { Some((*hunk, *old, *new)) } else { None })
            .collect()
    }

    #[test]
    fn context_lines_come_from_the_whole_file_numbered_on_both_sides_and_never_twice() {
        let review = review();
        let path = review.files[0].new_path.clone();
        let text: Vec<String> = (1..=60).map(|n| format!("line {n}")).collect();
        let context = Context { texts: BTreeMap::from([(path, Arc::new(text))]), around: BTreeMap::from([((0, 0), 10), ((0, 1), 20)]) };
        let rows = contexts(&review.clone().with_context(context.clone()));
        let above_first: Vec<u32> = rows.iter().filter(|(h, _, n)| *h == 0 && *n < 12).map(|(_, _, n)| *n).collect();
        assert_eq!(above_first, (2..12).collect::<Vec<_>>(), "ten lines above the first hunk");
        assert!(rows.iter().any(|&(h, o, n)| h == 0 && n == 17 && o == 16), "below the first hunk, old numbers follow its offset");
        let second_above: Vec<u32> = rows.iter().filter(|(h, _, n)| *h == 1 && *n < 41).map(|(_, _, n)| *n).collect();
        assert_eq!(second_above.first(), Some(&27), "the second hunk starts after what the first one showed");
        assert!(rows.iter().any(|&(h, o, n)| h == 1 && n == 40 && o == 39), "the second hunk's offset: old is one less");
        assert!(rows.iter().all(|(_, _, n)| *n <= 60), "never past the end of the file");
        let side_by_side = review.with_side_by_side(true).with_context(context);
        assert_eq!(contexts(&side_by_side), rows, "side by side shows the same lines around its hunks");
    }

    #[test]
    fn no_context_is_shown_before_the_file_arrives() {
        let context = Context { texts: BTreeMap::new(), around: BTreeMap::from([((0, 0), 10)]) };
        assert_eq!(contexts(&review().with_context(context)), [] as [(usize, u32, u32); 0]);
    }
}
