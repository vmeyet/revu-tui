//! Unified diffs as GitLab sends them: hunks only, no `---`/`+++` header.
pub mod fold;
pub mod words;

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub kind: LineKind,
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub text: String,
    /// Changed byte ranges inside `text`, filled by `words::mark`; empty for context lines.
    pub words: Vec<Range<usize>>,
    pub no_newline: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub header: String,
    pub old_start: u32,
    pub new_start: u32,
    pub lines: Vec<Line>,
}

const NO_NEWLINE: &str = "\\ No newline at end of file";

pub fn parse(unified: &str) -> Vec<Hunk> {
    let mut hunks: Vec<Hunk> = Vec::new();
    for raw in unified.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(hunk) = parse_header(line) {
            hunks.push(hunk);
            continue;
        }
        let Some(hunk) = hunks.last_mut() else { continue };
        if line == NO_NEWLINE {
            if let Some(last) = hunk.lines.last_mut() {
                last.no_newline = true;
            }
            continue;
        }
        if let Some(parsed) = parse_line(hunk, line) {
            hunk.lines.push(parsed);
        }
    }
    hunks
}

/// `@@ -a,b +c,d @@ rest`; the counts are optional (`@@ -1 +1 @@`).
fn parse_header(line: &str) -> Option<Hunk> {
    let rest = line.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(" +")?;
    let (new, _) = rest.split_once(" @@")?;
    Some(Hunk { header: line.to_owned(), old_start: start_of(old)?, new_start: start_of(new)?, lines: vec![] })
}

fn start_of(range: &str) -> Option<u32> {
    range.split(',').next()?.parse().ok()
}

fn parse_line(hunk: &Hunk, line: &str) -> Option<Line> {
    let (old, new) = next_numbers(hunk);
    let (kind, text) = match line.chars().next()? {
        '+' => (LineKind::Added, &line[1..]),
        '-' => (LineKind::Removed, &line[1..]),
        ' ' => (LineKind::Context, &line[1..]),
        _ => return None,
    };
    let (old, new) = match kind {
        LineKind::Context => (Some(old), Some(new)),
        LineKind::Added => (None, Some(new)),
        LineKind::Removed => (Some(old), None),
    };
    Some(Line { kind, old, new, text: text.to_owned(), words: vec![], no_newline: false })
}

/// The numbers the next line takes: one past the last seen on each side, or the hunk start.
fn next_numbers(hunk: &Hunk) -> (u32, u32) {
    let old = hunk.lines.iter().rev().find_map(|l| l.old).map_or(hunk.old_start, |n| n + 1);
    let new = hunk.lines.iter().rev().find_map(|l| l.new).map_or(hunk.new_start, |n| n + 1);
    (old, new)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numbers(hunk: &Hunk) -> Vec<(LineKind, Option<u32>, Option<u32>)> {
        hunk.lines.iter().map(|l| (l.kind, l.old, l.new)).collect()
    }

    #[test]
    fn empty_diff_has_no_hunks() {
        assert!(parse("").is_empty());
        assert!(parse("\n\n").is_empty());
    }

    #[test]
    fn added_file_numbers_only_the_new_side() {
        let hunks = parse(include_str!("fixtures/added.diff"));
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].header, "@@ -0,0 +1,3 @@");
        assert_eq!(
            numbers(&hunks[0]),
            vec![(LineKind::Added, None, Some(1)), (LineKind::Added, None, Some(2)), (LineKind::Added, None, Some(3))]
        );
        assert_eq!(hunks[0].lines[1].text, "    println!(\"hi\");");
    }

    #[test]
    fn deleted_file_numbers_only_the_old_side() {
        let hunks = parse(include_str!("fixtures/deleted.diff"));
        assert_eq!(
            numbers(&hunks[0]),
            vec![(LineKind::Removed, Some(1), None), (LineKind::Removed, Some(2), None), (LineKind::Removed, Some(3), None)]
        );
    }

    #[test]
    fn header_without_counts_starts_at_the_given_line() {
        let hunks = parse(include_str!("fixtures/no_counts.diff"));
        assert_eq!((hunks[0].old_start, hunks[0].new_start), (1, 1));
        assert_eq!(numbers(&hunks[0]), vec![(LineKind::Removed, Some(1), None), (LineKind::Added, None, Some(1))]);
    }

    #[test]
    fn crlf_is_stripped_from_every_line() {
        let hunks = parse(include_str!("fixtures/crlf.diff"));
        assert_eq!(hunks[0].header, "@@ -1,2 +1,2 @@");
        assert_eq!(hunks[0].lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), vec!["context", "old", "new"]);
    }

    #[test]
    fn no_newline_marker_flags_the_previous_line_only() {
        let hunks = parse(include_str!("fixtures/no_newline.diff"));
        let flags: Vec<bool> = hunks[0].lines.iter().map(|l| l.no_newline).collect();
        assert_eq!(flags, vec![false, true, false]);
        assert_eq!(hunks[0].lines.len(), 3, "the marker is not a line");
    }

    #[test]
    fn tabs_and_context_survive_and_numbers_interleave() {
        let hunks = parse(include_str!("fixtures/tabs.diff"));
        assert_eq!(hunks[0].header, "@@ -3,4 +3,5 @@ fn tabs()");
        assert_eq!(hunks[0].lines[0].text, "\tlet a = 1;");
        assert_eq!(
            numbers(&hunks[0]),
            vec![
                (LineKind::Context, Some(3), Some(3)),
                (LineKind::Removed, Some(4), None),
                (LineKind::Added, None, Some(4)),
                (LineKind::Added, None, Some(5)),
                (LineKind::Context, Some(5), Some(6)),
            ]
        );
    }

    #[test]
    fn several_hunks_restart_their_numbering() {
        let hunks = parse(include_str!("fixtures/two_hunks.diff"));
        assert_eq!(hunks.len(), 2);
        assert_eq!((hunks[1].old_start, hunks[1].new_start), (40, 40));
        assert_eq!(
            numbers(&hunks[1]),
            vec![(LineKind::Context, Some(40), Some(40)), (LineKind::Added, None, Some(41)), (LineKind::Context, Some(41), Some(42)),]
        );
    }

    #[test]
    fn text_before_the_first_header_is_ignored() {
        let hunks = parse("garbage\n@@ -1 +1 @@\n a\n");
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].lines.len(), 1);
    }
}
