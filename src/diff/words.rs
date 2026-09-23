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

/// One run of a changed pair read as a single row: text both lines keep, text only the old one
/// had, text only the new one has. Consecutive runs of one kind are merged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Segment {
    Same(String),
    Old(String),
    New(String),
}

pub fn segments(old: &str, new: &str) -> Vec<Segment> {
    let mut found: Vec<Segment> = Vec::new();
    for change in TextDiff::from_words(old, new).iter_all_changes() {
        let text = change.value();
        match (found.last_mut(), change.tag()) {
            (Some(Segment::Same(run)), ChangeTag::Equal)
            | (Some(Segment::Old(run)), ChangeTag::Delete)
            | (Some(Segment::New(run)), ChangeTag::Insert) => {
                run.push_str(text);
            }
            (_, ChangeTag::Equal) => found.push(Segment::Same(text.to_owned())),
            (_, ChangeTag::Delete) => found.push(Segment::Old(text.to_owned())),
            (_, ChangeTag::Insert) => found.push(Segment::New(text.to_owned())),
        }
    }
    found
}

/// When a changed pair reads better as one row: few changed words, and most of both lines kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InlineRule {
    /// Most changed runs allowed on each side.
    pub max_words: usize,
    /// Least share of each line, in percent of its bytes, the two lines must keep.
    pub min_same: u8,
}

impl Default for InlineRule {
    fn default() -> Self {
        Self { max_words: 2, min_same: 60 }
    }
}

impl InlineRule {
    pub fn fits(self, old: &str, new: &str) -> bool {
        let parts = segments(old, new);
        let olds = parts.iter().filter(|p| matches!(p, Segment::Old(_))).count();
        let news = parts.iter().filter(|p| matches!(p, Segment::New(_))).count();
        let same: usize = parts.iter().map(|p| if let Segment::Same(text) = p { text.len() } else { 0 }).sum();
        let kept = |len: usize| len > 0 && same * 100 >= usize::from(self.min_same) * len;
        olds + news > 0 && olds <= self.max_words && news <= self.max_words && kept(old.len()) && kept(new.len())
    }
}

/// The `(removed, added)` line indexes of the pairs in `hunk` that `rule` lets read as one row.
pub fn inline_pairs(hunk: &Hunk, rule: InlineRule) -> Vec<(usize, usize)> {
    pairs(&hunk.lines)
        .into_iter()
        .flat_map(|(removed, added)| removed.zip(added))
        .filter(|&(r, a)| rule.fits(&hunk.lines[r].text, &hunk.lines[a].text))
        .collect()
}

/// Every equal-run pair whose lines differ in whitespace only.
pub fn whitespace_pairs(hunk: &Hunk) -> Vec<(usize, usize)> {
    pairs(&hunk.lines)
        .into_iter()
        .flat_map(|(removed, added)| removed.zip(added))
        .filter(|&(r, a)| same_but_whitespace(&hunk.lines[r].text, &hunk.lines[a].text))
        .collect()
}

/// Equal once every space, tab and line ending is taken out.
pub fn same_but_whitespace(old: &str, new: &str) -> bool {
    old != new && old.chars().filter(|c| !c.is_whitespace()).eq(new.chars().filter(|c| !c.is_whitespace()))
}

fn changed_ranges(old: &str, new: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let (mut old_at, mut new_at) = (0, 0);
    let (mut old_words, mut new_words) = (Vec::new(), Vec::new());
    for part in segments(old, new) {
        match part {
            Segment::Same(text) => {
                old_at += text.len();
                new_at += text.len();
            }
            Segment::Old(text) => {
                push_merged(&mut old_words, old_at..old_at + text.len());
                old_at += text.len();
            }
            Segment::New(text) => {
                push_merged(&mut new_words, new_at..new_at + text.len());
                new_at += text.len();
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

    #[test]
    fn segments_keep_the_shared_text_between_the_changes() {
        assert_eq!(
            segments("let b = 2;", "let b = 20;"),
            vec![Segment::Same("let b = ".into()), Segment::Old("2;".into()), Segment::New("20;".into())]
        );
    }

    #[test]
    fn the_inline_rule_takes_small_changes_and_leaves_rewrites_split() {
        let rule = InlineRule::default();
        let cases = [
            ("let b = 2;", "let b = 20;", true, "one word"),
            ("let total = price * qty;", "let total = cost * count;", true, "two words, most kept"),
            ("a b c d e f g h", "a x c y e z g h", false, "three changed words"),
            ("Key::from(card.number())", "Key::from((card.number(), card.expiry()))", false, "one token is the whole line"),
            ("let a = 1;", "let a = 1;", false, "nothing changed"),
            ("", "x", false, "an empty side keeps nothing"),
        ];
        for (old, new, fits, why) in cases {
            assert_eq!(rule.fits(old, new), fits, "{why}: {old:?} -> {new:?}");
        }
        assert!(InlineRule { max_words: 3, min_same: 50 }.fits("a b c d e f g h", "a x c y e z g h"), "thresholds come from the rule");
    }

    #[test]
    fn only_equal_runs_that_fit_the_rule_are_inline_pairs() {
        let rule = InlineRule::default();
        let hunk =
            parse("@@ -1,3 +1,3 @@\n-let b = 2;\n-completely old text\n+let b = 20;\n+something else entirely\n let c = 3;\n")[0].clone();
        assert_eq!(inline_pairs(&hunk, rule), vec![(0, 2)]);
        let unequal = parse(include_str!("fixtures/tabs.diff"))[0].clone();
        assert!(inline_pairs(&unequal, rule).is_empty(), "one removed, two added");
    }

    #[test]
    fn whitespace_only_changes_pair_and_real_ones_do_not() {
        assert!(same_but_whitespace("let a = 1;", "let  a = 1; "));
        assert!(same_but_whitespace("\tif x {", "    if x {"));
        assert!(!same_but_whitespace("let a = 1;", "let a = 1;"), "an unchanged line is no whitespace change");
        assert!(!same_but_whitespace("let a = 1;", "let a = 2;"));
        let hunk = &crate::diff::parse("@@ -1,2 +1,2 @@\n-\tone();\n-two();\n+    one();\n+three();\n")[0];
        assert_eq!(whitespace_pairs(hunk), [(0, 2)]);
    }
}
