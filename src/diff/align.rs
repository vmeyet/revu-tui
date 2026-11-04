//! Which removed lines are twins of which added lines: each run of removed lines is aligned with the run of added lines
//! after it, so a line pairs with its most alike twin wherever it sits; pairs never cross, and a line with no twin stays plain.
//! A one-line change has only one candidate, so one word in common is enough to pair it.
//! A file has one budget of work: a run the budget cannot align compares its lines by position, and past that stays plain.
use super::words::{same_but_whitespace, tokens};
use super::{Line, LineKind, LinePair};
use similar::TextDiff;
use std::collections::HashSet;
use std::ops::Range;

/// Least similarity, over a line's code tokens, for two lines to count as twins.
pub(super) const MIN_LINE_SIMILARITY: f32 = 0.6;
/// Longest run, on either side, that is aligned: aligning costs one word diff per pair of lines.
pub(super) const MAX_ALIGNED_RUN: usize = 50;
/// Most work one file's word diffs may take, a unit being one byte of a line compared with one byte of another:
/// a diff of two unlike lines costs about the product of their lengths. Measured in release, this much takes up to
/// about 550ms on minified code, the densest text a file really holds, and 15ms on ordinary code.
/// It is one line of 6.7KB a side, or 50 lines of 190 bytes a side aligned.
pub(super) const MAX_FILE_WORK: usize = 90_000_000;

/// How the lines of a run were compared: the richest way the work left in the file allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Comparison {
    /// Every `-` line against every `+` line, so a line without a twin truly has none.
    Aligned,
    /// Each line against the one at the same place only.
    ByPosition,
    /// Nothing diffed: only lines that differ in whitespace pair, so a reindented block still reads quiet under `W`.
    WhitespaceOnly,
}

pub(super) struct ComparedRun {
    pub(super) pairs: Vec<LinePair>,
    pub(super) comparison: Comparison,
    /// What this cost out of the file's work, the word diffs of its pairs included.
    pub(super) work: usize,
}

/// The twins of a `-` run in the `+` run after it, spending at most `work_left`.
pub(super) fn compare_run(lines: &[Line], Change { removed, added }: &Change, work_left: usize) -> ComparedRun {
    let (old, new) = (&lines[removed.clone()], &lines[added.clone()]);
    let word_diffs = position_work(old, new);
    let aligned_work = run_bytes(old) * run_bytes(new) + word_diffs;
    if old.len() <= MAX_ALIGNED_RUN && new.len() <= MAX_ALIGNED_RUN && aligned_work <= work_left {
        return ComparedRun { pairs: align_run(lines, removed, added), comparison: Comparison::Aligned, work: aligned_work };
    }
    let compare_then_diff = 2 * word_diffs;
    if compare_then_diff <= work_left {
        let pairs = twins_by_position(lines, removed, added, alike_enough);
        return ComparedRun { pairs, comparison: Comparison::ByPosition, work: compare_then_diff };
    }
    let pairs = twins_by_position(lines, removed, added, same_but_whitespace);
    ComparedRun { pairs, comparison: Comparison::WhitespaceOnly, work: 0 }
}

fn align_run(lines: &[Line], removed: &Range<usize>, added: &Range<usize>) -> Vec<LinePair> {
    let (old, new) = (&lines[removed.clone()], &lines[added.clone()]);
    if let ([old_line], [new_line]) = (old, new)
        && share_a_word(&old_line.text, &new_line.text)
    {
        return pairs_by_position(removed.clone(), added.clone()).collect();
    }
    let in_hunk = |pair: LinePair| LinePair { removed: removed.start + pair.removed, added: added.start + pair.added };
    best_alignment(&twin_scores(old, new)).into_iter().map(in_hunk).collect()
}

fn twins_by_position(lines: &[Line], removed: &Range<usize>, added: &Range<usize>, keep: fn(&str, &str) -> bool) -> Vec<LinePair> {
    pairs_by_position(removed.clone(), added.clone()).filter(|pair| keep(&lines[pair.removed].text, &lines[pair.added].text)).collect()
}

fn position_work(removed: &[Line], added: &[Line]) -> usize {
    removed.iter().zip(added).map(|(old, new)| old.text.len() * new.text.len()).sum()
}

fn run_bytes(run: &[Line]) -> usize {
    run.iter().map(|line| line.text.len()).sum()
}

fn alike_enough(old: &str, new: &str) -> bool {
    twin_score(old, new, &code_tokens(old), &code_tokens(new)) > 0.0
}

/// A word holds a letter: a shared number or bracket says nothing about two lines being the same line.
fn share_a_word(old: &str, new: &str) -> bool {
    let new_tokens: HashSet<&str> = code_tokens(new).into_iter().collect();
    code_tokens(old).into_iter().any(|token| token.contains(char::is_alphabetic) && new_tokens.contains(token))
}

pub(super) fn pairs_by_position(removed: Range<usize>, added: Range<usize>) -> impl Iterator<Item = LinePair> {
    removed.zip(added).map(|(removed, added)| LinePair { removed, added })
}

/// How alike each removed line is to each added line, `0` below the bar for twins.
fn twin_scores(removed: &[Line], added: &[Line]) -> Vec<Vec<f32>> {
    let old_tokens: Vec<Vec<&str>> = removed.iter().map(|line| code_tokens(&line.text)).collect();
    let new_tokens: Vec<Vec<&str>> = added.iter().map(|line| code_tokens(&line.text)).collect();
    let score = |r: usize, a: usize| twin_score(&removed[r].text, &added[a].text, &old_tokens[r], &new_tokens[a]);
    (0..removed.len()).map(|r| (0..added.len()).map(|a| score(r, a)).collect()).collect()
}

/// How alike two lines are, `0` below the bar for twins; a change of whitespace alone makes them alike in full.
fn twin_score(old: &str, new: &str, old_tokens: &[&str], new_tokens: &[&str]) -> f32 {
    let alike = if same_but_whitespace(old, new) { 1.0 } else { similarity(old_tokens, new_tokens) };
    if alike >= MIN_LINE_SIMILARITY { alike } else { 0.0 }
}

/// The pairs of `scores` rows and columns with the highest total that never cross, all above `0`; ties go to the earliest pair.
fn best_alignment(scores: &[Vec<f32>]) -> Vec<LinePair> {
    let (olds, news) = (scores.len(), scores.first().map_or(0, Vec::len));
    let mut best = vec![vec![0.0_f32; news + 1]; olds + 1];
    for old in (0..olds).rev() {
        for new in (0..news).rev() {
            let taken = if scores[old][new] > 0.0 { scores[old][new] + best[old + 1][new + 1] } else { 0.0 };
            best[old][new] = taken.max(best[old + 1][new]).max(best[old][new + 1]);
        }
    }
    let mut found = Vec::new();
    let (mut old, mut new) = (0, 0);
    while old < olds && new < news {
        let skipped = best[old + 1][new].max(best[old][new + 1]);
        if scores[old][new] > 0.0 && scores[old][new] + best[old + 1][new + 1] >= skipped {
            found.push(LinePair { removed: old, added: new });
            old += 1;
            new += 1;
        } else if best[old + 1][new] >= best[old][new + 1] {
            old += 1;
        } else {
            new += 1;
        }
    }
    found
}

/// How much of two token lists match in order, from 0 to 1; nothing alike when either is empty.
pub(super) fn similarity(old: &[&str], new: &[&str]) -> f32 {
    if old.is_empty() || new.is_empty() {
        return 0.0;
    }
    TextDiff::from_slices(old, new).ratio()
}

/// The tokens that make a line's code what it is: indentation and spacing say nothing about which line it is.
pub(super) fn code_tokens(line: &str) -> Vec<&str> {
    tokens(line).into_iter().filter(|token| !token.starts_with(char::is_whitespace)).collect()
}

/// A `-` run and the `+` run right after it, as index ranges into a hunk's lines, either one possibly empty.
pub(super) struct Change {
    pub(super) removed: Range<usize>,
    pub(super) added: Range<usize>,
}

pub(super) fn changes(lines: &[Line]) -> Vec<Change> {
    let mut found = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let removed = run(lines, i, LineKind::Removed);
        let added = run(lines, removed.end, LineKind::Added);
        if !removed.is_empty() || !added.is_empty() {
            found.push(Change { removed: removed.clone(), added: added.clone() });
        }
        i = added.end.max(removed.end).max(i + 1);
    }
    found
}

fn run(lines: &[Line], from: usize, kind: LineKind) -> Range<usize> {
    let end = lines[from..].iter().position(|l| l.kind != kind).map_or(lines.len(), |n| from + n);
    from..end
}

#[cfg(test)]
mod tests {
    use super::super::parse;
    use super::super::words::tests::{change_of, pair, words_of};
    use super::super::words::{mark, mark_file};
    use super::*;

    #[test]
    fn a_run_one_line_longer_on_one_side_still_marks_its_twin() {
        let hunk = mark(&parse(include_str!("fixtures/tabs.diff"))[0]);
        assert_eq!(hunk.pairs, [pair(1, 2)]);
        assert_eq!(words_of(&hunk.lines[1]), vec!["2"]);
        assert_eq!(words_of(&hunk.lines[2]), vec!["3"]);
        assert_eq!(words_of(&hunk.lines[3]), [] as [&str; 0]);
    }

    #[test]
    fn a_line_inserted_and_another_deleted_leave_the_lines_between_with_their_own_twins() {
        let hunk = change_of(
            &["let a = 1;", "let b = 2;", "let c = 3;", "gone_for_good(now);"],
            &["brand_new(call);", "let a = 10;", "let b = 20;", "let c = 30;"],
        );
        assert_eq!(hunk.pairs, [pair(0, 5), pair(1, 6), pair(2, 7)]);
        assert_eq!(words_of(&hunk.lines[6]), vec!["20"]);
    }

    #[test]
    fn a_full_rewrite_gets_no_ranges() {
        let hunk = change_of(&["let total = price * qty;", "return total;"], &["for item in cart {", "    charge(item)?;"]);
        assert_eq!(hunk.pairs, []);
        assert!(hunk.lines.iter().all(|l| l.words.is_empty()));
    }

    #[test]
    fn a_removed_line_with_no_twin_between_two_pairs_gets_no_ranges() {
        let hunk = change_of(&["let a = 1;", "drop_this(now);", "let c = 3;"], &["let a = 10;", "let c = 30;"]);
        assert_eq!(hunk.pairs, [pair(0, 3), pair(2, 4)]);
        assert_eq!(words_of(&hunk.lines[1]), [] as [&str; 0]);
    }

    #[test]
    fn lines_alike_only_in_their_indentation_are_no_twins() {
        let hunk = change_of(&["    one    "], &["    two    "]);
        assert_eq!(hunk.pairs, []);
    }

    #[test]
    fn lines_of_punctuation_alone_can_be_twins() {
        let hunk = change_of(&["});"], &["})"]);
        assert_eq!(hunk.pairs, [pair(0, 1)]);
        assert_eq!(words_of(&hunk.lines[0]), vec![";"]);
    }

    #[test]
    fn a_blank_line_that_lost_its_spaces_pairs_with_its_twin() {
        assert_eq!(change_of(&["    "], &[""]).pairs, [pair(0, 1)]);
    }

    fn numbered_lines(count: usize, end: &str) -> Vec<String> {
        (0..count).map(|n| format!("let value{n} = compute({n}){end}")).collect()
    }

    #[test]
    fn a_run_at_the_cap_is_aligned() {
        let (old, new) = (numbered_lines(MAX_ALIGNED_RUN, ";"), numbered_lines(MAX_ALIGNED_RUN, "?;"));
        assert_eq!(change_of(&old, &new).pairs.len(), MAX_ALIGNED_RUN);
    }

    #[test]
    fn a_run_past_the_cap_on_one_side_pairs_alike_lines_by_position() {
        let (old, new) = (numbered_lines(2, ";"), numbered_lines(MAX_ALIGNED_RUN + 1, "?;"));
        let hunk = change_of(&old, &new);
        assert_eq!(hunk.pairs, [pair(0, 2), pair(1, 3)]);
        assert_eq!(words_of(&hunk.lines[3]), ["?"]);
    }

    /// A run too long to align of alike lines, each long enough for the run to take about `work` when compared by position.
    fn lines_taking(work: usize, end: &str) -> Vec<String> {
        let count = MAX_ALIGNED_RUN + 1;
        let line = format!("{}{end}", "word ".repeat((work / count / 2).isqrt() / 5));
        (0..count).map(|n| format!("{n:05} {line}")).collect()
    }

    #[test]
    fn a_run_within_the_file_budget_pairs_its_alike_lines_by_position() {
        let (old, new) = (lines_taking(MAX_FILE_WORK / 2, "a"), lines_taking(MAX_FILE_WORK / 2, "b"));
        assert_eq!(change_of(&old, &new).pairs.len(), old.len());
    }

    #[test]
    fn a_run_past_the_file_budget_stays_unmarked() {
        let (old, new) = (lines_taking(MAX_FILE_WORK * 2, "a"), lines_taking(MAX_FILE_WORK * 2, "b"));
        let hunk = change_of(&old, &new);
        assert_eq!(hunk.pairs, []);
        assert!(hunk.lines.iter().all(|l| l.words.is_empty()));
    }

    #[test]
    fn a_reindented_run_past_the_file_budget_pairs_its_lines_with_no_word_diff() {
        let old = lines_taking(MAX_FILE_WORK * 2, "a");
        let new: Vec<String> = old.iter().map(|line| format!("    {line}")).collect();
        let hunk = change_of(&old, &new);
        assert_eq!(hunk.pairs.len(), old.len());
        assert!(hunk.lines.iter().all(|l| l.words.is_empty()));
    }

    #[test]
    fn a_short_run_the_budget_cannot_align_compares_its_lines_by_position() {
        let long_word = "word ".repeat(60);
        let old: Vec<String> = (0..MAX_ALIGNED_RUN).map(|n| format!("{n} {long_word}a ").repeat(2)).collect();
        let new: Vec<String> = (0..MAX_ALIGNED_RUN).map(|n| format!("{n} {long_word}b ").repeat(2)).collect();
        let hunk = change_of(&old, &new);
        assert_eq!(hunk.pairs.len(), MAX_ALIGNED_RUN);
        assert_eq!(hunk.unaligned, [0]);
    }

    /// A one-line hunk costing about `work`.
    fn hunk_taking(work: usize, at: usize) -> String {
        let line = format!("{at} {}", "word ".repeat((work / 2).isqrt() / 5));
        format!("@@ -{at} +{at} @@\n-{line}a\n+{line}b\n")
    }

    #[test]
    fn hunks_of_one_file_share_its_budget() {
        let two_hunks = parse(&format!("{}{}", hunk_taking(MAX_FILE_WORK * 2 / 3, 1), hunk_taking(MAX_FILE_WORK * 2 / 3, 2)));
        let marked = mark_file(&two_hunks);
        assert_eq!(marked[0].pairs.len(), 1);
        assert_eq!(marked[1].pairs, [], "the second hunk finds the budget spent");
        assert_eq!(mark_file(&two_hunks[1..])[0].pairs.len(), 1, "alone it fits");
    }

    #[test]
    fn a_whitespace_only_change_on_a_very_long_line_pairs_with_no_word_diff() {
        let (old, new) = ("a,".repeat(800_000), "a, ".repeat(800_000));
        let hunk = change_of(&[old], &[new]);
        assert_eq!(hunk.pairs, [pair(0, 1)]);
        assert!(hunk.lines.iter().all(|l| l.words.is_empty()));
    }

    #[test]
    fn a_one_line_change_below_the_bar_pairs_when_its_lines_share_a_word() {
        let (old, new) = ("charge(card, total);", "let receipt = charge(card).with_total(total).await?;");
        assert!(!alike_enough(old, new));
        let hunk = change_of(&[old], &[new]);
        assert_eq!(hunk.pairs, [pair(0, 1)]);
        assert_eq!(words_of(&hunk.lines[1]), ["let receipt = ", ").with_total(", ".await?"]);
    }

    #[test]
    fn a_one_line_change_sharing_only_punctuation_stays_plain() {
        let (old, new) = (".counts(DIFF),", ".section(Section::View),");
        assert!(!alike_enough(old, new));
        assert_eq!(change_of(&[old], &[new]).pairs, []);
    }

    #[test]
    fn a_one_line_change_sharing_only_a_number_stays_plain() {
        let (old, new) = ("let a = 1;", "return 1;");
        assert!(!alike_enough(old, new));
        assert_eq!(change_of(&[old], &[new]).pairs, []);
    }

    #[test]
    fn a_word_in_common_pairs_nothing_when_a_side_has_more_than_one_line() {
        let (old, new) = ("charge(card, total);", "let receipt = charge(card).with_total(total).await?;");
        assert_eq!(change_of(&[old], &[new, "log(receipt);"]).pairs, []);
    }

    #[test]
    fn a_run_of_lines_too_heavy_to_diff_stays_unmarked() {
        let heavy = |end: &str| format!("{} {end}", "word ".repeat(MAX_FILE_WORK.isqrt() / 5 + 1));
        let hunk = change_of(&[heavy("a")], &[heavy("b")]);
        assert_eq!(hunk.pairs, []);
        assert!(hunk.lines.iter().all(|l| l.words.is_empty()));
    }

    #[test]
    fn a_reindented_run_past_the_cap_still_pairs_its_whitespace_changes_by_position() {
        let old = numbered_lines(MAX_ALIGNED_RUN + 1, ";");
        let new: Vec<String> = old.iter().map(|line| format!("    {line}")).collect();
        let hunk = change_of(&old, &new);
        assert_eq!(hunk.pairs.len(), MAX_ALIGNED_RUN + 1);
    }

    #[test]
    fn the_alignment_takes_the_best_total_over_the_best_single_pair() {
        let scores = [vec![0.7, 0.9], vec![0.0, 0.7]];
        assert_eq!(best_alignment(&scores), [pair(0, 0), pair(1, 1)]);
    }

    #[test]
    fn the_alignment_of_a_tie_takes_the_earliest_pair() {
        let scores = [vec![0.8, 0.8]];
        assert_eq!(best_alignment(&scores), [pair(0, 0)]);
    }

    #[test]
    fn the_alignment_of_crossing_lines_keeps_the_better_of_the_two() {
        let scores = [vec![0.0, 0.9], vec![0.7, 0.0]];
        assert_eq!(best_alignment(&scores), [pair(0, 1)]);
    }

    #[test]
    fn similarity_is_the_share_of_tokens_matched_in_order_and_nothing_when_a_side_is_empty() {
        assert!((similarity(&["a", "b"], &["a", "c"]) - 0.5).abs() < f32::EPSILON);
        assert!((similarity(&["a"], &["a"]) - 1.0).abs() < f32::EPSILON);
        assert!(similarity(&[], &[]).abs() < f32::EPSILON);
        assert!(similarity(&["a"], &[]).abs() < f32::EPSILON);
    }
}
