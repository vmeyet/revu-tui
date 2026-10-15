//! A long line wrapped into screen rows that keep its fill.
use ratatui::style::Style;
use ratatui::text::{Line, Span};

/// One long line as several screen rows of `width`: continuation rows indented under the text,
/// every row padded with the line's fill so a changed line stays one coloured block.
pub(super) fn wrap_row(line: Line<'static>, width: usize, indent: usize) -> Vec<Line<'static>> {
    let mut spans = line.spans;
    let fill = trailing_fill(&mut spans);
    let mut rows: Vec<Vec<Span<'static>>> = vec![];
    let mut row: Vec<Span<'static>> = vec![];
    let mut used = 0;
    for span in spans {
        let mut piece = String::new();
        for c in span.content.chars() {
            let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
            if used + w > width && used > indent {
                row.push(Span::styled(std::mem::take(&mut piece), span.style));
                rows.push(std::mem::replace(&mut row, vec![Span::styled(" ".repeat(indent), fill.unwrap_or_default())]));
                used = indent;
            }
            piece.push(c);
            used += w;
        }
        row.push(Span::styled(piece, span.style));
    }
    rows.push(row);
    rows.into_iter().map(|row| padded(row, width, fill)).collect()
}

/// Drops the blank padding a changed line ends with, and gives back its style to pad each row again.
fn trailing_fill(spans: &mut Vec<Span<'static>>) -> Option<Style> {
    let last = spans.last().filter(|s| is_blank(s))?;
    let style = last.style;
    spans.pop();
    Some(style)
}

pub(super) fn is_blank(span: &Span<'_>) -> bool {
    !span.content.is_empty() && span.content.chars().all(|c| c == ' ')
}

pub(super) fn padded(mut row: Vec<Span<'static>>, width: usize, fill: Option<Style>) -> Line<'static> {
    let used: usize = row.iter().map(Span::width).sum();
    if let (Some(style), true) = (fill, used < width) {
        row.push(Span::styled(" ".repeat(width - used), style));
    }
    Line::from(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme::Theme;

    #[test]
    fn wrapping_indents_continuations_and_keeps_the_fill_on_every_row() {
        let fill = Style::default().bg(Theme::named("dracula").unwrap().danger);
        let line = Line::from(vec![Span::raw("  1    1 "), Span::styled("+abcdefghij", fill), Span::styled("     ", fill)]);
        let rows = wrap_row(line, 14, 4);
        let text: Vec<String> = rows.iter().map(|r| r.spans.iter().map(|s| s.content.to_string()).collect()).collect();
        assert_eq!(text, ["  1    1 +abcd", "    efghij    "]);
        assert!(rows.iter().all(|r| r.width() == 14), "every row reaches the edge");
        assert_eq!(rows[1].spans.last().unwrap().style, fill, "the fill pads the last row");
    }
}
