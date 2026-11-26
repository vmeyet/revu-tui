//! Forge text made safe to show: a forge answers what anyone typed, and the terminal interprets what it is sent.
//! `forge::http` decodes every JSON answer through `json` and every error body through `text`, so the model holds no
//! character a terminal, a pager or a clipboard could be fooled by, and everything above it draws and copies it as is.
//! Code shown for review goes through `code` instead: a hidden character there is what the reviewer must see.
//! What revu sends goes through `outgoing`, so a suggestion reaches the forge as the code it stands for.
use crate::review::suggestion;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::ops::Range;

const MARKER_OPEN: &str = "⟨U+";
const MARKER_CLOSE: &str = "⟩";
const MARKER_START: char = '⟨';

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
/// A `⟨` that starts what reads as a marker is written as one too, so every marker on a line is a real one and
/// `unmarked` gives the code back.
pub(crate) fn code(raw: &str) -> String {
    if !raw.contains(['\r', MARKER_START]) && text(raw) == raw {
        return raw.to_owned();
    }
    let mut shown = String::with_capacity(raw.len());
    for (at, character) in raw.char_indices() {
        let rest = &raw[at..];
        let ends_line = character == '\r' && matches!(rest.as_bytes().get(1), None | Some(b'\n'));
        let is_marked = is_hidden(character) || (character == MARKER_START && marker_length(rest).is_some());
        match (ends_line, is_marked) {
            (true, _) => {}
            (false, true) => shown.push_str(&format!("{MARKER_OPEN}{:04X}{MARKER_CLOSE}", u32::from(character))),
            (false, false) => shown.push(character),
        }
    }
    shown
}

/// `shown` code with each marker `code` wrote back to the character it stands for.
pub(crate) fn unmarked(shown: &str) -> String {
    let mut raw = String::with_capacity(shown.len());
    let mut at = 0;
    for marker in markers(shown) {
        raw.push_str(&shown[at..marker.start]);
        match marked_character(&shown[marker.clone()]) {
            Some(character) => raw.push(character),
            None => raw.push_str(&shown[marker.clone()]),
        }
        at = marker.end;
    }
    raw.push_str(&shown[at..]);
    raw
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
    line.match_indices(MARKER_OPEN).filter_map(|(start, _)| marker_length(&line[start..]).map(|length| start..start + length)).collect()
}

/// The length in bytes of the marker `text` starts with: four to six upper-case hex digits between `⟨U+` and `⟩`.
fn marker_length(text: &str) -> Option<usize> {
    let after_open = text.strip_prefix(MARKER_OPEN)?;
    let digits = after_open.bytes().take_while(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(b)).count();
    let is_marker = (4..=6).contains(&digits) && after_open[digits..].starts_with(MARKER_CLOSE);
    is_marker.then(|| MARKER_OPEN.len() + digits + MARKER_CLOSE.len())
}

/// The character a marker stands for, when `code` would have written it.
fn marked_character(marker: &str) -> Option<char> {
    let digits = marker.strip_prefix(MARKER_OPEN)?.strip_suffix(MARKER_CLOSE)?;
    let character = char::from_u32(u32::from_str_radix(digits, 16).ok()?)?;
    (is_hidden(character) || character == MARKER_START).then_some(character)
}

/// `bytes` decoded as JSON, every string in it through `text`, except code shown for review, which goes through
/// `code`: the strings of the fields named `code_field`, and the lines of a suggestion block in any string, which the
/// forge commits as written. Keys are never shown, so they stay.
pub(crate) fn json<T: DeserializeOwned>(bytes: &[u8], code_field: Option<&str>) -> serde_json::Result<T> {
    let clean = |field: Option<&str>, raw: &str| match field {
        Some(field) if code_field == Some(field) => code(raw),
        _ => suggestion::rewrite(raw, code, text),
    };
    serde_json::from_value(strings_changed(serde_json::from_slice(bytes)?, None, &clean))
}

/// `body` as the forge must get it: in a suggestion block, each marker back to its character, since the forge
/// commits the block as written.
pub(crate) fn outgoing(body: &Value) -> Value {
    strings_changed(body.clone(), None, &|_, shown| suggestion::rewrite(shown, unmarked, str::to_owned))
}

/// `value` with each string through `change`, given the name of the field holding it, if any.
fn strings_changed(value: Value, field: Option<&str>, change: &impl Fn(Option<&str>, &str) -> String) -> Value {
    match value {
        Value::String(raw) => Value::String(change(field, &raw)),
        Value::Array(items) => Value::Array(items.into_iter().map(|item| strings_changed(item, None, change)).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(key, value)| {
                    let value = strings_changed(value, Some(&key), change);
                    (key, value)
                })
                .collect(),
        ),
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
    fn unmarked_gives_back_the_code_a_marker_lookalike_included() {
        let raw = "if admin\u{202e} {\x1b \"⟨U+202E⟩\" ⟨U+12⟩ ⟨\n";
        let shown = code(raw);
        assert_eq!(shown, "if admin⟨U+202E⟩ {⟨U+001B⟩ \"⟨U+27E8⟩U+202E⟩\" ⟨U+12⟩ ⟨\n", "a written marker would read as a hidden character");
        assert_eq!(unmarked(&shown), raw);
        assert_eq!(unmarked("⟨U+0041⟩"), "⟨U+0041⟩", "only what `code` writes is read back");
    }

    #[test]
    fn a_marker_is_plain_text_to_text() {
        let shown = code("a\u{202e}b\x1b");
        assert_eq!(text(&shown), shown);
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

    #[test]
    fn json_marks_the_lines_of_a_suggestion_block_the_forge_would_commit() {
        let raw = br#"{"id": 1, "labels": [], "body": "Nit\u202e:\r\n```suggestion:-0+0\r\nif ok\u202e {\r\n```\r\nthen\u202e"}"#;
        let note: Note = json(raw, None).unwrap();
        assert_eq!(note.body, "Nit:\n```suggestion:-0+0\nif ok⟨U+202E⟩ {\n```\nthen");
    }

    #[test]
    fn outgoing_gives_a_suggestion_block_back_its_characters_and_leaves_the_rest() {
        let shown = "keep ⟨U+202E⟩\n```suggestion:-0+0\nif ok⟨U+202E⟩ {\n```\nkeep ⟨U+202E⟩";
        let body = serde_json::json!({"body": shown, "position": {"line": 3}});
        let sent = outgoing(&body);
        assert_eq!(sent["body"], "keep ⟨U+202E⟩\n```suggestion:-0+0\nif ok\u{202e} {\n```\nkeep ⟨U+202E⟩");
        assert_eq!(sent["position"], body["position"]);
    }
}
