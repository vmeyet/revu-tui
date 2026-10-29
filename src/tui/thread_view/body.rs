//! A note's body as light markdown: code spans, links, bullets, quotes, tables, pictures and fenced code.
use super::{Piece, fallback};
use crate::diff::LineKind;
use crate::review::Review;
use crate::review::image;
use crate::syntax::{self, Language};
use crate::tui::diff_view::{coloured, paint};
use crate::tui::table::table_lines;
use crate::tui::theme::Theme;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use std::ops::Range;
use unicode_width::UnicodeWidthStr;

/// Code spans, bullets, quotes and tables `width` columns at most; the rest is the text as written,
/// wrapped by the widget. Pictures read as their `[image: …]` line, for the panes that never draw them.
pub(in crate::tui) fn body_lines<'a>(body: &str, width: usize, theme: Theme) -> Vec<Line<'a>> {
    body_pieces(body, &Fences::Plain, width, theme)
        .into_iter()
        .map(|piece| match piece {
            Piece::Text(line) | Piece::Written { line, .. } => line,
            Piece::Picture(image) => Line::from(Span::styled(fallback(&image), Style::default().fg(theme.link))),
        })
        .collect()
}

/// The lines a thread hangs on, which a suggestion in its notes replaces, and their file's language.
#[derive(Default)]
pub(super) struct Replaced {
    lines: Vec<String>,
    language: Option<&'static Language>,
}

impl Replaced {
    pub(super) fn at(review: &Review, position: Option<&crate::forge::Position>) -> Self {
        position.map_or_else(Self::default, |position| Self {
            lines: review.text_at(position),
            language: syntax::language_for(position.path(), &syntax::LANGUAGES),
        })
    }
}

/// How a body's fences read.
pub(super) enum Fences<'a> {
    /// In panes drawn each frame, where highlighting is too slow to repeat: code in `theme.code`,
    /// a suggestion as its `+` lines only. Nothing clicks there, so links stay written out.
    Plain,
    /// In the thread pane, whose rows are kept: code in syntax colours, a suggestion over what it
    /// replaces, and links drawn as their text, which clicks.
    Coloured(&'a Replaced),
}

/// A body as pieces, with a suggestion block drawn as a small diff (the replaced lines struck as
/// `-`, the suggested ones as `+`, in the diff colours) and each picture under its line.
pub(super) fn body_pieces(body: &str, fences: &Fences<'_>, width: usize, theme: Theme) -> Vec<Piece> {
    let links = match fences {
        Fences::Plain => Links::Literal,
        Fences::Coloured(_) => Links::Drawn,
    };
    let mut lines = vec![];
    let mut raws = body.lines().peekable();
    while let Some(raw) = raws.next() {
        let opener = raw.trim_start();
        if let Some(tag) = opener.strip_prefix("```") {
            let block: Vec<&str> = raws.by_ref().take_while(|line| !line.trim_start().starts_with("```")).collect();
            lines.extend(fence_pieces(tag, &block, fences, theme));
            continue;
        }
        if !opener.starts_with('|') {
            lines.extend(prose(raw, links, theme));
            continue;
        }
        let run: Vec<&str> = std::iter::once(raw).chain(std::iter::from_fn(|| raws.next_if(|l| l.trim_start().starts_with('|')))).collect();
        match table_lines(&run, width, theme) {
            Some(table) => lines.extend(table.into_iter().zip(&run).map(|(line, raw)| written(line, raw, vec![]))),
            None => lines.extend(run.iter().flat_map(|raw| prose(raw, links, theme))),
        }
    }
    lines
}

/// A fenced block in the syntax colours of the language its tag names, all in `theme.code` where none;
/// a suggestion in its file's language, over the diff's signs and fills.
fn fence_pieces(tag: &str, block: &[&str], fences: &Fences<'_>, theme: Theme) -> Vec<Piece> {
    let written_all = |lines: Vec<Line<'static>>| lines.into_iter().zip(block).map(|(line, raw)| written(line, raw, vec![]));
    let (replaced, file_language, named) = match fences {
        Fences::Plain => (&[][..], None, None),
        Fences::Coloured(replaced) => {
            let named = tag.split_whitespace().next().and_then(|name| syntax::language_named(name, &syntax::LANGUAGES));
            (replaced.lines.as_slice(), replaced.language, named)
        }
    };
    if tag.starts_with("suggestion") {
        let old = diff_lines(LineKind::Removed, replaced, file_language, theme).into_iter().map(Piece::Text);
        return old.chain(written_all(diff_lines(LineKind::Added, block, file_language, theme))).collect();
    }
    let base = if named.is_some() { Style::default() } else { Style::default().fg(theme.code) };
    written_all(code_lines(&Span::styled("  ", base), base, block, named, theme)).collect()
}

/// Lines of a suggestion with the sign and fill of a removed or added line in the diff.
fn diff_lines(kind: LineKind, lines: &[impl AsRef<str>], language: Option<&Language>, theme: Theme) -> Vec<Line<'static>> {
    let Some(paint) = paint(kind, theme) else { return vec![] };
    let sign = paint.sign_span();
    code_lines(&Span::styled(format!("{} ", sign.content), sign.style), paint.base(), lines, language, theme)
}

/// Each line after `sign`, its text in `base` where the language colours nothing.
fn code_lines(
    sign: &Span<'static>,
    base: Style,
    lines: &[impl AsRef<str>],
    language: Option<&Language>,
    theme: Theme,
) -> Vec<Line<'static>> {
    let texts: Vec<&str> = lines.iter().map(AsRef::as_ref).collect();
    let spans = language.map(|language| syntax::highlight(language, &texts.join("\n"))).unwrap_or_default();
    let line = |(index, text): (usize, &&str)| {
        let tokens = spans.get(index).map_or(&[][..], Vec::as_slice);
        let parts = coloured(text, 0, tokens, theme).into_iter().map(|(part, style)| Span::styled(part.to_owned(), base.patch(style)));
        Line::from(std::iter::once(sign.clone()).chain(parts).collect::<Vec<_>>())
    };
    texts.iter().enumerate().map(line).collect()
}

/// Whether a line draws its markdown links as their text or leaves them written out.
#[derive(Clone, Copy)]
enum Links {
    Literal,
    Drawn,
}

/// A line outside fences and tables, each picture under it.
fn prose(raw: &str, links: Links, theme: Theme) -> Vec<Piece> {
    let (text, pictures) = image::split(raw);
    let line = (pictures.is_empty() || !text.trim().is_empty()).then(|| {
        let (line, anchors) = text_line(&text, links, theme);
        written(line, raw, in_raw(anchors, raw))
    });
    line.into_iter().chain(pictures.into_iter().map(Piece::Picture)).collect()
}

/// `anchors` found in a drawing of `raw`, their markup moved to its bytes in `raw`: the drawing
/// took out pictures and turned a bullet into a glyph, but keeps its links in the same order.
fn in_raw(anchors: Vec<Anchor>, raw: &str) -> Vec<Anchor> {
    let markups = runs(raw, Links::Drawn).into_iter().filter_map(|run| match run {
        Run::Link { markup, .. } => Some(markup),
        Run::Plain(_) | Run::Code(_) => None,
    });
    anchors.into_iter().zip(markups).map(|(anchor, markup)| Anchor { markup, ..anchor }).collect()
}

fn written(line: Line<'static>, raw: &str, links: Vec<Anchor>) -> Piece {
    Piece::Written { line, raw: raw.to_owned(), links }
}

/// Where a markdown link points as written: a web address or a path on the forge's host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LinkUrl(String);

impl LinkUrl {
    /// A link revu can open: `http(s)://…` or an absolute `/path`; a relative path, an `#anchor`
    /// or a `mailto:` would open the wrong page, so they stay text.
    fn parse(url: &str) -> Option<Self> {
        let web = url.starts_with("http://") || url.starts_with("https://");
        let path = url.starts_with('/') && !url.starts_with("//");
        (web || path).then(|| Self(url.to_owned()))
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

/// A link drawn in a body line: the cells its text takes, the bytes of its line as written its
/// drawing drops (the `[` and the `](url)`), and where it points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Anchor {
    pub(super) cells: Range<usize>,
    pub(super) markup: [Range<usize>; 2],
    pub(super) url: LinkUrl,
}

/// One line of prose: bullets, quotes, code spans and links.
fn text_line(raw: &str, links: Links, theme: Theme) -> (Line<'static>, Vec<Anchor>) {
    match raw.trim_start() {
        rest if rest.starts_with("- ") || rest.starts_with("* ") => marked(&format!("• {}", &rest[2..]), links, theme),
        rest if rest.starts_with("> ") => (Line::from(Span::styled(format!("▏{}", &rest[2..]), Style::default().fg(theme.muted))), vec![]),
        _ => marked(raw, links, theme),
    }
}

/// `text` drawn with its code spans and links, and where each link sits.
fn marked(text: &str, links: Links, theme: Theme) -> (Line<'static>, Vec<Anchor>) {
    let runs = runs(text, links);
    let mut column = 0;
    let mut anchors = vec![];
    for run in &runs {
        let width = run.shown().width();
        if let Run::Link { url, markup, .. } = run {
            anchors.push(Anchor { cells: column..column + width, markup: markup.clone(), url: url.clone() });
        }
        column += width;
    }
    (Line::from(spans(&runs, theme)), anchors)
}

/// Code spans drawn, the rest as written, links too: the panes that call it cannot click them.
pub(in crate::tui) fn inline<'a>(text: &str, theme: Theme) -> Vec<Span<'a>> {
    spans(&runs(text, Links::Literal), theme)
}

fn spans<'a>(runs: &[Run<'_>], theme: Theme) -> Vec<Span<'a>> {
    let span = |run: &Run<'_>| match run {
        Run::Plain(text) => Span::raw((*text).to_owned()),
        Run::Code(code) => Span::styled((*code).to_owned(), Style::default().fg(theme.code).bg(theme.surface)),
        Run::Link { text, .. } => Span::styled((*text).to_owned(), Style::default().fg(theme.link)),
    };
    runs.iter().map(span).collect()
}

/// A stretch of a prose line: text as written, a code span, or a markdown link with the bytes of
/// the line its `[` and `](url)` take.
enum Run<'a> {
    Plain(&'a str),
    Code(&'a str),
    Link { text: &'a str, url: LinkUrl, markup: [Range<usize>; 2] },
}

impl Run<'_> {
    fn shown(&self) -> &str {
        match self {
            Self::Plain(text) | Self::Code(text) | Self::Link { text, .. } => text,
        }
    }
}

/// A line split on its backticks into code spans, and the text between them into its links where `links` draws them.
fn runs(text: &str, links: Links) -> Vec<Run<'_>> {
    let starts = text.split('`').scan(0, |at, part| {
        let start = *at;
        *at += part.len() + 1;
        Some(start)
    });
    let parts = text.split('`').zip(starts).enumerate().filter(|(_, (part, _))| !part.is_empty());
    parts
        .flat_map(|(i, (part, start))| match (i % 2 == 1, links) {
            (true, _) => vec![Run::Code(part)],
            (false, Links::Literal) => vec![Run::Plain(part)],
            (false, Links::Drawn) => link_runs(part, start),
        })
        .collect()
}

/// `text`, which starts at byte `start` of its line, as its markdown links `[text](url)` and what
/// lies between them; a picture's `![` is not a link.
fn link_runs(text: &str, start: usize) -> Vec<Run<'_>> {
    let mut runs = vec![];
    let mut at = 0;
    while let Some((open, link)) = next_link(&text[at..]) {
        let open = at + open;
        runs.extend((open > at).then(|| Run::Plain(&text[at..open])));
        let close = open + 1 + link.text.len();
        at = open + link.len;
        runs.push(Run::Link { text: link.text, url: link.url, markup: [start + open..start + open + 1, start + close..start + at] });
    }
    runs.extend((at < text.len()).then(|| Run::Plain(&text[at..])));
    runs
}

/// A markdown link as written: its text, where it points, and how many bytes it takes from its `[`.
struct LinkMarkdown<'a> {
    text: &'a str,
    url: LinkUrl,
    len: usize,
}

/// The first link in `text` and the byte its `[` sits at.
fn next_link(text: &str) -> Option<(usize, LinkMarkdown<'_>)> {
    let mut opens = text.match_indices('[').map(|(at, _)| at).filter(|&at| !text[..at].ends_with('!'));
    opens.find_map(|at| link_at(&text[at..]).map(|link| (at, link)))
}

/// The link `[text](url "title")` `text` starts with; the title is dropped. Empty text or a url
/// revu cannot open leaves it text.
fn link_at(text: &str) -> Option<LinkMarkdown<'_>> {
    let (shown, target) = text[1..].split_once(']')?;
    let inside = target.strip_prefix('(')?;
    let close = closing_paren(inside)?;
    let url = LinkUrl::parse(inside[..close].split_whitespace().next()?.trim_matches(['<', '>']))?;
    let len = 1 + shown.len() + 2 + close + 1;
    (!shown.is_empty()).then_some(LinkMarkdown { text: shown, url, len })
}

/// The byte of the `)` that closes `text`, past the parentheses opened and closed inside it.
fn closing_paren(text: &str) -> Option<usize> {
    let mut depth = 0_usize;
    for (at, c) in text.char_indices() {
        match c {
            '(' => depth += 1,
            ')' if depth == 0 => return Some(at),
            ')' => depth -= 1,
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::super::tests::text;
    use super::super::{Mark, written_rows};
    use super::*;
    use ratatui::widgets::Paragraph;

    /// Pieces as the lines a pane without pictures shows.
    fn texts(pieces: Vec<Piece>) -> Vec<Line<'static>> {
        pieces
            .into_iter()
            .map(|piece| match piece {
                Piece::Text(line) | Piece::Written { line, .. } => line,
                Piece::Picture(image) => Line::from(fallback(&image)),
            })
            .collect()
    }

    #[test]
    fn a_picture_leaves_its_line_and_sits_under_it() {
        let body = "Before:\n![the chart](/uploads/ab12/chart.png) broke\n<img alt=\"after\" src=\"https://x/a.png\">\n```\n![not](https://x/code.png)\n```";
        let pieces = body_pieces(body, &Fences::Plain, 80, Theme::default());
        let shape: Vec<String> = pieces
            .iter()
            .map(|p| match p {
                Piece::Text(line) | Piece::Written { line, .. } => {
                    format!("text {}", line.spans.iter().map(|s| s.content.to_string()).collect::<String>())
                }
                Piece::Picture(image) => format!("picture {}", image.url),
            })
            .collect();
        assert_eq!(
            shape,
            [
                "text Before:",
                "text  broke",
                "picture /uploads/ab12/chart.png",
                "picture https://x/a.png",
                "text   ![not](https://x/code.png)"
            ]
        );
    }

    #[test]
    fn bodies_keep_code_bullets_and_quotes() {
        let body = "Use `Key::from` here.\n- one\n> said\n```\nlet x = 1;\n```";
        let lines = text(&body_lines(body, 80, Theme::default()));
        assert_eq!(lines, ["Use Key::from here.", "• one", "▏said", "  let x = 1;"]);
    }

    /// A body as the thread pane draws it, where links click.
    fn in_thread(body: &str) -> Vec<Line<'static>> {
        texts(body_pieces(body, &Fences::Coloured(&Replaced::default()), 80, Theme::default()))
    }

    /// Each link a one-line body clicks through to as the thread pane lays it out `width` wide:
    /// its row, its column, the text it covers and its url.
    fn clicks(body: &str, width: usize) -> Vec<(usize, u16, String, String)> {
        let pieces = body_pieces(body, &Fences::Coloured(&Replaced::default()), 80, Theme::default());
        let [Piece::Written { line, raw, links }] = pieces.as_slice() else { panic!("one written line") };
        let rows = written_rows(line.clone(), raw, links, width, &str::to_owned);
        let marks = rows.into_iter().enumerate().flat_map(|(at, (_, marks))| marks.into_iter().map(move |mark| (at, mark)));
        marks
            .filter_map(|(at, mark)| match mark {
                Mark::Link { column, text, url } => Some((at, column, text, url)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_link_shows_its_text_in_the_link_colour() {
        let lines = in_thread("see [the docs](https://x/a \"title\") now");
        assert_eq!(text(&lines), ["see the docs now"]);
        assert_eq!(style_of(&lines[0], "the docs").fg, Some(Theme::default().link));
    }

    #[test]
    fn a_link_stays_written_out_in_a_pane_that_cannot_click_it() {
        assert_eq!(text(&body_lines("see [the docs](https://x/a) now", 80, Theme::default())), ["see [the docs](https://x/a) now"]);
    }

    #[test]
    fn a_link_inside_a_code_span_or_a_fence_stays_literal() {
        let lines = text(&in_thread("use `[a](https://x/a)` here\n```\n[b](https://x/b)\n```"));
        assert_eq!(lines, ["use [a](https://x/a) here", "  [b](https://x/b)"]);
    }

    #[test]
    fn a_link_clicks_through_on_each_row_its_text_is_drawn_on() {
        let guide = "https://x/guide".to_owned();
        assert_eq!(
            clicks("read [the guide](https://x/guide) first", 10),
            [(0, 5, "the ".into(), guide.clone()), (1, 0, "guide".into(), guide)]
        );
    }

    #[test]
    fn a_link_after_a_wide_emoji_clicks_on_its_own_text() {
        assert_eq!(clicks("⚠️ see [docs](https://x/a) now", 80), [(0, 7, "docs".into(), "https://x/a".into())]);
    }

    #[test]
    fn a_link_url_keeps_its_balanced_parentheses() {
        let body = "see [w](https://x/a_(b)) now";
        assert_eq!(text(&in_thread(body)), ["see w now"]);
        assert_eq!(clicks(body, 80), [(0, 4, "w".into(), "https://x/a_(b)".into())]);
    }

    #[test]
    fn only_a_web_address_or_an_absolute_path_makes_a_link() {
        let body = "[a](docs/x.md) [b](#top) [c](mailto:nina@x) [d](//x/y) [e](/acme/widgets)";
        assert_eq!(text(&in_thread(body)), ["[a](docs/x.md) [b](#top) [c](mailto:nina@x) [d](//x/y) e"]);
    }

    #[test]
    fn a_link_without_text_stays_literal() {
        assert_eq!(text(&in_thread("[](https://x/a) then")), ["[](https://x/a) then"]);
    }

    #[test]
    fn a_table_is_drawn_and_pipes_without_a_delimiter_row_stay_text() {
        let body = "Counts:\n| Name | Count |\n| --- | ---: |\n| alpha | 3 |\nthen\n| a | b |";
        let lines = text(&body_lines(body, 80, Theme::default()));
        assert_eq!(lines, ["Counts:", "Name  │ Count", "──────┼──────", "alpha │     3", "then", "| a | b |"]);
    }

    #[test]
    fn a_suggestion_reads_as_a_small_diff() {
        let body = "Try this:\n```suggestion:-0+0\nlet client = Client::default();\n```\nthanks";
        let replaced = Replaced { lines: vec!["let client = Client::new();".to_owned()], language: None };
        let lines = text(&texts(body_pieces(body, &Fences::Coloured(&replaced), 80, Theme::default())));
        assert_eq!(lines, ["Try this:", "- let client = Client::new();", "+ let client = Client::default();", "thanks"]);
        assert_eq!(text(&body_lines("```rust\nlet x = 1;\n```", 80, Theme::default())), ["  let x = 1;"], "other fences stay code");
    }

    fn tokyonight() -> Theme {
        Theme::named("tokyonight").unwrap()
    }

    /// The lines of a body in a pane whose rows are kept, its thread on `path`.
    fn kept_lines(body: &str, path: &str, replaced: &[&str]) -> Vec<Line<'static>> {
        let replaced = Replaced {
            lines: replaced.iter().map(|line| (*line).to_owned()).collect(),
            language: syntax::language_for(path, &syntax::LANGUAGES),
        };
        texts(body_pieces(body, &Fences::Coloured(&replaced), 80, tokyonight()))
    }

    fn style_of(line: &Line<'_>, content: &str) -> Style {
        line.spans.iter().find(|span| span.content == content).unwrap_or_else(|| panic!("no {content:?} in {line:?}")).style
    }

    #[test]
    fn a_typescript_fence_takes_the_diff_syntax_colours() {
        let lines = kept_lines("```ts\nconst total: number = 42; // cents\n```", "notes.txt", &[]);
        let syntax = tokyonight().syntax;
        assert_eq!(style_of(&lines[0], "const").fg, Some(syntax.keyword));
        assert_eq!(style_of(&lines[0], "42").fg, Some(syntax.number));
        assert_eq!(style_of(&lines[0], "// cents").fg, Some(syntax.comment));
        assert_eq!(style_of(&lines[0], " total: ").fg, None, "what the grammar leaves uncoloured reads as in the diff");
    }

    #[test]
    fn a_python_fence_is_highlighted_as_one_block() {
        let lines = kept_lines("```python\ndef charge(card):\n    return None  # later\n```", "notes.txt", &[]);
        let syntax = tokyonight().syntax;
        assert_eq!(style_of(&lines[0], "def").fg, Some(syntax.keyword));
        assert_eq!(style_of(&lines[0], "charge").fg, Some(syntax.function));
        assert_eq!(style_of(&lines[1], "return").fg, Some(syntax.keyword));
        assert_eq!(style_of(&lines[1], "# later").fg, Some(syntax.comment));
    }

    #[test]
    fn a_suggestion_takes_its_file_language_over_the_diff_signs_and_fills() {
        let theme = tokyonight();
        let lines = kept_lines("```suggestion:-0+0\nreturn card.total  # cents\n```", "billing/charge.py", &["return None"]);
        assert_eq!(text(&lines), ["- return None", "+ return card.total  # cents"]);
        let (sign, keyword) = (style_of(&lines[0], "- "), style_of(&lines[0], "return"));
        assert_eq!((sign.fg, sign.bg), (Some(theme.danger), theme.removed_fill));
        assert_eq!((keyword.fg, keyword.bg), (Some(theme.syntax.keyword), theme.removed_fill));
        let (sign, comment) = (style_of(&lines[1], "+ "), style_of(&lines[1], "# cents"));
        assert_eq!((sign.fg, sign.bg), (Some(theme.success), theme.added_fill));
        assert_eq!((comment.fg, comment.bg), (Some(theme.syntax.comment), theme.added_fill));
    }

    #[test]
    fn an_unknown_or_missing_tag_keeps_the_code_colour() {
        for body in ["```rust\nlet x = 1; // one\n```", "```\nconst x = 1;\n```"] {
            let lines = kept_lines(body, "src/app.ts", &[]);
            assert!(lines[0].spans.iter().all(|span| span.style.fg == Some(tokyonight().code)), "{body}: {lines:?}");
        }
        let plain = body_lines("```ts\nconst x = 1;\n```", 80, tokyonight());
        assert!(plain[0].spans.iter().all(|span| span.style.fg == Some(tokyonight().code)), "panes drawn each frame stay plain");
    }

    #[test]
    fn a_coloured_line_still_copies_as_written() {
        let pieces = body_pieces("```ts\n\tconst x = 1;\n```", &Fences::Coloured(&Replaced::default()), 80, tokyonight());
        let [Piece::Written { line, raw, .. }] = pieces.as_slice() else { panic!("one written line") };
        assert_eq!(raw, "\tconst x = 1;");
        let Some(Mark::Written { cells, .. }) = written_rows(line.clone(), raw, &[], 80, &str::to_owned).remove(0).1.pop() else {
            panic!("a written row")
        };
        assert_eq!(cells.last(), Some(&(12..13)), "the last cell copies the last byte");
    }

    #[test]
    fn snapshot_highlighted_comment() {
        let body = "Shorter:\n```ts\nconst total = items.length; // all\n```\n```suggestion:-0+0\nreturn total * 2\n```";
        let lines = kept_lines(body, "billing/charge.py", &["return total"]);
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 4)).unwrap();
        terminal.draw(|f| f.render_widget(Paragraph::new(lines), f.area())).unwrap();
        let buffer = terminal.backend().buffer();
        let runs: Vec<String> = (0..buffer.area.height)
            .flat_map(|y| {
                let cells: Vec<_> = (0..buffer.area.width).map(|x| &buffer[(x, y)]).collect();
                cells
                    .chunk_by(|a, b| (a.fg, a.bg) == (b.fg, b.bg))
                    .map(|run| format!("{:?}/{:?} {:?}", run[0].fg, run[0].bg, run.iter().map(|c| c.symbol()).collect::<String>()))
                    .chain(["---".to_owned()])
                    .collect::<Vec<_>>()
            })
            .collect();
        insta::assert_snapshot!("highlighted_comment", runs.join("\n"));
    }
}
