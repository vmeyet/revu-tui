//! What Claude reads for a question: the MR in one cached block, the part asked about in another,
//! within a character budget, and never a hunk cut in the middle.
use super::anthropic::{Ask, Block, Role, Turn};
use crate::diff::{Hunk, LineKind};
use crate::review::{File, Review};
use std::fmt::Write as _;
use std::ops::RangeInclusive;

pub const BUDGET: usize = 60_000;
const DESCRIPTION_CHARS: usize = 2000;
const AROUND: usize = 20;

const SYSTEM: &str = "You help a code reviewer reading a merge request (a pull request on GitHub) in a terminal.
Answer in plain markdown, short paragraphs, code in fences. Refer to lines as `path:new_line`.
When asked for a comment, write it as the reviewer would post it: direct, specific, no greeting.
Say when the diff alone cannot answer.";

/// The part of the MR a question is about.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    Mr,
    File(String),
    /// The hunk at this index of the file.
    Hunk(String, usize),
    /// A thread by id: its hunk and every note.
    Thread(String),
    /// New-side line numbers in one file.
    Lines(String, RangeInclusive<u32>),
}

/// The questions the `a` menu offers, and a free one.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Prompt {
    Explain,
    Risks,
    Summary,
    Thread,
    /// A review comment about the concern given.
    Comment(String),
    Free(String),
}

impl Prompt {
    pub fn question(&self) -> String {
        match self {
            Prompt::Explain => "Explain what this change does and why it might be here.".into(),
            Prompt::Risks => "List the risks and the things a careful reviewer should check. Be concrete.".into(),
            Prompt::Summary => "Summarise this MR in five lines and suggest the reading order of the files.".into(),
            Prompt::Thread => "Summarise this thread and say what is still open and who it waits on.".into(),
            Prompt::Comment(concern) => format!("Draft a review comment for these lines about: {concern}"),
            Prompt::Free(text) => text.clone(),
        }
    }
}

/// The first call of a conversation: system rules, the MR, the scope, then the question.
pub fn ask(review: &Review, scope: &Scope, prompt: &Prompt, budget: usize) -> Ask {
    let mut system = vec![Block { text: SYSTEM.into(), cached: false }, Block { text: mr_block(review), cached: true }];
    if *scope == Scope::Mr {
        system[1].text.push_str(&files_within(review.files.iter(), budget));
    } else {
        system.push(Block { text: scope_block(review, scope, budget), cached: true });
    }
    Ask { system, turns: vec![Turn { role: Role::User, text: prompt.question() }] }
}

/// Always sent: title, description, pipeline and the file list with counts.
fn mr_block(review: &Review) -> String {
    let mr = &review.mr;
    let mut text = format!("# {} {}\n\n", mr.project, mr.title);
    let _ = writeln!(text, "Author: {} · {} → {}", mr.author.username, mr.source_branch, mr.target_branch);
    if let Some(pipeline) = &mr.pipeline {
        let _ = writeln!(text, "Pipeline: {}", pipeline.status);
    }
    let _ = writeln!(text, "\n## Description\n\n{}\n\n## Files", cut(&mr.description, DESCRIPTION_CHARS));
    for file in &review.files {
        let _ = writeln!(text, "- {} (+{} −{})", file.new_path, file.additions, file.deletions);
    }
    text
}

fn scope_block(review: &Review, scope: &Scope, budget: usize) -> String {
    match scope {
        Scope::Mr => String::new(),
        Scope::File(path) => files_within(review.files.iter().filter(|f| f.new_path == *path), budget),
        Scope::Hunk(path, index) => file_of(review, path).map(|f| around(f, *index)).unwrap_or_default(),
        Scope::Thread(id) => thread_block(review, id),
        Scope::Lines(path, lines) => file_of(review, path)
            .and_then(|f| f.hunks.iter().position(|h| covers(h, lines)).map(|i| (f, i)))
            .map(|(f, i)| format!("{}\nThe reviewer asks about {path} lines {}-{}.\n", around(f, i), lines.start(), lines.end()))
            .unwrap_or_default(),
    }
}

/// Whole hunks of `files`, from the top, while they fit; a line says what was left out.
fn files_within<'a>(files: impl Iterator<Item = &'a File>, budget: usize) -> String {
    let mut text = String::from("\n## Diff\n");
    let mut left_out = 0;
    for file in files {
        let _ = writeln!(text, "\n### {}\n", file.new_path);
        for hunk in &file.hunks {
            let rendered = render(hunk, 0..hunk.lines.len());
            if left_out > 0 || text.len() + rendered.len() > budget {
                left_out += 1;
                continue;
            }
            text.push_str(&rendered);
        }
    }
    if left_out > 0 {
        let _ = writeln!(text, "\n({left_out} more hunks left out to fit; ask about a file to see them.)");
    }
    text
}

/// A hunk whole, with up to 20 lines of the hunks on each side for context.
fn around(file: &File, index: usize) -> String {
    let mut text = format!("## {}\n\n", file.new_path);
    if let Some(before) = index.checked_sub(1).and_then(|i| file.hunks.get(i)) {
        text.push_str(&render(before, before.lines.len().saturating_sub(AROUND)..before.lines.len()));
    }
    if let Some(hunk) = file.hunks.get(index) {
        text.push_str(&render(hunk, 0..hunk.lines.len()));
    }
    if let Some(after) = file.hunks.get(index + 1) {
        text.push_str(&render(after, 0..after.lines.len().min(AROUND)));
    }
    text
}

fn thread_block(review: &Review, id: &str) -> String {
    let Some(thread) = review.thread(id) else { return String::new() };
    let mut text = String::new();
    if let Some(anchor) = &thread.anchor {
        let lines = anchor.line..=anchor.line;
        let hunk = file_of(review, &anchor.path).and_then(|f| f.hunks.iter().position(|h| covers(h, &lines)).map(|i| (f, i)));
        if let Some((file, index)) = hunk {
            text.push_str(&around(file, index));
        }
        let _ = writeln!(text, "\nThe thread sits on {}:{}.", anchor.path, anchor.line);
    }
    text.push_str("\n## Thread\n\n");
    for note in thread.notes.iter().filter(|n| !n.system) {
        let _ = writeln!(text, "{}: {}\n", note.author.username, note.body);
    }
    text
}

fn file_of<'a>(review: &'a Review, path: &str) -> Option<&'a File> {
    review.files.iter().find(|f| f.new_path == path)
}

fn covers(hunk: &Hunk, lines: &RangeInclusive<u32>) -> bool {
    hunk.lines.iter().filter_map(|l| l.new).any(|n| lines.contains(&n))
}

fn render(hunk: &Hunk, lines: std::ops::Range<usize>) -> String {
    let mut text = format!("{}\n", hunk.header);
    for line in &hunk.lines[lines] {
        let sign = match line.kind {
            LineKind::Added => '+',
            LineKind::Removed => '-',
            LineKind::Context => ' ',
        };
        let _ = writeln!(text, "{sign}{}", line.text);
    }
    text
}

fn cut(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn review() -> Review {
        crate::review::tests::review()
    }

    fn scope_text(scope: &Scope, budget: usize) -> String {
        ask(&review(), scope, &Prompt::Explain, budget).system.iter().map(|b| b.text.clone()).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn the_mr_and_the_scope_are_separate_cached_blocks_and_the_question_comes_last() {
        let review = review();
        let path = review.files[0].new_path.clone();
        let ask = ask(&review, &Scope::File(path.clone()), &Prompt::Explain, BUDGET);
        assert_eq!(ask.system.iter().map(|b| b.cached).collect::<Vec<_>>(), [false, true, true]);
        assert!(ask.system[1].text.contains(&review.mr.title));
        assert!(ask.system[2].text.contains(&format!("### {path}")));
        assert_eq!(ask.turns, vec![Turn { role: Role::User, text: Prompt::Explain.question() }]);
    }

    #[test]
    fn a_small_budget_keeps_whole_hunks_and_says_how_many_were_left_out() {
        let full = scope_text(&Scope::Mr, BUDGET);
        let small = scope_text(&Scope::Mr, 200);
        assert!(small.len() < full.len());
        assert!(small.contains("more hunks left out"), "{small}");
        for kept in small.split("@@ ").skip(1) {
            let header = format!("@@ {}", kept.lines().next().unwrap());
            assert!(full.contains(&header), "only whole hunks are kept: {header}");
        }
    }

    #[test]
    fn lines_bring_their_hunk() {
        let review = review();
        let file = &review.files[0];
        let line = file.hunks[0].lines.iter().find_map(|l| l.new).unwrap();
        let text = scope_text(&Scope::Lines(file.new_path.clone(), line..=line), BUDGET);
        assert!(text.contains(&file.hunks[0].header) && text.contains(&format!("lines {line}-{line}")), "{text}");
    }

    #[test]
    fn hunks_and_threads_bring_their_code() {
        let review = review();
        let file = &review.files[0];
        let text = scope_text(&Scope::Hunk(file.new_path.clone(), 0), BUDGET);
        assert!(text.contains(&file.hunks[0].header), "{text}");
        let thread = review.threads.iter().find(|t| t.anchor.is_some()).unwrap();
        let text = scope_text(&Scope::Thread(thread.id.clone()), BUDGET);
        assert!(text.contains("## Thread") && text.contains(&thread.notes[0].author.username), "{text}");
    }

    #[test]
    fn prompts_read_as_the_spec_words_them() {
        assert!(Prompt::Comment("naming".into()).question().ends_with("about: naming"));
        assert!(Prompt::Summary.question().starts_with("Summarise this MR"));
        assert_eq!(Prompt::Free("why?".into()).question(), "why?");
    }
}
