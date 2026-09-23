//! Unified diffs as GitLab sends them: hunks only, no `---`/`+++` header.
#[cfg(test)]
pub(crate) mod arbitrary;
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
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn numbers(hunk: &Hunk) -> Vec<(LineKind, Option<u32>, Option<u32>)> {
        hunk.lines.iter().map(|l| (l.kind, l.old, l.new)).collect()
    }

    mod properties {
        #![allow(clippy::unwrap_used, clippy::expect_used)]
        use super::super::arbitrary::{self, Generated};
        use super::super::*;
        use proptest::prelude::*;

        fn numbers_on(hunk: &Hunk, side: impl Fn(&Line) -> Option<u32>) -> Vec<u32> {
            hunk.lines.iter().filter_map(side).collect()
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(256))]

            #[test]
            fn any_text_parses_without_panicking(text in "(@@ -[0-9,]{0,6} \\+[0-9,]{0,6} @@|[ +\\-]|[^\n]{0,12}|\n|\r){0,40}") {
                let _ = parse(&text);
            }

            #[test]
            fn a_generated_diff_parses_back_to_its_parts(generated in arbitrary::diff()) {
                let hunks = parse(&generated.text());
                prop_assert_eq!(hunks.len(), generated.parts.len());
                for (hunk, part) in hunks.iter().zip(&generated.parts) {
                    prop_assert_eq!(&hunk.header, &part.header());
                    prop_assert_eq!((hunk.old_start, hunk.new_start), (part.old_start, part.new_start));
                    let lines: Vec<(LineKind, String)> = hunk.lines.iter().map(|l| (l.kind, l.text.clone())).collect();
                    prop_assert_eq!(&lines, &part.lines);
                }
            }

            #[test]
            fn line_numbers_count_up_from_the_header_and_match_its_counts(generated in arbitrary::diff()) {
                for (hunk, part) in parse(&generated.text()).iter().zip(&generated.parts) {
                    let old = numbers_on(hunk, |l| l.old);
                    let new = numbers_on(hunk, |l| l.new);
                    let expected_old: Vec<u32> = (part.old_start..part.old_start + part.old_count()).collect();
                    let expected_new: Vec<u32> = (part.new_start..part.new_start + part.new_count()).collect();
                    prop_assert_eq!(old, expected_old);
                    prop_assert_eq!(new, expected_new);
                    for line in &hunk.lines {
                        let sides = (line.old.is_some(), line.new.is_some());
                        let expected = match line.kind {
                            LineKind::Context => (true, true),
                            LineKind::Added => (false, true),
                            LineKind::Removed => (true, false),
                        };
                        prop_assert_eq!(sides, expected);
                    }
                }
            }

            #[test]
            fn hunks_never_step_back(generated in arbitrary::diff()) {
                let hunks = parse(&generated.text());
                for pair in hunks.windows(2) {
                    let last_old = pair[0].lines.iter().rev().find_map(|l| l.old).unwrap_or(pair[0].old_start);
                    let last_new = pair[0].lines.iter().rev().find_map(|l| l.new).unwrap_or(pair[0].new_start);
                    prop_assert!(pair[1].old_start >= last_old && pair[1].new_start >= last_new);
                }
            }

            #[test]
            fn printing_parsed_hunks_gives_the_input_back(generated in arbitrary::diff()) {
                let text = generated.text();
                prop_assert_eq!(arbitrary::print(&parse(&text)), text);
            }

            #[test]
            fn crlf_line_ends_read_like_lf(generated in arbitrary::diff()) {
                let text = generated.text();
                let crlf = text.replace('\n', "\r\n");
                prop_assert_eq!(parse(&crlf), parse(&text));
            }
        }

        #[test]
        fn a_generated_diff_is_what_a_forge_sends() {
            let generated = Generated {
                parts: vec![arbitrary::Part {
                    old_start: 3,
                    new_start: 3,
                    terse: true,
                    context: "fn main()".into(),
                    lines: vec![(LineKind::Context, "a".into()), (LineKind::Removed, "b".into()), (LineKind::Added, "c".into())],
                }],
                no_newline: true,
            };
            assert_eq!(generated.text(), "@@ -3,2 +3,2 @@ fn main()\n a\n-b\n+c\n\\ No newline at end of file\n");
        }
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
