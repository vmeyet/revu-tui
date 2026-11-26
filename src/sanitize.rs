//! Forge text made safe to show: a forge answers what anyone typed, and the terminal interprets what it is sent.
//! `forge::http` decodes every JSON answer through `json` and every error body through `text`, so the model holds no
//! character a terminal, a pager or a clipboard could be fooled by, and everything above it draws and copies it as is.
//! Code shown for review goes through `code` instead: a hidden character there is what the reviewer must see.
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::ops::Range;

const MARKER_OPEN: &str = "⟨U+";
const MARKER_CLOSE: &str = "⟩";

/// `raw` without the characters mrk removes (`mrk::sanitize`): escape sequences, C0 and C1 controls, DEL, bidi marks,
/// overrides and isolates, zero-width spaces, tag characters. Joiners stay, emoji and some scripts need them.
/// Tabs and newlines stay as written: the views lay code and Markdown out with them.
pub(crate) fn text(raw: &str) -> String {
    raw.split('\t').map(mrk::sanitize::lines).collect::<Vec<_>>().join("\t")
}

/// `raw` code with each character `text` would remove written as a marker, `⟨U+202E⟩`: a bidi override or a
/// zero-width space in reviewed code is the trojan-source trick (CVE-2021-42574), so it shows instead of vanishing.
/// Escapes and other controls show the same way, harmless once written as text; a CR ending a line is a line ending.
/// The marker is plain text on the line, so widths, copies and line numbers count it like any other text.
pub(crate) fn code(raw: &str) -> String {
    let mut shown = String::with_capacity(raw.len());
    let mut characters = raw.chars().peekable();
    while let Some(character) = characters.next() {
        let ends_line = character == '\r' && characters.peek().is_none_or(|next| *next == '\n');
        if ends_line {
            continue;
        }
        if is_hidden(character) {
            shown.push_str(&format!("{MARKER_OPEN}{:04X}{MARKER_CLOSE}", u32::from(character)));
        } else {
            shown.push(character);
        }
    }
    shown
}

/// Whether mrk's rules remove or replace `character`; tabs and newlines lay code out, so they stay.
fn is_hidden(character: char) -> bool {
    match character {
        '\t' | '\n' => false,
        _ if character.is_ascii() => character.is_ascii_control(),
        _ => {
            let mut buffer = [0; 4];
            let alone: &str = character.encode_utf8(&mut buffer);
            mrk::sanitize::text(alone) != alone
        }
    }
}

/// The byte ranges of `line` that `code` wrote as markers, in order, so a view can paint them as a warning.
pub(crate) fn markers(line: &str) -> Vec<Range<usize>> {
    line.match_indices(MARKER_OPEN)
        .filter_map(|(start, _)| {
            let digits = &line[start + MARKER_OPEN.len()..];
            let digits = &digits[..digits.find(MARKER_CLOSE)?];
            let is_code_point = (4..=6).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_lowercase());
            is_code_point.then(|| start..start + MARKER_OPEN.len() + digits.len() + MARKER_CLOSE.len())
        })
        .collect()
}

/// `bytes` decoded as JSON, every string in it through `text`, except the strings of the fields named `code_field`,
/// which hold code shown for review and go through `code`; keys are never shown, so they stay.
pub(crate) fn json<T: DeserializeOwned>(bytes: &[u8], code_field: Option<&str>) -> serde_json::Result<T> {
    serde_json::from_value(cleaned(serde_json::from_slice(bytes)?, code_field))
}

fn cleaned(value: Value, code_field: Option<&str>) -> Value {
    match value {
        Value::String(raw) => Value::String(text(&raw)),
        Value::Array(items) => Value::Array(items.into_iter().map(|item| cleaned(item, code_field)).collect()),
        Value::Object(fields) => Value::Object(fields.into_iter().map(|(key, value)| field(key, value, code_field)).collect()),
        other => other,
    }
}

fn field(key: String, value: Value, code_field: Option<&str>) -> (String, Value) {
    let value = match value {
        Value::String(raw) if code_field == Some(key.as_str()) => Value::String(code(&raw)),
        other => cleaned(other, code_field),
    };
    (key, value)
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
        let note: Note = json(raw, None).unwrap();
        assert_eq!(note, Note { id: 9_007_199_254_740_993, body: "ok]52;c;cHduZWQ=".into(), labels: vec!["bug".into()] });
    }

    #[test]
    fn code_shows_each_hidden_character_as_a_marker_where_it_was() {
        let cases = [
            ("if (admin\u{202e} \u{2066}// x\u{2069}) {", "if (admin⟨U+202E⟩ ⟨U+2066⟩// x⟨U+2069⟩) {"),
            ("le\u{200b}t a\u{feff} = 1;\u{2028}", "le⟨U+200B⟩t a⟨U+FEFF⟩ = 1;⟨U+2028⟩"),
            ("x\u{e0041}\u{200f}", "x⟨U+E0041⟩⟨U+200F⟩"),
            ("echo \x1b[2J\u{9b}\x07", "echo ⟨U+001B⟩[2J⟨U+009B⟩⟨U+0007⟩"),
            ("a\rb", "a⟨U+000D⟩b"),
        ];
        for (raw, shown) in cases {
            assert_eq!(code(raw), shown, "{raw:?}");
        }
    }

    #[test]
    fn code_keeps_tabs_newlines_joiners_and_crlf_line_endings_as_line_endings() {
        let raw = "fn main() {\n\tlet café = \"👩\u{200d}💻 日本語\";\n}\n";
        assert_eq!(code(raw), raw);
        assert_eq!(code("one\r\ntwo\r"), "one\ntwo");
    }

    #[test]
    fn code_run_twice_changes_nothing() {
        let once = code("a\u{202e}b\x1b");
        assert_eq!(code(&once), once);
        assert_eq!(text(&once), once, "a marker is plain text to `text` too");
    }

    #[test]
    fn markers_are_found_at_their_bytes_and_lookalikes_are_not() {
        let line = code("日\u{202e}x\u{e0041}");
        let found: Vec<&str> = markers(&line).into_iter().map(|range| &line[range]).collect();
        assert_eq!(found, ["⟨U+202E⟩", "⟨U+E0041⟩"]);
        assert_eq!(markers("⟨U+12⟩ ⟨U+12345678⟩ ⟨U+202e⟩ ⟨U+202E"), Vec::<Range<usize>>::new());
    }

    #[derive(serde::Deserialize, Debug, PartialEq)]
    struct File {
        path: String,
        diff: String,
    }

    #[test]
    fn json_marks_the_code_field_and_cleans_the_others() {
        let raw = br#"[{"path": "a\u202e.rs", "diff": "+if ok\u202e {\u001b"}]"#;
        let files: Vec<File> = json(raw, Some("diff")).unwrap();
        assert_eq!(files, [File { path: "a.rs".into(), diff: "+if ok⟨U+202E⟩ {⟨U+001B⟩".into() }]);
    }
}
