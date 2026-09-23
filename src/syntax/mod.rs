//! Syntax colours for diff lines, from tree-sitter grammars.
//!
//! Adding a language is one grammar crate in `Cargo.toml` and one entry in [`LANGUAGES`]:
//! its name, the file extensions it claims, and a function building its highlight configuration.
//! Grammars load on first use only, so startup never pays for a language no open file needs.
use std::ops::Range;
use std::sync::OnceLock;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

/// What a piece of code is, as far as colour goes; each theme gives every token a colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Keyword,
    String,
    Comment,
    Number,
    Type,
    Function,
    Constant,
    Punctuation,
}

/// The coloured byte ranges of one line of text.
pub type Spans = Vec<(Range<usize>, Token)>;

/// One language the diff can colour.
pub struct Language {
    #[cfg_attr(not(test), expect(dead_code, reason = "names the entry for readers and tests"))]
    pub name: &'static str,
    /// Lower-case file extensions, without the dot.
    pub extensions: &'static [&'static str],
    config: fn() -> Option<HighlightConfiguration>,
    /// `None` inside when the grammar's query does not build: the language then stays uncoloured.
    loaded: OnceLock<Option<HighlightConfiguration>>,
}

impl Language {
    const fn new(name: &'static str, extensions: &'static [&'static str], config: fn() -> Option<HighlightConfiguration>) -> Self {
        Self { name, extensions, config, loaded: OnceLock::new() }
    }

    /// The configuration, built the first time a file of this language is highlighted.
    fn configuration(&self) -> Option<&HighlightConfiguration> {
        self.loaded
            .get_or_init(|| {
                let mut config = (self.config)()?;
                config.configure(&CAPTURES.map(|(name, _)| name));
                Some(config)
            })
            .as_ref()
    }
}

/// Every language revu colours. Order matters only when two claim one extension: the first wins.
pub static LANGUAGES: [Language; 6] = [
    Language::new("TypeScript", &["ts", "mts", "cts"], typescript),
    Language::new("TSX", &["tsx"], tsx),
    Language::new("JavaScript", &["js", "mjs", "cjs", "jsx"], javascript),
    Language::new("Python", &["py", "pyi"], python),
    Language::new("JSON", &["json", "jsonc"], json),
    Language::new("SQL", &["sql"], sql),
];

/// The language of a path, by its extension; nothing is loaded by asking.
pub fn language_for(path: &str, languages: &'static [Language]) -> Option<&'static Language> {
    let (_, extension) = path.rsplit_once('.')?;
    let extension = extension.to_ascii_lowercase();
    languages.iter().find(|language| language.extensions.contains(&extension.as_str()))
}

/// The spans of each line of `source`, split on `\n`; one entry per line, empty for uncoloured ones.
/// A grammar that fails to parse leaves every line uncoloured rather than failing the diff.
pub fn highlight(language: &Language, source: &str) -> Vec<Spans> {
    let starts: Vec<usize> = std::iter::once(0).chain(source.match_indices('\n').map(|(i, _)| i + 1)).collect();
    let mut lines: Vec<Spans> = vec![Vec::new(); starts.len()];
    let Some(config) = language.configuration() else { return lines };
    let mut highlighter = Highlighter::new();
    let Ok(events) = highlighter.highlight(config, source.as_bytes(), None, None, |_| None) else { return lines };
    let mut stack: Vec<Token> = Vec::new();
    for event in events {
        match event {
            Ok(HighlightEvent::HighlightStart(capture)) => stack.push(CAPTURES[capture.0].1),
            Ok(HighlightEvent::HighlightEnd) => {
                stack.pop();
            }
            Ok(HighlightEvent::Source { start, end }) => {
                if let Some(&token) = stack.last() {
                    push_span(&mut lines, &starts, start..end, token);
                }
            }
            Err(_) => return vec![Vec::new(); starts.len()],
        }
    }
    lines
}

/// Splits a source range across the lines it spans, as byte ranges within each line.
fn push_span(lines: &mut [Spans], starts: &[usize], range: Range<usize>, token: Token) {
    let first = starts.partition_point(|&s| s <= range.start).saturating_sub(1);
    for (index, &line_start) in starts.iter().enumerate().skip(first) {
        if line_start >= range.end {
            break;
        }
        let line_end = starts.get(index + 1).map_or(usize::MAX, |next| next - 1);
        let start = range.start.max(line_start);
        let end = range.end.min(line_end);
        if start < end {
            lines[index].push((start - line_start..end - line_start, token));
        }
    }
}

/// The capture names the grammars' queries use, most specific first, and the token each paints.
/// tree-sitter matches a capture to the longest listed name that prefixes it, so `string` also
/// covers `string.special`.
const CAPTURES: [(&str, Token); 21] = [
    ("keyword", Token::Keyword),
    ("conditional", Token::Keyword),
    ("repeat", Token::Keyword),
    ("include", Token::Keyword),
    ("string", Token::String),
    ("escape", Token::String),
    ("comment", Token::Comment),
    ("number", Token::Number),
    ("float", Token::Number),
    ("boolean", Token::Constant),
    ("constant", Token::Constant),
    ("type", Token::Type),
    ("constructor", Token::Type),
    ("attribute", Token::Type),
    ("function", Token::Function),
    ("method", Token::Function),
    ("variable.builtin", Token::Constant),
    ("property", Token::Function),
    ("operator", Token::Punctuation),
    ("punctuation", Token::Punctuation),
    ("tag", Token::Keyword),
];

fn build(language: tree_sitter::Language, name: &str, highlights: &str, locals: &str) -> Option<HighlightConfiguration> {
    HighlightConfiguration::new(language, name, highlights, "", locals).ok()
}

fn typescript() -> Option<HighlightConfiguration> {
    let highlights = format!("{}\n{}", tree_sitter_typescript::HIGHLIGHTS_QUERY, tree_sitter_javascript::HIGHLIGHT_QUERY);
    let locals = format!("{}\n{}", tree_sitter_typescript::LOCALS_QUERY, tree_sitter_javascript::LOCALS_QUERY);
    build(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(), "typescript", &highlights, &locals)
}

fn tsx() -> Option<HighlightConfiguration> {
    let highlights = format!(
        "{}\n{}\n{}",
        tree_sitter_typescript::HIGHLIGHTS_QUERY,
        tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
        tree_sitter_javascript::HIGHLIGHT_QUERY
    );
    let locals = format!("{}\n{}", tree_sitter_typescript::LOCALS_QUERY, tree_sitter_javascript::LOCALS_QUERY);
    build(tree_sitter_typescript::LANGUAGE_TSX.into(), "tsx", &highlights, &locals)
}

fn javascript() -> Option<HighlightConfiguration> {
    let highlights = format!("{}\n{}", tree_sitter_javascript::JSX_HIGHLIGHT_QUERY, tree_sitter_javascript::HIGHLIGHT_QUERY);
    build(tree_sitter_javascript::LANGUAGE.into(), "javascript", &highlights, tree_sitter_javascript::LOCALS_QUERY)
}

fn python() -> Option<HighlightConfiguration> {
    build(tree_sitter_python::LANGUAGE.into(), "python", tree_sitter_python::HIGHLIGHTS_QUERY, "")
}

fn json() -> Option<HighlightConfiguration> {
    build(tree_sitter_json::LANGUAGE.into(), "json", tree_sitter_json::HIGHLIGHTS_QUERY, "")
}

/// The grammar's query is written for Neovim: its number patterns use Lua's `%d`, which the Rust
/// highlighter reads as a literal `%`, so every literal stayed a string; comments also carry a
/// Neovim-only `@spell`. Here a literal is a string only when quoted, a number when it is digits.
fn sql() -> Option<HighlightConfiguration> {
    const LITERALS: &str = r#"((literal) @string (#match? @string "^['\"]"))
((literal) @number (#match? @number "^[-+]?[0-9]*[.]?[0-9]+$"))
"#;
    let upstream = tree_sitter_sequel::HIGHLIGHTS_QUERY.replace("@comment @spell", "@comment");
    let kept: Vec<String> = upstream
        .split("\n\n")
        .filter(|block| !block.contains("%d"))
        .map(|block| block.lines().filter(|line| line.trim() != "(literal) @string").collect::<Vec<_>>().join("\n"))
        .collect();
    build(tree_sitter_sequel::LANGUAGE.into(), "sql", &format!("{LITERALS}{}", kept.join("\n\n")), "")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn tokens_of<'a>(line: &'a str, spans: &Spans) -> Vec<(&'a str, Token)> {
        spans.iter().map(|(range, token)| (&line[range.clone()], *token)).collect()
    }

    fn by_name(name: &str) -> &'static Language {
        LANGUAGES.iter().find(|l| l.name == name).unwrap()
    }

    #[test]
    fn languages_are_found_by_extension_whatever_the_case() {
        let cases = [
            ("src/app.ts", Some("TypeScript")),
            ("src/App.TSX", Some("TSX")),
            ("web/index.mjs", Some("JavaScript")),
            ("tools/build.py", Some("Python")),
            ("package.json", Some("JSON")),
            ("src/main.rs", None),
            ("Makefile", None),
            ("archive.tar.gz", None),
        ];
        for (path, name) in cases {
            assert_eq!(language_for(path, &LANGUAGES).map(|l| l.name), name, "{path}");
        }
    }

    #[test]
    fn typescript_keywords_strings_and_comments_land_on_their_bytes() {
        let source = "const answer: number = 42; // the answer\nconst name = \"nina\";";
        let lines = highlight(by_name("TypeScript"), source);
        let first = tokens_of(source.lines().next().unwrap(), &lines[0]);
        assert!(first.contains(&("const", Token::Keyword)), "{first:?}");
        assert!(first.contains(&("number", Token::Type)), "{first:?}");
        assert!(first.contains(&("42", Token::Number)), "{first:?}");
        assert!(first.contains(&("// the answer", Token::Comment)), "{first:?}");
        let second = tokens_of(source.lines().nth(1).unwrap(), &lines[1]);
        assert!(second.contains(&("\"nina\"", Token::String)), "{second:?}");
    }

    #[test]
    fn sql_colours_keywords_strings_numbers_and_comments() {
        let sql = "SELECT id, count(*) FROM orders WHERE status = 'paid' AND total > 10; -- recent\n";
        let tokens = tokens_of(sql.lines().next().unwrap(), &highlight(by_name("SQL"), sql)[0]);
        assert!(tokens.contains(&("SELECT", Token::Keyword)) && tokens.contains(&("WHERE", Token::Keyword)), "{tokens:?}");
        assert!(tokens.contains(&("'paid'", Token::String)) && tokens.contains(&("10", Token::Number)), "{tokens:?}");
        assert!(tokens.contains(&("-- recent", Token::Comment)), "{tokens:?}");
        assert_eq!(language_for("db/migrations/0042_orders.SQL", &LANGUAGES).map(|l| l.name), Some("SQL"));
    }

    #[test]
    fn python_and_json_colour_their_own_tokens() {
        let py = "def charge(card):\n    return None  # later\n";
        let lines = highlight(by_name("Python"), py);
        let first = tokens_of(py.lines().next().unwrap(), &lines[0]);
        assert!(first.contains(&("def", Token::Keyword)) && first.contains(&("charge", Token::Function)), "{first:?}");
        let second = tokens_of(py.lines().nth(1).unwrap(), &lines[1]);
        assert!(second.contains(&("return", Token::Keyword)) && second.contains(&("# later", Token::Comment)), "{second:?}");
        let json = "{\"name\": \"revu\", \"stars\": 3, \"public\": false}";
        let tokens = tokens_of(json, &highlight(by_name("JSON"), json)[0]);
        assert!(tokens.contains(&("\"revu\"", Token::String)), "{tokens:?}");
        assert!(tokens.contains(&("3", Token::Number)) && tokens.contains(&("false", Token::Constant)), "{tokens:?}");
    }

    #[test]
    fn a_hunk_starting_mid_function_still_colours() {
        let fragment = "    const total = items.reduce((sum, item) => sum + item.price, 0);\n    return total;\n}";
        let lines = highlight(by_name("TypeScript"), fragment);
        assert_eq!(lines.len(), 3);
        let first = tokens_of(fragment.lines().next().unwrap(), &lines[0]);
        assert!(first.contains(&("const", Token::Keyword)), "{first:?}");
        let second = tokens_of(fragment.lines().nth(1).unwrap(), &lines[1]);
        assert!(second.contains(&("return", Token::Keyword)), "{second:?}");
    }

    #[test]
    fn a_token_across_lines_is_split_per_line() {
        let source = "x = \"\"\"one\ntwo\"\"\"\n";
        let lines = highlight(by_name("Python"), source);
        assert!(tokens_of("two\"\"\"", &lines[1]).contains(&("two\"\"\"", Token::String)), "{:?}", lines[1]);
    }

    #[test]
    fn every_grammar_query_builds() {
        for language in &LANGUAGES {
            assert!((language.config)().is_some(), "{} query does not build", language.name);
        }
    }

    #[test]
    fn looking_up_a_language_loads_no_grammar() {
        fn never() -> Option<HighlightConfiguration> {
            panic!("a lookup must not build the grammar")
        }
        static ONLY: [Language; 1] = [Language::new("Never", &["never"], never)];
        assert_eq!(language_for("a.never", &ONLY).map(|l| l.name), Some("Never"));
        assert_eq!(language_for("a.rs", &ONLY).map(|l| l.name), None);
        assert!(ONLY[0].loaded.get().is_none());
    }
}
