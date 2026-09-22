//! The ```suggestion``` block GitLab turns into an "apply" button, prefilled with the selected lines.

/// `lines` are the selected diff lines as shown, sign included; the block replaces them all.
pub fn prefill(lines: &[&str]) -> String {
    let below = lines.len().saturating_sub(1);
    let body: Vec<&str> = lines.iter().map(|l| strip_sign(l)).collect();
    format!("```suggestion:-0+{below}\n{}\n```\n", body.join("\n"))
}

fn strip_sign(line: &str) -> &str {
    line.strip_prefix(['+', '-', ' ']).unwrap_or(line)
}

#[cfg(test)]
mod tests {
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
}
