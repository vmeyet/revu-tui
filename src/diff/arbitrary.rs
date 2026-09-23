//! Generated unified diffs for property tests: what the forges send, built from parts we know,
//! so a test can compare what `parse` finds with what went in.
use super::{Hunk, LineKind};
use proptest::prelude::*;

/// One generated hunk: where it starts, whether the header spells counts of one, its lines.
#[derive(Clone, Debug)]
pub struct Part {
    pub old_start: u32,
    pub new_start: u32,
    pub terse: bool,
    pub context: String,
    pub lines: Vec<(LineKind, String)>,
}

impl Part {
    pub fn old_count(&self) -> u32 {
        self.lines.iter().filter(|(kind, _)| *kind != LineKind::Added).count() as u32
    }

    pub fn new_count(&self) -> u32 {
        self.lines.iter().filter(|(kind, _)| *kind != LineKind::Removed).count() as u32
    }

    /// `@@ -a,b +c,d @@ context`, with the `,1` left out on a terse header the way git does.
    pub fn header(&self) -> String {
        let side = |start: u32, count: u32| if self.terse && count == 1 { start.to_string() } else { format!("{start},{count}") };
        let context = if self.context.is_empty() { String::new() } else { format!(" {}", self.context) };
        format!("@@ -{} +{} @@{context}", side(self.old_start, self.old_count()), side(self.new_start, self.new_count()))
    }
}

/// A whole generated diff: its hunks, and whether the last line has no newline at end of file.
#[derive(Clone, Debug)]
pub struct Generated {
    pub parts: Vec<Part>,
    pub no_newline: bool,
}

impl Generated {
    pub fn text(&self) -> String {
        let mut out = String::new();
        for part in &self.parts {
            out.push_str(&part.header());
            out.push('\n');
            for (kind, text) in &part.lines {
                out.push(sign(*kind));
                out.push_str(text);
                out.push('\n');
            }
        }
        if self.no_newline {
            out.push_str("\\ No newline at end of file\n");
        }
        out
    }
}

/// The unified text of parsed hunks, as the forges send it: the inverse of `parse`.
pub fn print(hunks: &[Hunk]) -> String {
    let mut out = String::new();
    for hunk in hunks {
        out.push_str(&hunk.header);
        out.push('\n');
        for line in &hunk.lines {
            out.push(sign(line.kind));
            out.push_str(&line.text);
            out.push('\n');
            if line.no_newline {
                out.push_str("\\ No newline at end of file\n");
            }
        }
    }
    out
}

fn sign(kind: LineKind) -> char {
    match kind {
        LineKind::Context => ' ',
        LineKind::Added => '+',
        LineKind::Removed => '-',
    }
}

/// Line text as code looks: ASCII, tabs, trailing spaces, accents and emoji; never a newline or a
/// carriage return, which the parser treats as line ends.
fn text() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9 \t(){};:=,.<>/@+\\-éü✓🦀]{0,24}"
}

fn kind() -> impl Strategy<Value = LineKind> {
    prop_oneof![3 => Just(LineKind::Context), 2 => Just(LineKind::Added), 2 => Just(LineKind::Removed)]
}

/// Up to four hunks, each starting past where the previous one ended on both sides.
pub fn diff() -> impl Strategy<Value = Generated> {
    let part = (0u32..40, any::<bool>(), "[a-z_ (){}]{0,16}", prop::collection::vec((kind(), text()), 1..30));
    (1u32..50, prop::collection::vec(part, 1..5), any::<bool>()).prop_map(|(first, parts, no_newline)| {
        let (mut old_at, mut new_at) = (first, first);
        let parts = parts
            .into_iter()
            .map(|(gap, terse, context, lines)| {
                let part = Part { old_start: old_at + gap, new_start: new_at + gap, terse, context, lines };
                old_at = part.old_start + part.old_count();
                new_at = part.new_start + part.new_count();
                part
            })
            .collect();
        Generated { parts, no_newline }
    })
}
