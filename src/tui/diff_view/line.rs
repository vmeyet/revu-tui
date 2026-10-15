//! One diff line painted: its numbers, its sign, syntax colours under the diff's fills, changed words marked.
use super::{GUTTER_W, Gutter, TAB};
use crate::diff::words::{Segment, segments};
use crate::diff::{Line as DiffLine, LineKind};
use crate::review::Side;
use crate::syntax::Token;
use crate::tui::theme::Theme;
use crate::tui::ui::truncate_tabs;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use std::ops::Range;
use unicode_width::UnicodeWidthStr;

/// How one kind of changed line paints: sign, text colour, fills, and the colour of its meaning.
pub(in crate::tui) struct Paint {
    sign: &'static str,
    text: Color,
    fill: Option<Color>,
    word: Option<Color>,
    accent: Color,
}

impl Paint {
    /// The line's text colour, on its fill when the theme has one.
    pub(in crate::tui) fn base(&self) -> Style {
        self.fill.map_or(Style::default(), |fill| Style::default().bg(fill)).fg(self.text)
    }

    pub(in crate::tui) fn sign_span(&self) -> Span<'static> {
        Span::styled(self.sign, self.base().fg(self.accent))
    }
}

pub(in crate::tui) fn paint(kind: LineKind, theme: Theme) -> Option<Paint> {
    match kind {
        LineKind::Added => {
            Some(Paint { sign: "+", text: theme.added, fill: theme.added_fill, word: theme.added_word, accent: theme.success })
        }
        LineKind::Removed => {
            Some(Paint { sign: "-", text: theme.removed, fill: theme.removed_fill, word: theme.removed_word, accent: theme.danger })
        }
        LineKind::Context => None,
    }
}

pub(super) fn number(n: Option<u32>) -> String {
    n.map_or_else(|| " ".repeat(GUTTER_W), |n| format!("{n:>GUTTER_W$}"))
}

/// A changed line is filled edge to edge; without a fill, the text itself carries the colour
/// and changed words go bold, so every theme reads on every terminal.
pub(super) fn line_spans<'a>(
    line: &DiffLine,
    code: &[(Range<usize>, Token)],
    selected: bool,
    gutter: Gutter,
    width: usize,
    theme: Theme,
) -> Vec<Span<'a>> {
    numbered_spans(gutter.numbers(line.old, line.new), line, code, selected, width, theme)
}

/// A line drawn after `gutter`, its numbers: both of them or one inline, the one of its side side by side.
pub(super) fn numbered_spans<'a>(
    gutter: String,
    line: &DiffLine,
    code: &[(Range<usize>, Token)],
    selected: bool,
    width: usize,
    theme: Theme,
) -> Vec<Span<'a>> {
    let gutter_colour = if selected { theme.muted } else { theme.faded };
    let paint = paint(line.kind, theme);
    let base = paint.as_ref().map_or(Style::default(), Paint::base);
    let sign = paint.as_ref().map_or_else(|| Span::styled(" ", base), Paint::sign_span);
    let word = paint.as_ref().map(|p| match p.word {
        Some(fill) => base.fg(p.accent).bg(fill),
        None => base.add_modifier(Modifier::BOLD),
    });
    let room = width.saturating_sub(gutter.width() + 1);
    let mut spans = vec![Span::styled(gutter, base.fg(gutter_colour)), sign];
    let text = text_spans(line, code, room, base, word, theme);
    let used: usize = text.iter().map(Span::width).sum();
    spans.extend(text);
    if paint.is_some() && used < room {
        spans.push(Span::styled(" ".repeat(room - used), base));
    }
    spans
}

/// The text cut into pieces wherever a changed word or a syntax colour starts or stops: syntax
/// sets the text colour, the diff keeps the fills; tabs made visible, trailing spaces marked, cut to `room`.
fn text_spans<'a>(
    line: &DiffLine,
    code: &[(Range<usize>, Token)],
    room: usize,
    base: Style,
    word: Option<Style>,
    theme: Theme,
) -> Vec<Span<'a>> {
    let changed = line.kind != LineKind::Context;
    let trimmed = line.text.trim_end_matches([' ', '\t']);
    let trailing = line.text.len() - trimmed.len();
    let mut edges: Vec<usize> = line.words.iter().chain(code.iter().map(|(range, _)| range)).flat_map(|r| [r.start, r.end]).collect();
    edges.extend([0, trimmed.len()]);
    edges.retain(|&edge| edge <= trimmed.len() && trimmed.is_char_boundary(edge));
    edges.sort_unstable();
    edges.dedup();
    let mut parts: Vec<(&str, Style)> = vec![];
    for piece in edges.windows(2) {
        let (from, to) = (piece[0], piece[1]);
        let in_word = line.words.iter().any(|w| w.start <= from && from < w.end);
        let style = if in_word { word.unwrap_or(base) } else { base };
        parts.push((&trimmed[from..to], with_syntax(style, code, from, theme)));
    }
    let dots = "·".repeat(trailing);
    if changed && trailing > 0 {
        parts.push((&dots, base.fg(theme.warn)));
    }
    fit(parts, room)
}

/// `style` with the syntax colour of the token covering byte `at`, if any.
fn with_syntax(style: Style, code: &[(Range<usize>, Token)], at: usize, theme: Theme) -> Style {
    code.iter().find(|(range, _)| range.start <= at && at < range.end).map_or(style, |(_, token)| style.fg(theme.syntax.colour(*token)))
}

/// A removed line and its added twin on one row: the kept text in its syntax colours, each old
/// word struck through in the removed colours, then its replacement in the added colours.
/// `code` is the added line's syntax, so offsets follow the new text: kept and new pieces advance it.
pub(super) fn pair_spans<'a>(
    old: &DiffLine,
    new: &DiffLine,
    code: &[(Range<usize>, Token)],
    selected: bool,
    gutter: Gutter,
    width: usize,
    theme: Theme,
) -> Vec<Span<'a>> {
    let faint = Style::default().fg(if selected { theme.muted } else { theme.faded });
    let with_fill = |style: Style, fill: Option<Color>| fill.map_or(style, |f| style.bg(f));
    let dropped = with_fill(Style::default().fg(theme.danger).add_modifier(Modifier::CROSSED_OUT), theme.removed_word);
    let added = with_fill(Style::default().fg(theme.success), theme.added_word);
    let parts = segments(old, new);
    let mut styled: Vec<(&str, Style)> = vec![];
    let mut at = 0;
    for part in &parts {
        match part {
            Segment::Same(text) => {
                styled.extend(coloured(text, at, code, theme));
                at += text.len();
            }
            Segment::Old(text) => styled.push((text.as_str(), dropped)),
            Segment::New(text) => {
                styled.push((text.as_str(), added));
                at += text.len();
            }
        }
    }
    let mut spans = vec![Span::styled(gutter.numbers(old.old, new.new), faint), Span::styled("~", Style::default().fg(theme.warn))];
    spans.extend(fit(styled, width.saturating_sub(gutter.width() + 1)));
    spans
}

/// `text`, found at byte `at` of its line, cut wherever a syntax token starts or stops.
pub(in crate::tui) fn coloured<'t>(text: &'t str, at: usize, code: &[(Range<usize>, Token)], theme: Theme) -> Vec<(&'t str, Style)> {
    let end = at + text.len();
    let mut edges: Vec<usize> = code.iter().flat_map(|(r, _)| [r.start, r.end]).filter(|&e| at < e && e < end).map(|e| e - at).collect();
    edges.extend([0, text.len()]);
    edges.retain(|&edge| text.is_char_boundary(edge));
    edges.sort_unstable();
    edges.dedup();
    edges.windows(2).map(|w| (&text[w[0]..w[1]], with_syntax(Style::default(), code, at + w[0], theme))).collect()
}

/// A line that changed only in whitespace, under `W`: read as context, both numbers, a `≈` for a sign.
pub(super) fn quiet_spans<'a>(
    old: &DiffLine,
    new: &DiffLine,
    code: &[(Range<usize>, Token)],
    selected: bool,
    gutter: Gutter,
    width: usize,
    theme: Theme,
) -> Vec<Span<'a>> {
    let context = DiffLine { old: old.old, ..quiet(new) };
    with_quiet_sign(line_spans(&context, code, selected, gutter, width, theme), theme)
}

/// A line that changed only in whitespace, drawn as context.
pub(super) fn quiet(line: &DiffLine) -> DiffLine {
    DiffLine { kind: LineKind::Context, words: vec![], ..line.clone() }
}

/// A line of the side a peek reads, drawn as plain code numbered on that side alone.
pub(super) fn peeked(line: &DiffLine, side: Side) -> DiffLine {
    let (old, new) = match side {
        Side::Old => (line.old, None),
        Side::New => (None, line.new),
    };
    DiffLine { old, new, ..quiet(line) }
}

pub(super) fn with_quiet_sign(mut spans: Vec<Span<'_>>, theme: Theme) -> Vec<Span<'_>> {
    if let Some(sign) = spans.get_mut(1) {
        *sign = Span::styled("≈", Style::default().fg(theme.faded));
    }
    spans
}

/// Styled pieces laid end to end, tabs made visible, cut to `room` columns.
fn fit<'a>(parts: Vec<(&str, Style)>, room: usize) -> Vec<Span<'a>> {
    let mut spans = vec![];
    let mut used = 0;
    for (text, style) in parts {
        if used >= room {
            break;
        }
        let cut = truncate_tabs(text, TAB, room - used);
        used += cut.width();
        spans.push(Span::styled(cut, style));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff;
    use crate::tui::diff_view::test_support::{cart, painted, spans_text, total_pair};

    /// Each piece of a painted line as `fg/bg text`, so a snapshot shows the colours.
    #[test]
    fn an_inline_row_keeps_syntax_on_the_kept_text_and_diff_colours_on_the_words() {
        let hunk = total_pair();
        let (old, new) = (&hunk.lines[0], &hunk.lines[1]);
        let code = [(0..5, Token::Keyword), (14..15, Token::Number)];
        let theme = Theme::named("tokyonight").unwrap();
        let spans = pair_spans(old, new, &code, false, Gutter::Both, 80, theme);
        let style_of = |text: &str| spans.iter().find(|s| s.content == text).map(|s| s.style).unwrap();
        assert_eq!(spans_text(&spans).trim_end(), "   1    1 ~const total = 1;2;");
        assert_eq!(style_of("const").fg, Some(theme.syntax.colour(Token::Keyword)), "kept text takes its syntax colour");
        assert_eq!(style_of(" total = ").fg, None);
        assert_eq!(style_of("1;").fg, Some(theme.danger), "the old word keeps the diff colour");
        assert!(style_of("1;").add_modifier.contains(Modifier::CROSSED_OUT));
        assert_eq!(style_of("2;").fg, Some(theme.success), "the new word keeps the diff colour");
    }

    #[test]
    fn syntax_sets_the_text_and_the_diff_keeps_the_fills() {
        let file = cart();
        let theme = Theme::named("tokyonight").unwrap();
        let spans = line_spans(&file.hunks[0].lines[1], file.spans(0, 1), false, Gutter::Both, 100, theme);
        let keyword = spans.iter().find(|s| s.content == "const").unwrap();
        assert_eq!((keyword.style.fg, keyword.style.bg), (Some(theme.syntax.keyword), theme.removed_fill));
        let comment = spans.iter().find(|s| s.content.contains("// cents")).unwrap();
        assert_eq!((comment.style.fg, comment.style.bg), (Some(theme.syntax.comment), theme.removed_fill));
        let changed = line_spans(&file.hunks[0].lines[2], file.spans(0, 2), false, Gutter::Both, 100, theme);
        let word = changed.iter().find(|s| s.content.contains("quantity")).unwrap();
        assert_eq!(word.style.bg, theme.added_word, "a changed word keeps its stronger fill under the syntax colour");
    }

    #[test]
    fn on_an_unknown_ground_syntax_colours_the_text_and_only_the_sign_says_removed() {
        let file = cart();
        let theme = Theme::default();
        let spans = line_spans(&file.hunks[0].lines[1], file.spans(0, 1), false, Gutter::Both, 100, theme);
        assert_eq!(spans[1].content, "-");
        assert_eq!(spans[1].style.fg, Some(theme.danger));
        let keyword = spans.iter().find(|s| s.content == "const").unwrap();
        assert_eq!((keyword.style.fg, keyword.style.bg), (Some(theme.syntax.keyword), None));
    }

    #[test]
    fn snapshot_highlighted_typescript_hunk() {
        let file = cart();
        let theme = Theme::named("tokyonight").unwrap();
        let lines: Vec<String> = file.hunks[0]
            .lines
            .iter()
            .enumerate()
            .map(|(i, line)| painted(&line_spans(line, file.spans(0, i), false, Gutter::Both, 100, theme)))
            .collect();
        insta::assert_snapshot!("highlighted_typescript_hunk", lines.join("\n---\n"));
    }

    #[test]
    fn line_shows_both_gutters_the_sign_and_visible_tabs() {
        let hunk = &diff::parse("@@ -1,2 +1,2 @@\n \tkeep  \n-\told\n+\tnew  \n")[0];
        let context = spans_text(&line_spans(&hunk.lines[0], &[], false, Gutter::Both, 60, Theme::default()));
        assert_eq!(context, "   1    1  →   keep");
        let added = spans_text(&line_spans(&hunk.lines[2], &[], false, Gutter::Both, 60, Theme::default()));
        assert_eq!(added.trim_end(), "        2 +→   new··", "trailing spaces are marked on changed lines");
        assert_eq!(added.width(), 60, "a changed line is filled edge to edge");
    }

    #[test]
    fn one_gutter_numbers_a_line_on_its_new_side_and_a_removed_line_on_its_old_side() {
        let hunk = &diff::parse("@@ -7,2 +9,2 @@\n keep\n-old\n+new\n")[0];
        let drawn = |index: usize| spans_text(&line_spans(&hunk.lines[index], &[], false, Gutter::One, 60, Theme::default()));
        assert_eq!(drawn(0), "   9  keep");
        assert_eq!(drawn(1).trim_end(), "   8 -old");
        assert_eq!(drawn(2).trim_end(), "  10 +new");
        assert_eq!(drawn(2).width(), 60, "a changed line is still filled edge to edge");
    }

    #[test]
    fn one_gutter_numbers_a_modified_line_on_its_new_side_and_gives_the_text_the_freed_columns() {
        let hunk =
            diff::words::mark(&diff::parse(&format!("@@ -3 +5 @@\n-const total = 1;{0}\n+const total = 2;{0}\n", "x".repeat(40)))[0]);
        let drawn = |gutter| spans_text(&pair_spans(&hunk.lines[0], &hunk.lines[1], &[], false, gutter, 30, Theme::default()));
        assert!(drawn(Gutter::One).starts_with("   5 ~const total = "), "{}", drawn(Gutter::One));
        assert_eq!(drawn(Gutter::One).width(), 30);
        assert_eq!(drawn(Gutter::Both).width(), 30, "both cut at the pane's edge");
    }

    #[test]
    fn long_lines_are_cut_to_the_pane() {
        let hunk = &diff::parse(&format!("@@ -1 +1 @@\n+{}\n", "x".repeat(100)))[0];
        let text = spans_text(&line_spans(&hunk.lines[0], &[], false, Gutter::Both, 30, Theme::default()));
        assert_eq!(text.width(), 30);
        assert!(text.ends_with('…'));
    }
}
