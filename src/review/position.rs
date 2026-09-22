//! The `position` payload GitLab wants with a diff note, built from where the cursor is.
use super::Review;
use crate::api::types::{LineCode, LineRange, Position};
use crate::diff::{Line, LineKind};

/// `sha1(path)_old_new`, GitLab's name for one diff line; a missing side is `0`.
pub fn line_code(path: &str, old: Option<u32>, new: Option<u32>) -> String {
    let digest = sha1_smol::Sha1::from(path.as_bytes()).digest().to_string();
    format!("{digest}_{}_{}", old.unwrap_or(0), new.unwrap_or(0))
}

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
    let path = |line: &Line| if line.kind == LineKind::Removed { &target.old_path } else { &target.new_path };
    let range = LineRange { start: code_of(path(first), first), end: code_of(path(last), last) };
    Some(position(review, start.0, last, Some(range)))
}

fn position(review: &Review, file: usize, end: &Line, line_range: Option<LineRange>) -> Position {
    let target = &review.files[file];
    let refs = &review.mr.diff_refs;
    Position {
        base_sha: refs.base_sha.clone(),
        head_sha: refs.head_sha.clone(),
        start_sha: refs.start_sha.clone(),
        position_type: "text".into(),
        old_path: Some(target.old_path.clone()),
        new_path: Some(target.new_path.clone()),
        old_line: end.old.filter(|_| end.kind != LineKind::Added),
        new_line: end.new.filter(|_| end.kind != LineKind::Removed),
        line_range,
    }
}

fn code_of(path: &str, line: &Line) -> LineCode {
    let kind = match line.kind {
        LineKind::Added => Some("new".into()),
        LineKind::Removed => Some("old".into()),
        LineKind::Context => None,
    };
    LineCode { line_code: Some(line_code(path, line.old, line.new)), kind, old_line: line.old, new_line: line.new }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::review::tests::review;

    fn sha(path: &str) -> String {
        sha1_smol::Sha1::from(path.as_bytes()).digest().to_string()
    }

    #[test]
    fn line_code_hashes_the_path_and_zeroes_the_missing_side() {
        assert_eq!(sha("abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(line_code("abc", None, Some(13)), "a9993e364706816aba3e25717850c26c9cd0d89d_0_13");
        assert_eq!(line_code("abc", Some(13), None), "a9993e364706816aba3e25717850c26c9cd0d89d_13_0");
        assert_eq!(line_code("abc", Some(12), Some(12)), "a9993e364706816aba3e25717850c26c9cd0d89d_12_12");
    }

    #[test]
    fn one_line_positions_follow_the_line_kind() {
        let review = review();
        let cases = [
            ((0, 0, 0), Some(12), Some(12), "context carries both"),
            ((0, 0, 1), Some(13), None, "removed carries old only"),
            ((0, 0, 2), None, Some(13), "added carries new only"),
            ((0, 1, 3), None, Some(43), "second hunk, added"),
        ];
        for ((file, hunk, line), old, new, why) in cases {
            let position = for_line(&review, file, hunk, line).unwrap();
            assert_eq!((position.old_line, position.new_line), (old, new), "{why}");
            assert_eq!(position.line_range, None, "{why}");
            assert_eq!(position.position_type, "text");
            assert_eq!((position.base_sha.as_str(), position.head_sha.as_str(), position.start_sha.as_str()), ("aaaa", "bbbb", "aaaa"));
            assert_eq!(position.new_path.as_deref(), Some("src/pay/charge.rs"));
            assert_eq!(position.old_path.as_deref(), Some("src/pay/charge.rs"));
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
        assert_eq!((position.old_line, position.new_line), (None, Some(14)), "the end line is an added one");
        let range = position.line_range.unwrap();
        let path = sha("src/pay/charge.rs");
        assert_eq!(
            range.start,
            LineCode { line_code: Some(format!("{path}_13_0")), kind: Some("old".into()), old_line: Some(13), new_line: None }
        );
        assert_eq!(
            range.end,
            LineCode { line_code: Some(format!("{path}_0_14")), kind: Some("new".into()), old_line: None, new_line: Some(14) }
        );
    }

    #[test]
    fn a_range_may_cross_hunks_but_not_files() {
        let review = review();
        let across = for_range(&review, (0, 0, 4), (0, 1, 0)).unwrap();
        let range = across.line_range.unwrap();
        assert_eq!(range.start.kind, None, "context start");
        assert_eq!((range.start.old_line, range.start.new_line), (Some(14), Some(15)));
        assert_eq!((range.end.old_line, range.end.new_line), (Some(40), Some(41)));
        assert_eq!((across.old_line, across.new_line), (Some(40), Some(41)));
        assert_eq!(for_range(&review, (0, 0, 0), (1, 0, 0)), None);
    }

    #[test]
    fn a_one_line_range_is_a_range_all_the_same() {
        let review = review();
        let position = for_range(&review, (0, 0, 2), (0, 0, 2)).unwrap();
        let range = position.line_range.unwrap();
        assert_eq!(range.start, range.end);
        assert_eq!(range.end.kind.as_deref(), Some("new"));
    }
}
