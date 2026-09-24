//! The user docs stay true and readable: reference pages match the code, links resolve, and the
//! prose follows the house style (one sentence per line, short sentences, no em dash, no filler).
#![allow(clippy::unwrap_used, clippy::expect_used)]
use assert_cmd::Command;
use std::path::{Path, PathBuf};

const MAX_WORDS: usize = 25;
const FILLER: [&str; 6] = ["simply", "just", "basically", "easily", "obviously", "of course"];

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// README.md and every Markdown file under docs/.
fn pages() -> Vec<PathBuf> {
    let mut found = vec![root().join("README.md")];
    let mut dirs = vec![root().join("docs")];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "md") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// The prose lines of a page, with their numbers: outside fenced blocks, tables, headings and comments.
fn prose(text: &str) -> Vec<(usize, String)> {
    let mut fenced = false;
    let mut lines = vec![];
    for (n, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced || trimmed.is_empty() || trimmed.starts_with('|') || trimmed.starts_with('#') || trimmed.starts_with("<!--") {
            continue;
        }
        lines.push((n + 1, plain(without_marker(trimmed))));
    }
    lines
}

/// A list item or a quote without its marker: `1. `, `- `, `> `.
fn without_marker(line: &str) -> &str {
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    let numbered = digits > 0 && line[digits..].starts_with(". ");
    if numbered {
        return &line[digits + 2..];
    }
    ["- ", "* ", "> "].iter().find_map(|m| line.strip_prefix(m)).unwrap_or(line)
}

/// The words a reader reads: inline code becomes one word, a link keeps its label.
fn plain(line: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    for c in line.chars() {
        if c == '`' {
            in_code = !in_code;
            if in_code {
                out.push_str("code");
            }
        } else if !in_code {
            out.push(c);
        }
    }
    let mut text = out;
    while let Some(start) = text.find("](") {
        let Some(end) = text[start..].find(')') else { break };
        text.replace_range(start + 1..=start + end, "");
    }
    text
}

/// Where a second sentence starts on the line: `. ` then a capital or a quote.
fn second_sentence(line: &str) -> Option<usize> {
    let bytes: Vec<char> = line.chars().collect();
    (1..bytes.len().saturating_sub(1))
        .find(|&i| matches!(bytes[i - 1], '.' | '?' | '!') && bytes[i] == ' ' && (bytes[i + 1].is_uppercase() || bytes[i + 1] == '"'))
}

fn words(line: &str) -> usize {
    line.split_whitespace().filter(|w| w.chars().any(char::is_alphanumeric)).count()
}

#[test]
fn the_reference_pages_match_the_code() {
    Command::cargo_bin("revu").unwrap().current_dir(root()).args(["docs", "--check"]).assert().success();
}

#[test]
fn the_docs_follow_the_house_style() {
    let mut faults = vec![];
    for page in pages() {
        let text = std::fs::read_to_string(&page).unwrap();
        let name = page.strip_prefix(root()).unwrap().display().to_string();
        if let Some(n) = text.lines().position(|l| l.contains('\u{2014}')) {
            faults.push(format!("{name}:{}: em dash", n + 1));
        }
        for (n, line) in prose(&text) {
            let lower = format!(" {} ", line.to_lowercase());
            if second_sentence(&line).is_some() {
                faults.push(format!("{name}:{n}: two sentences on one line"));
            }
            if words(&line) > MAX_WORDS {
                faults.push(format!("{name}:{n}: {} words, at most {MAX_WORDS}", words(&line)));
            }
            for filler in FILLER {
                if lower.contains(&format!(" {filler} ")) || lower.contains(&format!(" {filler},")) {
                    faults.push(format!("{name}:{n}: filler word `{filler}`"));
                }
            }
        }
    }
    assert!(faults.is_empty(), "\n{}", faults.join("\n"));
}

#[test]
fn every_relative_link_points_at_a_file() {
    let mut broken = vec![];
    for page in pages() {
        let text = std::fs::read_to_string(&page).unwrap();
        let mut rest = text.as_str();
        while let Some(start) = rest.find("](") {
            let after = &rest[start + 2..];
            let Some(end) = after.find(')') else { break };
            let target = after[..end].split('#').next().unwrap_or("");
            rest = &after[end..];
            let placeholder = target.starts_with('{');
            if target.is_empty() || placeholder || target.contains("://") || target.starts_with("mailto:") {
                continue;
            }
            if !page.parent().unwrap().join(target).exists() {
                broken.push(format!("{}: {target}", page.strip_prefix(root()).unwrap().display()));
            }
        }
    }
    assert!(broken.is_empty(), "\n{}", broken.join("\n"));
}

#[test]
fn the_style_checks_catch_what_they_should() {
    assert!(second_sentence("One idea. Two ideas.").is_some());
    assert!(second_sentence("Press `a.b` then go.").is_none());
    assert!(second_sentence(&plain("Run `revu docs`. It writes.")).is_some());
    assert_eq!(words(&plain("Open `revu list --json` and [the guide](a/b.md).")), 5);
    assert!(second_sentence(without_marker("1. Press `P`.")).is_none());
}
