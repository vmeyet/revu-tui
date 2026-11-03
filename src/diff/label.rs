//! How much a change matters, so a reviewer sees where to spend attention: a label for a hunk, the worst of its runs.
//! A run is one `-` block and the `+` block after it. Labels only inform: nothing hides, sorts or counts by them.
//! Rows draw no word for `Edited`, the common case, so no label on a row means an ordinary edit.
use super::align::{Change, MIN_LINE_SIMILARITY, changes, code_tokens, similarity, too_large_to_align};
use super::{Hunk, Line};

/// From the least to the most attention a change asks for: the order makes the worst of many their `max`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ChangeLabel {
    /// Only whitespace differs, line by line.
    Whitespace,
    /// The same code tokens laid out on other lines.
    Formatting,
    /// Small changes in similar lines, new code, removed code, or anything not proven harmless.
    Edited,
    /// Removed and added code with little in common.
    Rewritten,
}

impl ChangeLabel {
    pub(crate) fn word(self) -> &'static str {
        match self {
            Self::Whitespace => "whitespace",
            Self::Formatting => "formatting",
            Self::Edited => "edited",
            Self::Rewritten => "rewritten",
        }
    }
}

/// Extensions of languages where whitespace between tokens has no meaning; any other file never reads as harmless.
const WHITESPACE_FREE_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "go", "java", "c", "h", "cc", "cpp", "hpp", "css", "scss", "json", "tf", "tfvars", "hcl",
];
/// Fewest lines each side of a run needs to read as `Rewritten`: below this a run is a small edit, whatever its twins.
const MIN_REWRITTEN_LINES: usize = 3;
const QUOTES: [char; 3] = ['"', '\'', '`'];
/// What makes a line break part of the code: it ends a line comment, or a line of the preprocessor.
const LINE_BOUND_MARKS: [&str; 2] = ["//", "#"];

/// `hunk` with its label; `path` says whether whitespace can be harmless there. No changed line, no label.
pub(crate) fn labelled(hunk: Hunk, path: &str) -> Hunk {
    let label = changes(&hunk.lines).into_iter().map(|change| run_label(&hunk, change, path)).max();
    Hunk { label, ..hunk }
}

fn run_label(hunk: &Hunk, Change { removed, added }: Change, path: &str) -> ChangeLabel {
    let (old, new) = (&hunk.lines[removed.clone()], &hunk.lines[added]);
    if old.is_empty() || new.is_empty() {
        return ChangeLabel::Edited;
    }
    if same_code(old, new) {
        return if whitespace_is_free(path) { layout_label(old, new) } else { ChangeLabel::Edited };
    }
    let paired = hunk.pairs.iter().filter(|pair| removed.contains(&pair.removed)).count();
    let rewritten = !too_large_to_align(old, new) && mostly_unpaired(old.len(), new.len(), paired) && !alike_as_a_whole(old, new);
    if rewritten { ChangeLabel::Rewritten } else { ChangeLabel::Edited }
}

fn same_code(old: &[Line], new: &[Line]) -> bool {
    run_tokens(old).eq(run_tokens(new))
}

fn run_tokens(run: &[Line]) -> impl Iterator<Item = &str> {
    run.iter().flat_map(|line| code_tokens(&line.text))
}

fn whitespace_is_free(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, extension)| WHITESPACE_FREE_EXTENSIONS.iter().any(|known| known.eq_ignore_ascii_case(extension)))
}

/// The harmless label two runs of the same code earn, `Edited` when a quote or a line ending could carry meaning.
fn layout_label(old: &[Line], new: &[Line]) -> ChangeLabel {
    let same_lines = old.len() == new.len() && old.iter().zip(new).all(|(old, new)| code_tokens(&old.text) == code_tokens(&new.text));
    if same_lines {
        let safe = old.iter().zip(new).all(|(old, new)| whitespace_outside_literals(&old.text, &new.text));
        return if safe { ChangeLabel::Whitespace } else { ChangeLabel::Edited };
    }
    let safe = old.iter().chain(new).all(|line| !holds_quote(&line.text) && !LINE_BOUND_MARKS.iter().any(|mark| line.text.contains(mark)));
    if safe { ChangeLabel::Formatting } else { ChangeLabel::Edited }
}

/// Twins of the same tokens whose whitespace change cannot sit in a string: no quote at all, or only the
/// ends of the line moved and its quotes close on the line.
fn whitespace_outside_literals(old: &str, new: &str) -> bool {
    !holds_quote(old) || (old.trim() == new.trim() && QUOTES.iter().all(|quote| old.matches(*quote).count() % 2 == 0))
}

fn holds_quote(line: &str) -> bool {
    line.contains(QUOTES)
}

/// Code re-wrapped with one token added has no twins line by line, yet reads the same over the whole run.
fn alike_as_a_whole(old: &[Line], new: &[Line]) -> bool {
    let (old, new): (Vec<&str>, Vec<&str>) = (run_tokens(old).collect(), run_tokens(new).collect());
    similarity(&old, &new) >= MIN_LINE_SIMILARITY
}

/// Under half of the shorter side has a twin, the longer side's extra lines being new code rather than a rewrite.
fn mostly_unpaired(old: usize, new: usize, paired: usize) -> bool {
    let shorter = old.min(new);
    shorter >= MIN_REWRITTEN_LINES && paired * 2 < shorter
}

#[cfg(test)]
mod tests {
    use super::super::{parse, words};
    use super::*;

    fn label_at(path: &str, diff: &str) -> Option<ChangeLabel> {
        let hunk = words::mark(&parse(diff)[0]);
        labelled(hunk, path).label
    }

    fn label_of(diff: &str) -> Option<ChangeLabel> {
        label_at("a.rs", diff)
    }

    /// A run of `lines` lines on each side, each made by `removed` and `added` from its number.
    fn run_of(lines: usize, removed: impl Fn(usize) -> String, added: impl Fn(usize) -> String) -> String {
        let old: String = (0..lines).map(|i| format!("-{}\n", removed(i))).collect();
        let new: String = (0..lines).map(|i| format!("+{}\n", added(i))).collect();
        format!("@@ -1,{lines} +1,{lines} @@\n{old}{new}")
    }

    const REINDENT: &str = "@@ -1 +1 @@\n-  charge(card);\n+    charge(card);\n";

    #[test]
    fn labels_order_from_the_least_to_the_most_attention() {
        assert!(ChangeLabel::Whitespace < ChangeLabel::Formatting);
        assert!(ChangeLabel::Formatting < ChangeLabel::Edited);
        assert!(ChangeLabel::Edited < ChangeLabel::Rewritten);
    }

    #[test]
    fn a_reindented_line_reads_as_whitespace() {
        assert_eq!(label_of(REINDENT), Some(ChangeLabel::Whitespace));
    }

    #[test]
    fn spaces_inside_a_line_read_as_whitespace() {
        assert_eq!(label_of("@@ -1 +1 @@\n-charge( card );\n+charge(card);\n"), Some(ChangeLabel::Whitespace));
    }

    #[test]
    fn a_line_re_wrapped_onto_more_lines_reads_as_formatting() {
        let diff = "@@ -1 +1,3 @@\n-charge(card, total);\n+charge(\n+    card, total\n+);\n";
        assert_eq!(label_of(diff), Some(ChangeLabel::Formatting));
    }

    #[test]
    fn tokens_moved_across_as_many_lines_read_as_formatting() {
        let diff = "@@ -1,2 +1,2 @@\n-charge(card,\n-    total);\n+charge(\n+    card, total);\n";
        assert_eq!(label_of(diff), Some(ChangeLabel::Formatting));
    }

    #[test]
    fn two_words_joined_read_as_edited() {
        assert_eq!(label_of("@@ -1 +1 @@\n-let total;\n+lettotal;\n"), Some(ChangeLabel::Edited));
    }

    #[test]
    fn a_small_change_in_a_similar_line_reads_as_edited() {
        assert_eq!(label_of("@@ -1 +1 @@\n-let total = 1;\n+let total = 2;\n"), Some(ChangeLabel::Edited));
    }

    #[test]
    fn new_code_reads_as_edited() {
        assert_eq!(label_of("@@ -0,0 +1,2 @@\n+let a = 1;\n+let b = 2;\n"), Some(ChangeLabel::Edited));
    }

    #[test]
    fn removed_code_reads_as_edited() {
        assert_eq!(label_of("@@ -1,2 +0,0 @@\n-let a = 1;\n-let b = 2;\n"), Some(ChangeLabel::Edited));
    }

    #[test]
    fn a_blank_line_added_reads_as_edited() {
        assert_eq!(label_of("@@ -1 +1,2 @@\n a\n+\n"), Some(ChangeLabel::Edited));
    }

    #[test]
    fn a_hunk_of_context_alone_has_no_label() {
        assert_eq!(label_of("@@ -1 +1 @@\n a\n"), None);
    }

    fn unrelated(lines: usize) -> String {
        run_of(lines, |i| format!("let total{i} = sum(items, {i});"), |i| format!("return Err(Refused::At({i}).into_report());"))
    }

    #[test]
    fn code_replaced_by_code_with_little_in_common_reads_as_rewritten() {
        assert_eq!(label_of(&unrelated(MIN_REWRITTEN_LINES)), Some(ChangeLabel::Rewritten));
    }

    #[test]
    fn a_short_replacement_stays_edited_whatever_its_twins() {
        assert_eq!(label_of(&unrelated(MIN_REWRITTEN_LINES - 1)), Some(ChangeLabel::Edited));
    }

    #[test]
    fn a_rewrite_reads_as_rewritten_in_a_file_where_whitespace_has_meaning() {
        assert_eq!(label_at("a.py", &unrelated(MIN_REWRITTEN_LINES)), Some(ChangeLabel::Rewritten));
    }

    #[test]
    fn lines_that_keep_their_twins_among_many_new_ones_stay_edited() {
        let kept = "-let a = sum(items, 1);\n-let b = sum(items, 2);\n-let c = sum(items, 3);\n";
        let twins = "+let a = sum(items, 9);\n+let b = sum(items, 8);\n+let c = sum(items, 7);\n";
        let added: String = (0..20).map(|i| format!("+return Err(Refused::At({i}).into_report());\n")).collect();
        assert_eq!(label_of(&format!("@@ -1,3 +1,23 @@\n{kept}{twins}{added}")), Some(ChangeLabel::Edited));
    }

    #[test]
    fn code_re_wrapped_with_one_token_added_stays_edited() {
        let diff = "@@ -1,3 +1,7 @@\n-charge(card, total, currency);\n-refund(card, total);\n-log(card);\n\
                    +charge(\n+    card, total, currency,\n+);\n+refund(\n+    card, total,\n+);\n+log(card,);\n";
        assert_eq!(label_of(diff), Some(ChangeLabel::Edited));
    }

    #[test]
    fn a_run_over_the_alignment_cap_never_reads_as_rewritten() {
        assert_eq!(label_of(&unrelated(51)), Some(ChangeLabel::Edited));
    }

    #[test]
    fn a_run_over_the_alignment_cap_with_few_pairs_by_position_never_reads_as_rewritten() {
        let alike = |i: usize| format!("let total{i} = sum(items, 1);");
        let unlike = |i: usize| format!("return Err(Refused::At({i}).into_report());");
        let diff = run_of(51, alike, |i| if i < 5 { format!("let total{i} = sum(items, 2);") } else { unlike(i) });
        assert_eq!(words::mark(&parse(&diff)[0]).pairs.len(), 5);
        assert_eq!(label_of(&diff), Some(ChangeLabel::Edited));
    }

    #[test]
    fn a_run_too_heavy_to_align_never_reads_as_rewritten() {
        let heavy = |word: &str, i: usize| format!("{i} {}", word.repeat(800));
        assert_eq!(label_of(&run_of(10, |i| heavy("old_call(); ", i), |i| heavy("new, ", i))), Some(ChangeLabel::Edited));
    }

    #[test]
    fn a_one_line_change_that_shares_a_word_reads_as_edited() {
        let diff = "@@ -1 +1 @@\n-charge(card, total);\n+let receipt = charge(card).with_total(total).await?;\n";
        assert_eq!(words::mark(&parse(diff)[0]).pairs.len(), 1);
        assert_eq!(label_of(diff), Some(ChangeLabel::Edited));
    }

    #[test]
    fn a_run_over_the_alignment_cap_still_reads_as_whitespace_when_proven() {
        let reindented = run_of(51, |i| format!("  a{i};"), |i| format!("    a{i};"));
        assert_eq!(label_of(&reindented), Some(ChangeLabel::Whitespace));
    }

    #[test]
    fn a_run_over_the_alignment_cap_still_reads_as_formatting_when_proven() {
        let old: String = (0..51).map(|i| format!("-a{i}, b{i},\n")).collect();
        let new: String = (0..51).map(|i| format!("+a{i},\n+b{i},\n")).collect();
        assert_eq!(label_of(&format!("@@ -1,51 +1,102 @@\n{old}{new}")), Some(ChangeLabel::Formatting));
    }

    #[test]
    fn whitespace_counts_as_harmless_only_in_listed_extensions() {
        for path in ["a.py", "ci.yaml", "ci.yml", "Makefile", "README.md", "a.unknown", "no_extension", "a.rs/Makefile"] {
            assert_eq!(label_at(path, REINDENT), Some(ChangeLabel::Edited), "{path}");
        }
        for path in ["src/a.rs", "a.tsx", "main.tf", "A.RS"] {
            assert_eq!(label_at(path, REINDENT), Some(ChangeLabel::Whitespace), "{path}");
        }
    }

    #[test]
    fn re_wrapped_lines_in_an_unlisted_file_read_as_edited_not_rewritten() {
        let diff = "@@ -1,3 +1,9 @@\n-call(a, b)\n-call(c, d)\n-call(e, f)\n\
                    +call(\n+a, b)\n+call(\n+c, d)\n+call(\n+e, f)\n";
        assert_eq!(label_at("a.py", diff), Some(ChangeLabel::Edited));
    }

    #[test]
    fn whitespace_changed_inside_a_string_reads_as_edited() {
        for quote in QUOTES {
            let diff = format!("@@ -1 +1 @@\n-log({quote}a  b{quote});\n+log({quote}a b{quote});\n");
            assert_eq!(label_of(&diff), Some(ChangeLabel::Edited), "{quote}");
        }
    }

    #[test]
    fn a_reindented_line_whose_strings_close_on_it_reads_as_whitespace() {
        assert_eq!(label_of("@@ -1 +1 @@\n-  log(\"a  b\");\n+    log(\"a  b\");\n"), Some(ChangeLabel::Whitespace));
    }

    #[test]
    fn a_reindented_line_with_a_lone_quote_reads_as_edited() {
        assert_eq!(label_of("@@ -1 +1 @@\n-  end of a long string\";\n+    end of a long string\";\n"), Some(ChangeLabel::Edited));
    }

    #[test]
    fn spaces_moved_next_to_a_string_read_as_edited() {
        assert_eq!(label_of("@@ -1 +1 @@\n-log( \"a\" );\n+log(\"a\");\n"), Some(ChangeLabel::Edited));
    }

    #[test]
    fn a_string_re_wrapped_reads_as_edited() {
        let diff = "@@ -1 +1,2 @@\n-log(\"a b\");\n+log(\"a\n+b\");\n";
        assert_eq!(label_of(diff), Some(ChangeLabel::Edited));
    }

    #[test]
    fn code_pulled_into_a_line_comment_reads_as_edited() {
        let diff = "@@ -1,2 +1 @@\n-charge(); // first\n-refund();\n+charge(); // first refund();\n";
        assert_eq!(label_of(diff), Some(ChangeLabel::Edited));
    }

    #[test]
    fn a_hunk_reads_as_its_worst_run() {
        let diff = format!("{REINDENT} kept\n-let a = 1;\n+let a = 2;\n");
        assert_eq!(label_of(&diff), Some(ChangeLabel::Edited));
    }
}
