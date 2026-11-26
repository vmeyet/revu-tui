//! Forge text made safe to show: a forge answers what anyone typed, and the terminal interprets what it is sent.
//! `forge::http` decodes every JSON answer through `json` and every error body through `text`, so the model holds no
//! character a terminal, a pager or a clipboard could be fooled by, and everything above it draws and copies it as is.
use serde::de::DeserializeOwned;
use serde_json::Value;

/// `raw` without the characters mrk removes (`mrk::sanitize`): escape sequences, C0 and C1 controls, DEL, bidi marks,
/// overrides and isolates, zero-width spaces, tag characters. Joiners stay, emoji and some scripts need them.
/// Tabs and newlines stay as written: the views lay code and Markdown out with them.
pub(crate) fn text(raw: &str) -> String {
    raw.split('\t').map(mrk::sanitize::lines).collect::<Vec<_>>().join("\t")
}

/// `bytes` decoded as JSON, every string in it through `text`; keys are never shown, so they stay.
pub(crate) fn json<T: DeserializeOwned>(bytes: &[u8]) -> serde_json::Result<T> {
    serde_json::from_value(cleaned(serde_json::from_slice(bytes)?))
}

fn cleaned(value: Value) -> Value {
    match value {
        Value::String(raw) => Value::String(text(&raw)),
        Value::Array(items) => Value::Array(items.into_iter().map(cleaned).collect()),
        Value::Object(fields) => Value::Object(fields.into_iter().map(|(key, value)| (key, cleaned(value))).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_characters_are_removed_and_the_words_kept() {
        let cases = [
            ("fix\x1b[31m the\x1b[0m bug", "fix[31m the[0m bug"),
            ("fix \x1b]8;;https://evil.example\x1b\\the bug\x1b]8;;\x1b\\", "fix ]8;;https://evil.example\\the bug]8;;\\"),
            ("fix\u{9b}31m the bug\u{85}", "fix31m the bug"),
            ("fix the \u{202e}gub\u{202c}", "fix the gub"),
            ("fix \u{2066}the\u{2067} \u{2068}bug\u{2069}", "fix the bug"),
            ("fi\u{200b}x th\u{2060}e b\u{feff}ug", "fix the bug"),
            ("fix the bug\x07", "fix the bug"),
            ("fix the bug\r", "fix the bug"),
            ("fix the bug\u{e0041}\u{e0042}", "fix the bug"),
        ];
        for (raw, clean) in cases {
            assert_eq!(text(raw), clean, "{raw:?}");
        }
    }

    #[test]
    fn tabs_newlines_emoji_and_other_scripts_stay() {
        let raw = "fn main() {\n\tlet café = \"👩‍💻 日本語 🇫🇷\";\n}\n";
        assert_eq!(text(raw), raw);
    }

    #[test]
    fn a_crlf_body_reads_as_lf() {
        assert_eq!(text("first\r\nsecond"), "first\nsecond");
    }

    #[derive(serde::Deserialize, Debug, PartialEq)]
    struct Note {
        id: u64,
        body: String,
        labels: Vec<String>,
    }

    #[test]
    fn json_cleans_every_string_and_keeps_numbers() {
        let raw = br#"{"id": 9007199254740993, "body": "ok\u001b]52;c;cHduZWQ=\u0007\u202e", "labels": ["bu\u200bg"]}"#;
        let note: Note = json(raw).unwrap();
        assert_eq!(note, Note { id: 9_007_199_254_740_993, body: "ok]52;c;cHduZWQ=".into(), labels: vec!["bug".into()] });
    }
}
