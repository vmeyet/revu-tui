//! Where a note hangs, built from where the cursor is: one line, or the lines of a `V` selection.
use super::Review;
use crate::diff::{Line, LineKind};
use crate::forge::{LineRef, Position};

/// A note on one line. `None` when the indexes point outside the diff.
pub fn for_line(review: &Review, file: usize, hunk: usize, line: usize) -> Option<Position> {
    let target = review.files.get(file)?;
    let end = target.hunks.get(hunk)?.lines.get(line)?;
    Some(position(review, file, end, None))
}

/// A note on the lines from `start` to `end`, each `(file, hunk, line)`, inside one file.
pub fn for_range(review: &Review, start: (usize, usize, usize), end: (usize, usize, usize)) -> Option<Position> {
    if start.0 != end.0 {
        return None;
    }
    let target = review.files.get(start.0)?;
    let first = target.hunks.get(start.1)?.lines.get(start.2)?;
    let last = target.hunks.get(end.1)?.lines.get(end.2)?;
    Some(position(review, start.0, last, Some(line_ref(first))))
}

fn position(review: &Review, file: usize, end: &Line, start: Option<LineRef>) -> Position {
    let target = &review.files[file];
    Position {
        refs: review.mr.refs.clone(),
        old_path: target.old_path.clone(),
        new_path: target.new_path.clone(),
        line: line_ref(end),
        start,
    }
}

/// The numbers of a diff line on the sides it exists on.
fn line_ref(line: &Line) -> LineRef {
    LineRef { old: line.old.filter(|_| line.kind != LineKind::Added), new: line.new.filter(|_| line.kind != LineKind::Removed) }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::forge::Refs;
    use crate::review::tests::review;

    fn at(old: Option<u32>, new: Option<u32>) -> LineRef {
        LineRef { old, new }
    }

    mod properties {
        #![allow(clippy::unwrap_used, clippy::expect_used)]
        use super::super::*;
        use crate::diff::arbitrary;
        use crate::review::tests::{lines_of, review_of};
        use proptest::prelude::*;

        fn expected(line: &Line) -> LineRef {
            match line.kind {
                LineKind::Context => LineRef { old: line.old, new: line.new },
                LineKind::Added => LineRef { old: None, new: line.new },
                LineKind::Removed => LineRef { old: line.old, new: None },
            }
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(64))]

            #[test]
            fn every_line_gets_the_numbers_of_the_sides_it_lives_on(generated in arbitrary::diff()) {
                let review = review_of(&generated.text());
                for (h, l) in lines_of(&review) {
                    let line = &review.files[0].hunks[h].lines[l];
                    let position = for_line(&review, 0, h, l).unwrap();
                    prop_assert_eq!(position.line, expected(line));
                    prop_assert!(position.line.number().is_some());
                    prop_assert_eq!(position.start, None);
                    prop_assert_eq!((position.old_path.as_str(), position.new_path.as_str()), ("src/old.rs", "src/new.rs"));
                    prop_assert_eq!(&position.refs, &review.mr.refs);
                }
            }

            #[test]
            fn a_range_names_its_first_and_last_line_and_stays_in_one_file(generated in arbitrary::diff(), pick in any::<(prop::sample::Index, prop::sample::Index)>()) {
                let review = review_of(&generated.text());
                let lines = lines_of(&review);
                let (a, b) = (pick.0.index(lines.len()), pick.1.index(lines.len()));
                let (first, last) = (lines[a.min(b)], lines[a.max(b)]);
                let position = for_range(&review, (0, first.0, first.1), (0, last.0, last.1)).unwrap();
                let hunks = &review.files[0].hunks;
                prop_assert_eq!(position.line, expected(&hunks[last.0].lines[last.1]));
                prop_assert_eq!(position.start, Some(expected(&hunks[first.0].lines[first.1])));
                prop_assert_eq!(for_range(&review, (0, first.0, first.1), (1, 0, 0)), None);
            }
        }
    }

    #[test]
    fn one_line_positions_follow_the_line_kind() {
        let review = review();
        let cases = [
            ((0, 0, 0), at(Some(12), Some(12)), "context carries both"),
            ((0, 0, 1), at(Some(13), None), "removed carries old only"),
            ((0, 0, 2), at(None, Some(13)), "added carries new only"),
            ((0, 1, 3), at(None, Some(43)), "second hunk, added"),
        ];
        for ((file, hunk, line), expected, why) in cases {
            let position = for_line(&review, file, hunk, line).unwrap();
            assert_eq!(position.line, expected, "{why}");
            assert_eq!(position.start, None, "{why}");
            assert_eq!(position.refs, Refs { base: "aaaa".into(), start: "aaaa".into(), head: "bbbb".into() });
            assert_eq!((position.old_path.as_str(), position.new_path.as_str()), ("src/pay/charge.rs", "src/pay/charge.rs"));
        }
    }

    #[test]
    fn out_of_range_indexes_give_nothing() {
        let review = review();
        assert_eq!(for_line(&review, 9, 0, 0), None);
        assert_eq!(for_line(&review, 0, 9, 0), None);
        assert_eq!(for_line(&review, 0, 0, 99), None);
        assert_eq!(for_range(&review, (0, 0, 0), (0, 0, 99)), None);
    }

    #[test]
    fn a_range_ends_where_the_selection_ends_and_names_both_edges() {
        let review = review();
        let position = for_range(&review, (0, 0, 1), (0, 0, 3)).unwrap();
        assert_eq!(position.line, at(None, Some(14)), "the end line is an added one");
        assert_eq!(position.start, Some(at(Some(13), None)), "the start line is a removed one");
    }

    #[test]
    fn a_range_may_cross_hunks_but_not_files() {
        let review = review();
        let across = for_range(&review, (0, 0, 4), (0, 1, 0)).unwrap();
        assert_eq!(across.start, Some(at(Some(14), Some(15))), "context start");
        assert_eq!(across.line, at(Some(40), Some(41)));
        assert_eq!(for_range(&review, (0, 0, 0), (1, 0, 0)), None);
    }

    #[test]
    fn a_one_line_range_is_a_range_all_the_same() {
        let review = review();
        let position = for_range(&review, (0, 0, 2), (0, 0, 2)).unwrap();
        assert_eq!(position.start, Some(position.line));
        assert_eq!(position.line, at(None, Some(13)));
    }
}
