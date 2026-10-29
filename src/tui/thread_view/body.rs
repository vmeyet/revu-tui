//! A note's body: revu draws its fenced code, suggestions and pictures, mrk the Markdown between them.
use super::{Piece, fallback};
use crate::diff::LineKind;
use crate::review::Review;
use crate::review::image::{self, Image};
use crate::syntax::{self, Language};
use crate::tui::diff_view::{coloured, paint};
use crate::tui::theme::Theme;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use std::ops::Range;

/// The body `width` columns wide, its code fences in the code colour. Pictures read as their `[image: …]` line, for
/// the panes that never draw them.
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

/// How a body's fences and links read.
pub(super) enum Fences<'a> {
    /// In panes drawn each frame, where highlighting is too slow to repeat: code in `theme.code`,
    /// a suggestion as its `+` lines only. Nothing clicks there, so a link's target follows its text.
    Plain,
    /// In the thread pane, whose rows are kept: code in syntax colours, a suggestion over what it
    /// replaces, and links drawn as their text, which clicks.
    Coloured(&'a Replaced),
}

/// A stretch of a body: Markdown for mrk, or what revu draws itself, a fenced block or a picture.
enum Segment<'a> {
    Markdown(Vec<String>),
    Fence { tag: &'a str, lines: Vec<&'a str> },
    Picture(Image),
}

/// A body as pieces, with a suggestion block drawn as a small diff (the replaced lines struck as
/// `-`, the suggested ones as `+`, in the diff colours) and each picture under its line.
pub(super) fn body_pieces(body: &str, fences: &Fences<'_>, width: usize, theme: Theme) -> Vec<Piece> {
    segments(body)
        .into_iter()
        .flat_map(|segment| match segment {
            Segment::Markdown(lines) => markdown(&lines, width, fences, theme),
            Segment::Fence { tag, lines } => fence_pieces(tag, &lines, fences, theme),
            Segment::Picture(image) => vec![Piece::Picture(image)],
        })
        .collect()
}

/// The body cut where a fence opens and closes, and after each line holding a picture, which leaves its line.
/// A list or a quote holding one is cut in two there: mrk draws each side on its own.
fn segments(body: &str) -> Vec<Segment<'_>> {
    let mut segments = vec![];
    let mut raws = body.lines();
    while let Some(raw) = raws.next() {
        if let Some(tag) = raw.trim_start().strip_prefix("```") {
            let lines = raws.by_ref().take_while(|line| !line.trim_start().starts_with("```")).collect();
            segments.push(Segment::Fence { tag, lines });
            continue;
        }
        let (text, pictures) = image::split(raw);
        if pictures.is_empty() || !text.trim().is_empty() {
            match segments.last_mut() {
                Some(Segment::Markdown(lines)) => lines.push(text),
                _ => segments.push(Segment::Markdown(vec![text])),
            }
        }
        segments.extend(pictures.into_iter().map(Segment::Picture));
    }
    segments
}

/// Markdown as mrk draws it, each row copied as drawn and its links clickable where `fences` says.
#[cfg(feature = "prose")]
fn markdown(lines: &[String], width: usize, fences: &Fences<'_>, theme: Theme) -> Vec<Piece> {
    use crate::tui::prose::Links;
    // The forges show each line break of a comment, so each line ends in a hard break.
    let text = lines.iter().map(|line| format!("{line}  ")).collect::<Vec<_>>().join("\n");
    let shown = if matches!(fences, Fences::Coloured(_)) { Links::Clickable } else { Links::Printed };
    let rows = crate::tui::prose::markdown(&text, width, shown, theme);
    rows.into_iter()
        .map(|row| {
            let links = anchors(&row);
            let line = Line::from(row.into_iter().map(|linked| linked.span).collect::<Vec<_>>());
            let raw = line.to_string();
            written(line, &raw, links)
        })
        .collect()
}

/// Without mrk, the Markdown as written.
#[cfg(not(feature = "prose"))]
fn markdown(lines: &[String], _: usize, _: &Fences<'_>, _: Theme) -> Vec<Piece> {
    lines.iter().map(|line| written(Line::raw(line.clone()), line, vec![])).collect()
}

/// The links a row of mrk's spans draws, each run of spans to one target as one.
#[cfg(feature = "prose")]
fn anchors(row: &[crate::tui::prose::Linked]) -> Vec<Anchor> {
    let mut column = 0;
    let runs = row.chunk_by(|a, b| a.link == b.link).filter_map(|run| {
        let cells = column..column + run.iter().map(|linked| linked.span.width()).sum::<usize>();
        column = cells.end;
        Some(Anchor { cells, url: LinkUrl::parse(run[0].link.as_deref()?)? })
    });
    runs.collect()
}

/// Wider than any pane, so mrk keeps a short text on one row: the caller wraps it.
#[cfg(feature = "prose")]
const ONE_ROW: usize = u16::MAX as usize;

/// One line of Markdown as spans, for the panes that draw a short text inline; a link's target follows its text.
#[cfg(feature = "prose")]
pub(in crate::tui) fn inline<'a>(text: &str, theme: Theme) -> Vec<Span<'a>> {
    let rows = crate::tui::prose::markdown(text, ONE_ROW, crate::tui::prose::Links::Printed, theme);
    rows.into_iter().flatten().map(|linked| linked.span).collect()
}

/// Without mrk, the text as written.
#[cfg(not(feature = "prose"))]
pub(in crate::tui) fn inline<'a>(text: &str, _: Theme) -> Vec<Span<'a>> {
    vec![Span::raw(text.to_owned())]
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

fn written(line: Line<'static>, raw: &str, links: Vec<Anchor>) -> Piece {
    Piece::Written { line, raw: raw.to_owned(), links }
}

/// Where a markdown link points as written: a web address or a path on the forge's host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LinkUrl(String);

impl LinkUrl {
    /// A link revu can open: `http(s)://…` or an absolute `/path`; a relative path, an `#anchor`
    /// or a `mailto:` would open the wrong page, so they stay text.
    #[cfg(feature = "prose")]
    fn parse(url: &str) -> Option<Self> {
        let web = url.starts_with("http://") || url.starts_with("https://");
        let path = url.starts_with('/') && !url.starts_with("//");
        (web || path).then(|| Self(url.to_owned()))
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

/// A link drawn in a body line: the cells its text takes and where it points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Anchor {
    pub(super) cells: Range<usize>,
    pub(super) url: LinkUrl,
}

#[cfg(test)]
mod tests {
    use super::super::tests::text;
    use super::super::{Mark, written_rows};
    use super::*;
    use ratatui::style::Modifier;
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

    fn tokyonight() -> Theme {
        Theme::named("tokyonight").unwrap()
    }

    /// A body as the thread pane draws it `width` columns wide, where links click.
    fn in_thread(body: &str, width: usize) -> Vec<Piece> {
        body_pieces(body, &Fences::Coloured(&Replaced::default()), width, tokyonight())
    }

    fn style_of(line: &Line<'_>, content: &str) -> Style {
        line.spans.iter().find(|span| span.content == content).unwrap_or_else(|| panic!("no {content:?} in {line:?}")).style
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
                "text broke",
                "picture /uploads/ab12/chart.png",
                "picture https://x/a.png",
                "text   ![not](https://x/code.png)"
            ]
        );
    }

    #[test]
    fn mrk_draws_code_spans_bullets_quotes_and_emphasis() {
        let lines = texts(in_thread("Use `Key::from` **now**.\n\n- one\n\n> said", 80));
        assert_eq!(text(&lines), ["Use  Key::from  now.", "", "• one", "", "│ said"]);
        assert!(style_of(&lines[0], "now").add_modifier.contains(Modifier::BOLD));
        assert!(style_of(&lines[0], " Key::from ").bg.is_some(), "a code span sits on its own ground");
    }

    #[test]
    fn each_line_break_of_a_comment_stays_one() {
        assert_eq!(text(&texts(in_thread("first line\nsecond line", 80))), ["first line", "second line"]);
    }

    #[test]
    fn a_long_line_wraps_to_the_width() {
        assert_eq!(text(&texts(in_thread("one two three four", 9))), ["one two", "three", "four"]);
    }

    #[test]
    fn a_table_is_drawn_by_mrk() {
        let lines = text(&texts(in_thread("| Name | Count |\n| --- | ---: |\n| alpha | 3 |", 80)));
        assert!(lines.iter().any(|line| line.contains("alpha") && line.contains('3') && !line.contains('|')), "{lines:?}");
    }

    /// Each link a body clicks through to as the thread pane lays it out `width` wide:
    /// its row, its column, the text it covers and its url.
    fn clicks(body: &str, width: usize) -> Vec<(usize, u16, String, String)> {
        let rows = in_thread(body, width).into_iter().flat_map(|piece| match piece {
            Piece::Written { line, raw, links } => written_rows(line, &raw, &links, width, &str::to_owned),
            Piece::Text(_) | Piece::Picture(_) => vec![],
        });
        let marks = rows.enumerate().flat_map(|(at, (_, marks))| marks.into_iter().map(move |mark| (at, mark)));
        marks
            .filter_map(|(at, mark)| match mark {
                Mark::Link { column, text, url } => Some((at, column, text, url)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_link_shows_its_text_in_the_link_colour() {
        let lines = texts(in_thread("see [the docs](https://x/a \"title\") now", 80));
        assert_eq!(text(&lines), ["see the docs now"]);
        assert_eq!(style_of(&lines[0], "the docs").fg, Some(tokyonight().link));
    }

    #[test]
    fn a_link_shows_its_target_in_a_pane_that_cannot_click_it() {
        assert_eq!(text(&body_lines("see [the docs](https://x/a) now", 80, tokyonight())), ["see the docs <https://x/a> now"]);
    }

    #[test]
    fn a_link_inside_a_code_span_or_a_fence_stays_literal() {
        let lines = text(&texts(in_thread("use `[a](https://x/a)` here\n```\n[b](https://x/b)\n```", 80)));
        assert_eq!(lines, ["use  [a](https://x/a)  here", "  [b](https://x/b)"]);
    }

    #[test]
    fn a_link_clicks_through_on_each_row_its_text_is_drawn_on() {
        let guide = "https://x/guide".to_owned();
        assert_eq!(
            clicks("read [the guide](https://x/guide) first", 10),
            [(0, 5, "the".into(), guide.clone()), (1, 0, "guide".into(), guide)]
        );
    }

    #[test]
    fn a_link_after_a_wide_emoji_clicks_on_its_own_text() {
        assert_eq!(clicks("⚠️ see [docs](https://x/a) now", 80), [(0, 7, "docs".into(), "https://x/a".into())]);
    }

    #[test]
    fn only_a_web_address_or_an_absolute_path_clicks() {
        let body = "[a](docs/x.md) [b](#top) [c](mailto:nina@x) [e](/acme/widgets)";
        assert_eq!(clicks(body, 80), [(0, 6, "e".into(), "/acme/widgets".into())]);
    }

    #[test]
    fn a_drag_over_a_row_copies_its_drawn_letters() {
        let pieces = in_thread("a **bold** _it_ ~~gone~~", 80);
        let [Piece::Written { line, raw, links }] = pieces.as_slice() else { panic!("one written line") };
        let Some(Mark::Written { cells, .. }) = written_rows(line.clone(), raw, links, 80, &str::to_owned).remove(0).1.pop() else {
            panic!("a written row")
        };
        let copied: String = cells.into_iter().map(|bytes| &raw[bytes]).collect();
        assert_eq!(copied, "a bold it gone");
    }

    #[test]
    fn a_suggestion_reads_as_a_small_diff() {
        let body = "Try this:\n```suggestion:-0+0\nlet client = Client::default();\n```\nthanks";
        let replaced = Replaced { lines: vec!["let client = Client::new();".to_owned()], language: None };
        let lines = text(&texts(body_pieces(body, &Fences::Coloured(&replaced), 80, Theme::default())));
        assert_eq!(lines, ["Try this:", "- let client = Client::new();", "+ let client = Client::default();", "thanks"]);
        assert_eq!(text(&body_lines("```rust\nlet x = 1;\n```", 80, Theme::default())), ["  let x = 1;"], "other fences stay code");
    }

    /// The lines of a body in a pane whose rows are kept, its thread on `path`.
    fn kept_lines(body: &str, path: &str, replaced: &[&str]) -> Vec<Line<'static>> {
        let replaced = Replaced {
            lines: replaced.iter().map(|line| (*line).to_owned()).collect(),
            language: syntax::language_for(path, &syntax::LANGUAGES),
        };
        texts(body_pieces(body, &Fences::Coloured(&replaced), 80, tokyonight()))
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
        let pieces = in_thread("```ts\n\tconst x = 1;\n```", 80);
        let [Piece::Written { line, raw, .. }] = pieces.as_slice() else { panic!("one written line") };
        assert_eq!(raw, "\tconst x = 1;");
        let Some(Mark::Written { cells, .. }) = written_rows(line.clone(), raw, &[], 80, &str::to_owned).remove(0).1.pop() else {
            panic!("a written row")
        };
        assert_eq!(cells.last(), Some(&(12..13)), "the last cell copies the last byte");
    }

    #[test]
    fn inline_text_draws_its_markdown_on_one_row() {
        let spans = inline("**Bold** and `code`", tokyonight());
        assert_eq!(spans.iter().map(|span| span.content.as_ref()).collect::<String>(), "Bold and  code ");
        assert!(spans[0].style.add_modifier.contains(Modifier::BOLD));
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
