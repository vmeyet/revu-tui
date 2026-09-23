//! The ```suggestion``` block both forges turn into an "apply" button: written prefilled with the
//! selected lines, read back from a note, and applied to a file.
use anyhow::{Result, bail};

/// `lines` are the selected diff lines as shown, sign included; the block replaces them all.
pub fn prefill(lines: &[&str]) -> String {
    let below = lines.len().saturating_sub(1);
    let body: Vec<&str> = lines.iter().map(|l| strip_sign(l)).collect();
    format!("```suggestion:-0+{below}\n{}\n```\n", body.join("\n"))
}

/// A suggestion read from a note: how many lines above and below the anchored one it replaces
/// (`suggestion:-A+B`, both 0 when absent), and the text that takes their place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proposal {
    pub above: u32,
    pub below: u32,
    pub text: String,
}

/// The first suggestion block of `body`, if it has one.
pub fn parse(body: &str) -> Option<Proposal> {
    let mut lines = body.lines();
    let opener = lines.by_ref().find(|l| l.trim_start().starts_with("```suggestion"))?;
    let (above, below) = span(opener.trim_start().trim_start_matches("```suggestion"));
    let text: Vec<&str> = lines.take_while(|l| l.trim_start() != "```").collect();
    Some(Proposal { above, below, text: text.join("\n") })
}

/// `:-2+1` → (2, 1); anything else reads as the anchored line alone.
fn span(rest: &str) -> (u32, u32) {
    let Some(rest) = rest.strip_prefix(":-") else { return (0, 0) };
    let Some((above, below)) = rest.split_once('+') else { return (0, 0) };
    (above.trim().parse().unwrap_or(0), below.trim().parse().unwrap_or(0))
}

/// `file` with the lines around `line` (1-based) replaced by the proposal, its final newline kept.
pub fn apply_to(file: &str, line: u32, proposal: &Proposal) -> Result<String> {
    let lines: Vec<&str> = file.lines().collect();
    let Some(first) = line.checked_sub(proposal.above).filter(|&first| first >= 1) else { bail!("the suggestion starts above the file") };
    let last = line + proposal.below;
    if last as usize > lines.len() {
        bail!("the suggestion ends below the file: the lines moved since it was written");
    }
    let (first, last) = (first as usize - 1, last as usize);
    let replacement: Vec<&str> = if proposal.text.is_empty() { vec![] } else { proposal.text.lines().collect() };
    let mut out: Vec<&str> = lines[..first].to_vec();
    out.extend(replacement);
    out.extend(&lines[last..]);
    let ending = if file.ends_with('\n') { "\n" } else { "" };
    Ok(format!("{}{ending}", out.join("\n")))
}

fn strip_sign(line: &str) -> &str {
    line.strip_prefix(['+', '-', ' ']).unwrap_or(line)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn one_line_replaces_itself() {
        assert_eq!(prefill(&["+    let attempt = Attempt::first();"]), "```suggestion:-0+0\n    let attempt = Attempt::first();\n```\n");
    }

    #[test]
    fn a_range_extends_below_by_its_length_minus_one() {
        let block = prefill(&[
            "-    let client = Client::new();",
            "+    let client = Client::with_key(idempotency_key(card));",
            "     audit::record(&response);",
        ]);
        assert!(block.starts_with("```suggestion:-0+2\n"), "{block}");
        assert!(block.contains("\n    let client = Client::new();\n    let client = Client::with_key"), "{block}");
        assert!(block.contains("\n    audit::record(&response);\n```\n"), "{block}");
    }

    #[test]
    fn unsigned_text_is_kept_as_is() {
        assert_eq!(prefill(&["plain"]), "```suggestion:-0+0\nplain\n```\n");
        assert_eq!(prefill(&[]), "```suggestion:-0+0\n\n```\n");
    }

    #[test]
    fn a_note_gives_back_its_suggestion_and_span() {
        let body = "Nit:\n```suggestion:-1+2\nlet b = 21;\nlet c = 3;\n```\nthanks";
        assert_eq!(parse(body), Some(Proposal { above: 1, below: 2, text: "let b = 21;\nlet c = 3;".into() }));
        assert_eq!(parse("```suggestion\nx\n```").map(|p| (p.above, p.below)), Some((0, 0)));
        assert_eq!(parse("```suggestion:-0+0\n```").map(|p| p.text), Some(String::new()), "an empty block deletes the line");
        assert_eq!(parse("```rust\nx\n```"), None);
    }

    #[test]
    fn applying_replaces_the_span_and_keeps_the_rest() {
        let file = "a\nb\nc\nd\n";
        let one = Proposal { above: 0, below: 0, text: "B".into() };
        assert_eq!(apply_to(file, 2, &one).unwrap(), "a\nB\nc\nd\n");
        let wide = Proposal { above: 1, below: 1, text: "X".into() };
        assert_eq!(apply_to(file, 2, &wide).unwrap(), "X\nd\n");
        let gone = Proposal { above: 0, below: 0, text: String::new() };
        assert_eq!(apply_to("a\nb", 2, &gone).unwrap(), "a");
        assert!(apply_to(file, 4, &Proposal { above: 0, below: 3, text: "z".into() }).is_err());
        assert!(apply_to(file, 1, &Proposal { above: 1, below: 0, text: "z".into() }).is_err());
    }
}
