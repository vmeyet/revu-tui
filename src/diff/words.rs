//! Intra-line highlighting: `align` finds which removed and added lines are twins, and this marks what changed between them.
//! A line is cut like code, into identifiers, whitespace runs and single punctuation characters, so
//! `Client::new();` to `Client::with_key(key);` marks `new`, `with_key` and `key`, and a changed word never drags its space along.
use super::align::{Change, Comparison, MAX_FILE_WORK, changes, compare_run, pairs_by_position};
use super::{Hunk, Line, LinePair};
use similar::{ChangeTag, TextDiff};
use std::ops::Range;
use unicode_width::UnicodeWidthChar;

/// Glues two emoji into one drawn character.
const JOINER: char = '\u{200D}';
const SKIN_TONES: std::ops::RangeInclusive<char> = '\u{1F3FB}'..='\u{1F3FF}';

/// `hunks`, one file's, with their twins and what changed between them, as far as the file's budget of work goes.
pub(crate) fn mark_file(hunks: &[Hunk]) -> Vec<Hunk> {
    marked_within(hunks, MAX_FILE_WORK)
}

/// `hunks` with their whitespace-only twins alone and no word diff: a file too large to be worth any.
pub(crate) fn mark_whitespace_only(hunks: &[Hunk]) -> Vec<Hunk> {
    marked_within(hunks, 0)
}

fn marked_within(hunks: &[Hunk], work: usize) -> Vec<Hunk> {
    let mut work_left = work;
    let mut marked = Vec::with_capacity(hunks.len());
    for hunk in hunks {
        let (hunk, spent) = mark_hunk(hunk, work_left);
        work_left -= spent;
        marked.push(hunk);
    }
    marked
}

fn mark_hunk(hunk: &Hunk, work: usize) -> (Hunk, usize) {
    let (mut pairs, mut diffed, mut unaligned, mut spent) = (vec![], vec![], vec![], 0);
    let both_sides = changes(&hunk.lines).into_iter().filter(|change| !change.removed.is_empty() && !change.added.is_empty());
    for change in both_sides {
        let run = compare_run(&hunk.lines, &change, work - spent);
        spent += run.work;
        if run.comparison != Comparison::Aligned {
            unaligned.push(change.removed.start);
        }
        if run.comparison != Comparison::WhitespaceOnly {
            diffed.extend_from_slice(&run.pairs);
        }
        pairs.extend(run.pairs);
    }
    let lines = with_changed_words(&hunk.lines, &diffed);
    (
        Hunk {
            header: hunk.header.clone(),
            old_start: hunk.old_start,
            new_start: hunk.new_start,
            lines,
            pairs,
            unaligned,
            label: hunk.label,
        },
        spent,
    )
}

fn with_changed_words(lines: &[Line], pairs: &[LinePair]) -> Vec<Line> {
    let mut marked = lines.to_vec();
    for &LinePair { removed, added } in pairs {
        let (old_words, new_words) = changed_ranges(&lines[removed].text, &lines[added].text);
        marked[removed].words = old_words;
        marked[added].words = new_words;
    }
    marked
}

/// One run of a changed pair read as a single row: text both lines keep, text only the old one
/// had, text only the new one has. Consecutive runs of one kind are merged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Segment {
    Same(String),
    Old(String),
    New(String),
}

/// The runs of a changed pair, read back from the changed ranges `mark_file` stored on both lines.
pub(crate) fn segments(old: &Line, new: &Line) -> Vec<Segment> {
    let mut found = Vec::new();
    let (mut old_at, mut new_at) = (0, 0);
    let mut old_words = old.words.iter().peekable();
    let mut new_words = new.words.iter().peekable();
    loop {
        if let Some(word) = old_words.next_if(|w| w.start == old_at) {
            found.push(Segment::Old(old.text[word.clone()].to_owned()));
            old_at = word.end;
        } else if let Some(word) = new_words.next_if(|w| w.start == new_at) {
            found.push(Segment::New(new.text[word.clone()].to_owned()));
            new_at = word.end;
        } else {
            let old_until = old_words.peek().map_or(old.text.len(), |w| w.start);
            let new_until = new_words.peek().map_or(new.text.len(), |w| w.start);
            let kept = (old_until - old_at).min(new_until - new_at);
            if kept == 0 {
                break;
            }
            found.push(Segment::Same(old.text[old_at..old_at + kept].to_owned()));
            old_at += kept;
            new_at += kept;
        }
    }
    found
}

pub(crate) fn text_segments(old: &str, new: &str) -> Vec<Segment> {
    merge_close_changes(token_runs(old, new))
}

fn token_runs(old: &str, new: &str) -> Vec<Segment> {
    let (old_tokens, new_tokens) = (tokens(old), tokens(new));
    let mut found: Vec<Segment> = Vec::new();
    for change in TextDiff::from_slices(&old_tokens, &new_tokens).iter_all_changes() {
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

/// Cuts `line` into identifier runs, whitespace runs and single punctuation characters.
pub(super) fn tokens(line: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = line;
    while let Some(first) = rest.chars().next() {
        let len = match first {
            c if is_word_char(c) => prefix_len(rest, |c| is_word_char(c) || is_mark(c)),
            c if c.is_whitespace() => prefix_len(rest, char::is_whitespace),
            c => c.len_utf8(),
        };
        let (token, after) = rest.split_at(len + attached_len(&rest[len..]));
        found.push(token);
        rest = after;
    }
    found
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Draws in the cell of the character before it: a combining mark, a variation selector, a joiner, a skin tone.
fn is_mark(c: char) -> bool {
    c.width() == Some(0) || SKIN_TONES.contains(&c)
}

/// How many bytes at the start of `after` belong to the character before it: its marks, and whatever a joiner glues on.
pub(crate) fn attached_len(after: &str) -> usize {
    let mut joined = false;
    for (at, c) in after.char_indices() {
        if !joined && !is_mark(c) {
            return at;
        }
        joined = c == JOINER;
    }
    after.len()
}

fn prefix_len(text: &str, keep: fn(char) -> bool) -> usize {
    text.find(|c| !keep(c)).unwrap_or(text.len())
}

/// Joins two replacements with only a thin gap between them, the gap turning into old and new text.
/// Both must replace text: a change on one side only keeps its gap, so `new()` to `with_key(key)`
/// never marks an unchanged bracket on the old line just to join `with_key` and `key`.
fn merge_close_changes(runs: Vec<Segment>) -> Vec<Segment> {
    let mut found = Vec::new();
    for run in runs {
        found.push(run);
        if let [.., Segment::Old(old), Segment::New(new), Segment::Same(gap), Segment::Old(next_old), Segment::New(next_new)] =
            found.as_mut_slice()
            && is_thin_gap(gap)
        {
            old.extend([gap.as_str(), next_old]);
            new.extend([gap.as_str(), next_new]);
            found.truncate(found.len() - 3);
        }
    }
    found
}

/// Only whitespace, or one punctuation character.
fn is_thin_gap(gap: &str) -> bool {
    matches!(tokens(gap).as_slice(), [token] if !token.starts_with(is_word_char))
}

/// Most changed runs a pair may have on each side and still read as one row.
const INLINE_MAX_WORDS: usize = 2;
/// Least share of each line, in percent of its bytes, a pair must keep to read as one row.
const INLINE_MIN_SAME: usize = 60;

/// Whether a changed pair reads better as one row: few changed words, and most of both lines kept.
/// Read from the changed ranges `mark_file` stored on both lines.
fn fits_one_row(old: &Line, new: &Line) -> bool {
    let (olds, news) = (old.words.len(), new.words.len());
    let same = old.text.len() - old.words.iter().map(ExactSizeIterator::len).sum::<usize>();
    let kept = |len: usize| len > 0 && same * 100 >= INLINE_MIN_SAME * len;
    olds + news > 0 && olds <= INLINE_MAX_WORDS && news <= INLINE_MAX_WORDS && kept(old.text.len()) && kept(new.text.len())
}

/// The pairs `mark_file` found in `hunk` that fit on one row.
pub(crate) fn inline_pairs(hunk: &Hunk) -> Vec<LinePair> {
    pairs_where(hunk, fits_one_row)
}

/// The pairs `inline_pairs` finds, and those whose lines differ in whitespace only, which read as one row under `W`.
pub(crate) fn inline_or_whitespace_pairs(hunk: &Hunk) -> Vec<LinePair> {
    pairs_where(hunk, |old, new| fits_one_row(old, new) || same_but_whitespace(&old.text, &new.text))
}

fn pairs_where(hunk: &Hunk, keep: impl Fn(&Line, &Line) -> bool) -> Vec<LinePair> {
    hunk.pairs.iter().copied().filter(|pair| keep(&hunk.lines[pair.removed], &hunk.lines[pair.added])).collect()
}

/// The rows of a side by side diff holding both sides: the pairs `mark_file` found, and between them the lines
/// without a twin set beside each other by position, the longer side's extra lines left alone.
pub(crate) fn side_by_side_pairs(hunk: &Hunk) -> Vec<LinePair> {
    let mut marked = hunk.pairs.iter().copied().peekable();
    let mut found = Vec::new();
    for Change { removed, added } in changes(&hunk.lines) {
        let (mut old_at, mut new_at) = (removed.start, added.start);
        while let Some(pair) = marked.next_if(|pair| removed.contains(&pair.removed)) {
            found.extend(pairs_by_position(old_at..pair.removed, new_at..pair.added));
            found.push(pair);
            (old_at, new_at) = (pair.removed + 1, pair.added + 1);
        }
        found.extend(pairs_by_position(old_at..removed.end, new_at..added.end));
    }
    found
}

/// Equal once every space, tab and line ending is taken out.
pub(crate) fn same_but_whitespace(old: &str, new: &str) -> bool {
    old != new && old.chars().filter(|c| !c.is_whitespace()).eq(new.chars().filter(|c| !c.is_whitespace()))
}

fn changed_ranges(old: &str, new: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let (mut old_at, mut new_at) = (0, 0);
    let (mut old_words, mut new_words) = (Vec::new(), Vec::new());
    for part in text_segments(old, new) {
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
pub(crate) fn mark(hunk: &Hunk) -> Hunk {
    mark_file(std::slice::from_ref(hunk)).remove(0)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::diff::parse;

    /// Two lines read as twins whatever their similarity, each holding the ranges `mark_file` stores on a pair.
    fn marked_pair(old: &str, new: &str) -> [Line; 2] {
        let hunk = &parse(&format!("@@ -1 +1 @@\n-{old}\n+{new}\n"))[0];
        let (old_words, new_words) = changed_ranges(old, new);
        [Line { words: old_words, ..hunk.lines[0].clone() }, Line { words: new_words, ..hunk.lines[1].clone() }]
    }

    pub(in crate::diff) fn pair(removed: usize, added: usize) -> LinePair {
        LinePair { removed, added }
    }

    pub(in crate::diff) fn words_of(line: &Line) -> Vec<&str> {
        line.words.iter().map(|r| &line.text[r.clone()]).collect()
    }

    mod properties {
        use super::super::*;
        use super::marked_pair;
        use crate::diff::{LineKind, arbitrary, parse};
        use proptest::prelude::*;

        fn line() -> impl Strategy<Value = String> {
            "[a-z0-9 \t(){};=.,éü🦀]{0,40}"
        }

        fn alike_hunk() -> impl Strategy<Value = Hunk> {
            arbitrary::alike_diff().prop_map(|text| parse(&text).remove(0))
        }

        fn joined(parts: &[Segment], keep: impl Fn(&Segment) -> Option<&str>) -> String {
            parts.iter().filter_map(keep).collect()
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(256))]

            #[test]
            fn segments_read_from_the_marked_ranges_match_a_fresh_word_diff(old in line(), new in line()) {
                let [old_line, new_line] = marked_pair(&old, &new);
                prop_assert_eq!(segments(&old_line, &new_line), text_segments(&old, &new));
            }

            #[test]
            fn segments_rebuild_both_lines(old in line(), new in line()) {
                let parts = text_segments(&old, &new);
                let old_side = joined(&parts, |p| match p { Segment::Same(t) | Segment::Old(t) => Some(t.as_str()), Segment::New(_) => None });
                let new_side = joined(&parts, |p| match p { Segment::Same(t) | Segment::New(t) => Some(t.as_str()), Segment::Old(_) => None });
                prop_assert_eq!(old_side, old);
                prop_assert_eq!(new_side, new);
            }

            #[test]
            fn no_two_neighbours_are_the_same_kind(old in line(), new in line()) {
                let parts = text_segments(&old, &new);
                for pair in parts.windows(2) {
                    prop_assert_ne!(std::mem::discriminant(&pair[0]), std::mem::discriminant(&pair[1]));
                }
            }

            #[test]
            fn no_thin_gap_is_left_between_two_replacements(old in line(), new in line()) {
                let parts = text_segments(&old, &new);
                for window in parts.windows(5) {
                    if let [Segment::Old(_), Segment::New(_), Segment::Same(gap), Segment::Old(_), Segment::New(_)] = window {
                        prop_assert!(!is_thin_gap(gap), "{gap:?} in {parts:?}");
                    }
                }
            }

            #[test]
            fn changed_ranges_sit_on_character_boundaries_in_order(old in line(), new in line()) {
                let (old_words, new_words) = changed_ranges(&old, &new);
                for (text, ranges) in [(&old, &old_words), (&new, &new_words)] {
                    for range in ranges {
                        prop_assert!(range.start < range.end && range.end <= text.len());
                        prop_assert!(text.is_char_boundary(range.start) && text.is_char_boundary(range.end));
                    }
                    for pair in ranges.windows(2) {
                        prop_assert!(pair[0].end < pair[1].start, "merged ranges never touch");
                    }
                }
            }

            #[test]
            fn marking_a_hunk_keeps_its_lines_and_marks_only_changed_ones(generated in arbitrary::diff()) {
                for hunk in parse(&generated.text()) {
                    let marked = mark(&hunk);
                    prop_assert_eq!(marked.lines.len(), hunk.lines.len());
                    for (before, after) in hunk.lines.iter().zip(&marked.lines) {
                        prop_assert_eq!((&before.text, before.kind), (&after.text, after.kind));
                        if after.kind == LineKind::Context {
                            prop_assert!(after.words.is_empty());
                        }
                        for range in &after.words {
                            prop_assert!(range.end <= after.text.len() && after.text.is_char_boundary(range.start));
                        }
                    }
                }
            }

            #[test]
            fn an_inline_pair_is_always_a_removed_line_then_its_added_twin(hunk in alike_hunk()) {
                let hunk = mark(&hunk);
                for LinePair { removed, added } in inline_pairs(&hunk) {
                    prop_assert_eq!(hunk.lines[removed].kind, LineKind::Removed);
                    prop_assert_eq!(hunk.lines[added].kind, LineKind::Added);
                    prop_assert!(removed < added);
                }
            }

            #[test]
            fn every_marked_pair_reads_back_as_a_fresh_word_diff(hunk in alike_hunk()) {
                let hunk = mark(&hunk);
                for LinePair { removed, added } in &hunk.pairs {
                    let (old, new) = (&hunk.lines[*removed], &hunk.lines[*added]);
                    prop_assert_eq!(segments(old, new), text_segments(&old.text, &new.text));
                }
            }

            #[test]
            fn only_a_paired_line_holds_changed_ranges(hunk in alike_hunk()) {
                let hunk = mark(&hunk);
                for (index, line) in hunk.lines.iter().enumerate() {
                    if !line.words.is_empty() {
                        prop_assert!(hunk.pairs.iter().any(|pair| pair.removed == index || pair.added == index));
                    }
                }
            }

            #[test]
            fn marked_pairs_never_cross_and_stay_inside_one_change(hunk in alike_hunk()) {
                let hunk = mark(&hunk);
                for neighbours in hunk.pairs.windows(2) {
                    prop_assert!(neighbours[0].removed < neighbours[1].removed && neighbours[0].added < neighbours[1].added);
                }
                for LinePair { removed, added } in &hunk.pairs {
                    prop_assert!(hunk.lines[*removed..*added].iter().all(|line| line.kind != LineKind::Context));
                }
            }

            #[test]
            fn side_by_side_pairs_never_cross_and_hold_every_marked_pair(hunk in alike_hunk()) {
                let hunk = mark(&hunk);
                let rows = side_by_side_pairs(&hunk);
                for neighbours in rows.windows(2) {
                    prop_assert!(neighbours[0].removed < neighbours[1].removed && neighbours[0].added < neighbours[1].added);
                }
                prop_assert!(hunk.pairs.iter().all(|pair| rows.contains(pair)));
            }
        }
    }

    #[test]
    fn a_single_pair_marks_only_what_changed() {
        let hunk = mark(&parse(include_str!("fixtures/two_hunks.diff"))[0]);
        assert_eq!(words_of(&hunk.lines[1]), vec!["new"]);
        assert_eq!(words_of(&hunk.lines[2]), vec!["with_key", "key"]);
        assert!(hunk.lines[0].words.is_empty() && hunk.lines[3].words.is_empty());
    }

    #[test]
    fn runs_of_alike_lines_pair_each_line_with_its_twin() {
        let hunk = mark(&parse("@@ -1,2 +1,2 @@\n-a b c d\n-w x y z\n+a B c d\n+w x y Z\n")[0]);
        assert_eq!(hunk.pairs, [pair(0, 2), pair(1, 3)]);
        assert_eq!(words_of(&hunk.lines[0]), vec!["b"]);
        assert_eq!(words_of(&hunk.lines[2]), vec!["B"]);
        assert_eq!(words_of(&hunk.lines[1]), vec!["z"]);
        assert_eq!(words_of(&hunk.lines[3]), vec!["Z"]);
    }

    /// `-` and `+` lines for each text of `old` and `new`, in a single change.
    pub(in crate::diff) fn change_of(old: &[impl AsRef<str>], new: &[impl AsRef<str>]) -> Hunk {
        let removed = old.iter().map(|text| format!("-{}\n", text.as_ref()));
        let added = new.iter().map(|text| format!("+{}\n", text.as_ref()));
        let body: String = removed.chain(added).collect();
        mark(&parse(&format!("@@ -1 +1 @@\n{body}"))[0])
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
        let [old, new] = marked_pair("let b = 2;", "let b = 20;");
        assert_eq!(
            segments(&old, &new),
            vec![Segment::Same("let b = ".into()), Segment::Old("2".into()), Segment::New("20".into()), Segment::Same(";".into())]
        );
    }

    fn same(text: &str) -> Segment {
        Segment::Same(text.into())
    }

    fn old(text: &str) -> Segment {
        Segment::Old(text.into())
    }

    fn new(text: &str) -> Segment {
        Segment::New(text.into())
    }

    #[test]
    fn a_line_cuts_into_identifiers_whitespace_runs_and_single_punctuation() {
        assert_eq!(tokens("é_1  🦀::x"), ["é_1", "  ", "🦀", ":", ":", "x"]);
    }

    #[test]
    fn a_combining_mark_stays_in_the_word_before_it() {
        assert_eq!(tokens("cafe\u{301} au"), ["cafe\u{301}", " ", "au"]);
    }

    #[test]
    fn a_combining_mark_inside_a_word_does_not_cut_it() {
        assert_eq!(tokens("re\u{301}sume\u{301};"), ["re\u{301}sume\u{301}", ";"]);
    }

    #[test]
    fn a_variation_selector_stays_with_the_character_before_it() {
        assert_eq!(tokens("\u{2764}\u{FE0F}!"), ["\u{2764}\u{FE0F}", "!"]);
    }

    #[test]
    fn a_joiner_keeps_an_emoji_sequence_in_one_token() {
        assert_eq!(tokens("\u{1F469}\u{200D}\u{1F4BB};"), ["\u{1F469}\u{200D}\u{1F4BB}", ";"]);
    }

    #[test]
    fn a_skin_tone_stays_with_the_emoji_before_it() {
        assert_eq!(tokens("\u{1F44D}\u{1F3FD};"), ["\u{1F44D}\u{1F3FD}", ";"]);
    }

    #[test]
    fn a_changed_emoji_sequence_marks_all_of_it() {
        let (old, new) = ("a \u{1F469}\u{200D}\u{1F4BB}", "a \u{1F469}\u{200D}\u{1F527}");
        assert_eq!(text_segments(old, new), [same("a "), self::old("\u{1F469}\u{200D}\u{1F4BB}"), self::new("\u{1F469}\u{200D}\u{1F527}")]);
    }

    #[test]
    fn replacements_with_only_whitespace_between_them_merge() {
        assert_eq!(text_segments("a foo  bar;", "a baz  qux;"), [same("a "), old("foo  bar"), new("baz  qux"), same(";")]);
    }

    #[test]
    fn replacements_with_one_punctuation_character_between_them_merge() {
        assert_eq!(text_segments("a.b", "x.y"), [old("a.b"), new("x.y")]);
    }

    #[test]
    fn a_chain_of_close_replacements_merges_into_one() {
        assert_eq!(text_segments("a b.c", "x y.z"), [old("a b.c"), new("x y.z")]);
    }

    #[test]
    fn a_comma_and_its_space_keep_two_replacements_apart() {
        assert_eq!(text_segments("f(a, b)", "f(x, y)"), [same("f("), old("a"), new("x"), same(", "), old("b"), new("y"), same(")")]);
    }

    #[test]
    fn an_identifier_keeps_two_replacements_apart() {
        assert_eq!(text_segments("(x)", "[x]"), [old("("), new("["), same("x"), old(")"), new("]")]);
    }

    #[test]
    fn a_change_on_one_side_keeps_its_gap_unmarked() {
        assert_eq!(text_segments("new()", "with_key(key)"), [old("new"), new("with_key"), same("("), new("key"), same(")")]);
    }

    #[test]
    fn the_inline_rule_takes_small_changes_and_leaves_rewrites_split() {
        let cases = [
            ("let b = 2;", "let b = 20;", true, "one word"),
            ("let total = price * qty;", "let total = cost * count;", true, "two words, most kept"),
            ("a b c d e f g h", "a x c y e z g h", false, "three changed words"),
            ("Key::from(card.number())", "Key::from((card.number(), card.expiry()))", false, "too much of the new line is added"),
            ("old words here", "new terms there", false, "a merged rewrite keeps nothing"),
            ("let a = 1;", "let a = 1;", false, "nothing changed"),
            ("", "x", false, "an empty side keeps nothing"),
        ];
        for (old, new, fits, why) in cases {
            let [old_line, new_line] = marked_pair(old, new);
            assert_eq!(fits_one_row(&old_line, &new_line), fits, "{why}: {old:?} -> {new:?}");
        }
    }

    #[test]
    fn only_twins_that_fit_the_rule_are_inline_pairs() {
        let hunk =
            mark(&parse("@@ -1,3 +1,3 @@\n-let b = 2;\n-completely old text\n+let b = 20;\n+something else entirely\n let c = 3;\n")[0]);
        assert_eq!(inline_pairs(&hunk), vec![pair(0, 2)]);
    }

    #[test]
    fn side_by_side_sets_lines_without_a_twin_beside_each_other_by_position() {
        let hunk = mark(&parse("@@ -1,7 +1,6 @@\n a\n-b\n-c\n-d\n+B\n e\n-f\n+F\n+G\n+H\n")[0]);
        assert_eq!(hunk.pairs, []);
        assert_eq!(side_by_side_pairs(&hunk), [pair(1, 4), pair(6, 7)], "three removed beside one added, one removed beside three added");
        let added_only = mark(&parse("@@ -1 +1,2 @@\n a\n+b\n")[0]);
        assert!(side_by_side_pairs(&added_only).is_empty(), "an added run alone has no twin");
    }

    #[test]
    fn side_by_side_keeps_twins_on_one_row_and_fills_the_gaps_around_them() {
        let hunk = change_of(
            &["drop this", "let a = 1;", "drop that", "let c = 3;"],
            &["brand new", "another one", "let a = 10;", "let c = 30;", "and more"],
        );
        assert_eq!(hunk.pairs, [pair(1, 6), pair(3, 7)]);
        assert_eq!(side_by_side_pairs(&hunk), [pair(0, 4), pair(1, 6), pair(3, 7)]);
    }

    #[test]
    fn whitespace_only_changes_pair_and_real_ones_do_not() {
        assert!(same_but_whitespace("let a = 1;", "let  a = 1; "));
        assert!(same_but_whitespace("\tif x {", "    if x {"));
        assert!(!same_but_whitespace("let a = 1;", "let a = 1;"), "an unchanged line is no whitespace change");
        assert!(!same_but_whitespace("let a = 1;", "let a = 2;"));
        let hunk = mark(&parse("@@ -1,2 +1,2 @@\n-\tone();\n-two();\n+    one();\n+three();\n")[0]);
        assert_eq!(inline_or_whitespace_pairs(&hunk), [pair(0, 2)]);
    }
}
