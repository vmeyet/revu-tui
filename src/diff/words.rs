//! Intra-line highlighting: a run of removed lines followed by an equal run of added lines pairs up
//! line by line, the rule GitHub and delta use. Unequal runs stay plain.
//! `similar` tokenises on whitespace, so `Client::new();` is one word and a changed word never drags its space along.
use super::{Hunk, Line, LineKind};
use similar::{ChangeTag, TextDiff};
use std::ops::Range;

pub fn mark(hunk: &Hunk) -> Hunk {
    let mut lines = hunk.lines.clone();
    for (removed, added) in pairs(&hunk.lines) {
        for (r, a) in removed.zip(added) {
            let (old_words, new_words) = changed_ranges(&hunk.lines[r].text, &hunk.lines[a].text);
            lines[r].words = old_words;
            lines[a].words = new_words;
        }
    }
    Hunk { lines, ..hunk.clone() }
}

/// Index ranges of each `-` run and the `+` run right after it, kept only when both are as long.
fn pairs(lines: &[Line]) -> Vec<(Range<usize>, Range<usize>)> {
    let mut found = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let removed = run(lines, i, LineKind::Removed);
        let added = run(lines, removed.end, LineKind::Added);
        if !removed.is_empty() && removed.len() == added.len() {
            found.push((removed.clone(), added.clone()));
        }
        i = added.end.max(removed.end).max(i + 1);
    }
    found
}

fn run(lines: &[Line], from: usize, kind: LineKind) -> Range<usize> {
    let end = lines[from..].iter().position(|l| l.kind != kind).map_or(lines.len(), |n| from + n);
    from..end
}

fn changed_ranges(old: &str, new: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let diff = TextDiff::from_words(old, new);
    let (mut old_at, mut new_at) = (0, 0);
    let (mut old_words, mut new_words) = (Vec::new(), Vec::new());
    for change in diff.iter_all_changes() {
        let len = change.value().len();
        match change.tag() {
            ChangeTag::Equal => {
                old_at += len;
                new_at += len;
            }
            ChangeTag::Delete => {
                push_merged(&mut old_words, old_at..old_at + len);
                old_at += len;
            }
            ChangeTag::Insert => {
                push_merged(&mut new_words, new_at..new_at + len);
                new_at += len;
            }
        }
    }
    (old_words, new_words)
}

fn push_merged(ranges: &mut Vec<Range<usize>>, range: Range<usize>) {
    match ranges.last_mut() {
        Some(last) if last.end == range.start => last.end = range.end,
        _ => ranges.push(range),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::diff::parse;

    fn words_of(line: &Line) -> Vec<&str> {
        line.words.iter().map(|r| &line.text[r.clone()]).collect()
    }

    #[test]
    fn a_single_pair_marks_only_what_changed() {
        let hunk = mark(&parse(include_str!("fixtures/two_hunks.diff"))[0]);
        assert_eq!(words_of(&hunk.lines[1]), vec!["Client::new();"]);
        assert_eq!(words_of(&hunk.lines[2]), vec!["Client::with_key(key);"]);
        assert!(hunk.lines[0].words.is_empty() && hunk.lines[3].words.is_empty());
    }

    #[test]
    fn unequal_runs_get_no_ranges() {
        let hunk = mark(&parse(include_str!("fixtures/tabs.diff"))[0]);
        assert!(hunk.lines.iter().all(|l| l.words.is_empty()), "one removed, two added");
    }

    #[test]
    fn equal_runs_pair_line_by_line() {
        let hunk = mark(&parse("@@ -1,2 +1,2 @@\n-a b c\n-x y z\n+a B c\n+x y Z\n")[0]);
        assert_eq!(words_of(&hunk.lines[0]), vec!["b"]);
        assert_eq!(words_of(&hunk.lines[2]), vec!["B"]);
        assert_eq!(words_of(&hunk.lines[1]), vec!["z"]);
        assert_eq!(words_of(&hunk.lines[3]), vec!["Z"]);
    }

    #[test]
    fn adjacent_changes_merge_into_one_range() {
        let (old, new) = changed_ranges("let a = 1;", "let bb = 22;");
        assert_eq!(old.len(), 2, "{old:?}");
        assert_eq!(new.len(), 2, "{new:?}");
        assert_eq!(&"let bb = 22;"[new[0].clone()], "bb");
    }

    #[test]
    fn identical_lines_have_no_ranges_and_the_source_is_untouched() {
        let source = parse("@@ -1 +1 @@\n-same\n+same\n");
        let marked = mark(&source[0]);
        assert!(marked.lines.iter().all(|l| l.words.is_empty()));
        assert!(source[0].lines.iter().all(|l| l.words.is_empty()));
    }
}
