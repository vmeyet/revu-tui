//! Markdown tables in the diff: a markdown file's table lines keep their text and only draw their pipes as box lines,
//! or, wrapped, wrap each cell inside its column.
use super::theme::Theme;
use ratatui::style::Style;
use ratatui::text::Span;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// A wrapped column keeps room for a short word.
const MIN_WRAPPED: usize = 3;
/// The outer `│ ` and ` │` around a wrapped row's cells.
const EDGES: usize = 4;
const GAP: &str = " │ ";

type Styled = [(char, Style)];

/// Column widths that fit `width`, taking one column from the widest until they do.
fn fit(natural: &[usize], width: usize) -> Option<Vec<usize>> {
    let gaps = GAP.width() * natural.len().saturating_sub(1);
    let mut widths = natural.to_vec();
    while widths.iter().sum::<usize>() + gaps > width {
        let widest = widths.iter().enumerate().max_by_key(|(_, w)| **w).map(|(i, _)| i)?;
        if widths[widest] <= MIN_WRAPPED {
            return None;
        }
        widths[widest] -= 1;
    }
    Some(widths)
}

/// A raw markdown table line as the diff shows it, its text untouched: only its drawing changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum TableLine {
    Cells,
    Delimiter,
}

impl TableLine {
    pub(super) fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if !text.starts_with('|') {
            return None;
        }
        let delimiter = text.contains('-') && text.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '));
        Some(if delimiter { Self::Delimiter } else { Self::Cells })
    }
}

/// `spans` with the line's pipes, and a delimiter row's dashes, drawn as box lines in the faded
/// colour. Each glyph takes the one cell its character took, so widths and highlights still line up.
pub(super) fn boxed(spans: Vec<Span<'_>>, line: TableLine, theme: Theme) -> Vec<Span<'_>> {
    let text: String = spans.iter().map(|span| span.content.as_ref()).collect();
    let glyphs = glyphs(&text, line);
    let mut drawn = vec![];
    let mut at = 0;
    for span in spans {
        let mut from = 0;
        for (i, _) in span.content.char_indices() {
            if let Ok(found) = glyphs.binary_search_by_key(&(at + i), |(offset, _)| *offset) {
                drawn.extend((from < i).then(|| Span::styled(span.content[from..i].to_owned(), span.style)));
                drawn.push(Span::styled(glyphs[found].1, span.style.fg(theme.faded)));
                from = i + 1;
            }
        }
        drawn.extend((from < span.content.len()).then(|| Span::styled(span.content[from..].to_owned(), span.style)));
        at += span.content.len();
    }
    drawn
}

/// A table line wider than `width` as rows exactly `width` wide, each cell wrapped inside its column.
/// The columns come from this line's pipe positions alone, so the rows of an aligned table stay aligned.
/// `None` when the line fits, or when its columns cannot fit even at their narrowest.
pub(super) fn wrapped(spans: &[Span<'_>], line: TableLine, width: usize, fill: Style, theme: Theme) -> Option<Vec<Vec<Span<'static>>>> {
    let chars: Vec<(char, Style)> = spans.iter().flat_map(|span| span.content.chars().map(move |c| (c, span.style))).collect();
    let text: String = chars.iter().map(|(c, _)| c).collect::<String>().trim_end_matches(' ').to_owned();
    if text.width() <= width {
        return None;
    }
    let pipes = pipes(&text);
    let (&first, &last) = (pipes.first()?, pipes.last()?);
    let (lead, tail) = (&chars[..first], &chars[last + 1..text.chars().count()]);
    let cells: Vec<&Styled> = pipes.windows(2).map(|pair| &chars[pair[0] + 1..pair[1]]).collect();
    let natural: Vec<usize> = cells.iter().map(|cell| width_of(cell).saturating_sub(2).max(MIN_WRAPPED)).collect();
    let widths = fit(&natural, width.checked_sub(width_of(lead) + width_of(tail) + EDGES)?)?;
    let pipe = |i: usize| chars[pipes[i]].1.fg(theme.faded);
    let columns: Vec<Vec<Vec<(char, Style)>>> = cells
        .iter()
        .zip(&widths)
        .enumerate()
        .map(|(i, (cell, &w))| match line {
            TableLine::Delimiter => vec![vec![('─', pipe(i)); w + 2]],
            TableLine::Cells => wrap_cell(trimmed(cell), w).into_iter().map(|piece| padded_cell(piece, w, fill)).collect(),
        })
        .collect();
    let joint = |i: usize| if line == TableLine::Delimiter && i != 0 && i != cells.len() { '┼' } else { '│' };
    let height = columns.iter().map(Vec::len).max().unwrap_or(1);
    let rows = (0..height).map(|r| {
        let mut row = if r == 0 { lead.to_vec() } else { vec![(' ', fill); width_of(lead)] };
        row.push((joint(0), pipe(0)));
        for (i, (column, &w)) in columns.iter().zip(&widths).enumerate() {
            row.extend(column.get(r).cloned().unwrap_or_else(|| vec![(' ', fill); w + 2]));
            row.push((joint(i + 1), pipe(i + 1)));
        }
        row.extend(if r == 0 { tail } else { &[] });
        row.extend(vec![(' ', fill); width.saturating_sub(width_of(&row))]);
        spans_of(&row)
    });
    Some(rows.collect())
}

/// The character index of each unescaped pipe.
fn pipes(text: &str) -> Vec<usize> {
    let offsets: Vec<usize> = glyphs(text, TableLine::Cells).into_iter().map(|(at, _)| at).collect();
    text.char_indices().enumerate().filter(|(_, (at, _))| offsets.contains(at)).map(|(i, _)| i).collect()
}

/// `cell` in pieces at most `width` wide, broken at the last space in reach, else between letters.
fn wrap_cell(cell: &Styled, width: usize) -> Vec<&Styled> {
    let mut pieces = vec![];
    let mut rest = cell;
    while !rest.is_empty() {
        let reach = reach(rest, width).max(1);
        let cut = if reach == rest.len() {
            reach
        } else {
            rest[..=reach].iter().rposition(|(c, _)| *c == ' ').filter(|&at| at > 0).unwrap_or(reach)
        };
        pieces.push(trimmed(&rest[..cut]));
        rest = trimmed(&rest[cut..]);
    }
    pieces
}

/// A piece of a cell between the column's padding spaces, filled to `width`.
fn padded_cell(piece: &Styled, width: usize, fill: Style) -> Vec<(char, Style)> {
    let after = vec![(' ', fill); width.saturating_sub(width_of(piece)) + 1];
    std::iter::once((' ', fill)).chain(piece.iter().copied()).chain(after).collect()
}

/// How many of `chars` fit in `width` columns.
fn reach(chars: &Styled, width: usize) -> usize {
    chars
        .iter()
        .scan(0, |used, (c, _)| {
            *used += c.width().unwrap_or(0);
            (*used <= width).then_some(())
        })
        .count()
}

fn trimmed(chars: &Styled) -> &Styled {
    let start = chars.iter().position(|(c, _)| *c != ' ').unwrap_or(chars.len());
    let end = chars.iter().rposition(|(c, _)| *c != ' ').map_or(start, |i| i + 1);
    &chars[start..end]
}

fn width_of(chars: &Styled) -> usize {
    chars.iter().map(|(c, _)| c.width().unwrap_or(0)).sum()
}

fn spans_of(chars: &Styled) -> Vec<Span<'static>> {
    chars.chunk_by(|a, b| a.1 == b.1).map(|run| Span::styled(run.iter().map(|(c, _)| c).collect::<String>(), run[0].1)).collect()
}

/// The box glyph of each unescaped pipe and, on a delimiter row, each dash, by byte offset;
/// a delimiter row crosses its inner pipes and keeps its outer ones straight.
fn glyphs(text: &str, line: TableLine) -> Vec<(usize, &'static str)> {
    let first = text.len() - text.trim_start().len();
    let last = text.trim_end_matches([' ', '·']).len().saturating_sub(1);
    text.char_indices()
        .filter_map(|(at, c)| match (c, line) {
            ('|', _) if text[..at].ends_with('\\') => None,
            ('|', TableLine::Delimiter) if at != first && at != last => Some((at, "┼")),
            ('|', _) => Some((at, "│")),
            ('-', TableLine::Delimiter) => Some((at, "─")),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed_text(text: &str) -> Option<String> {
        let line = TableLine::parse(text)?;
        let spans = boxed(vec![Span::raw(text.to_owned())], line, Theme::default());
        Some(spans.iter().map(|s| s.content.as_ref()).collect())
    }

    #[test]
    fn a_table_row_draws_its_unescaped_pipes_as_box_lines() {
        assert_eq!(boxed_text("| a | b \\| c |").as_deref(), Some("│ a │ b \\| c │"));
        assert_eq!(boxed_text("  | a-b |").as_deref(), Some("  │ a-b │"), "dashes in a cell stay");
    }

    #[test]
    fn a_delimiter_row_draws_a_rule_crossed_at_its_inner_pipes() {
        assert_eq!(boxed_text("|:---|---:|").as_deref(), Some("│:───┼───:│"));
        assert_eq!(boxed_text("| --- | --- |").as_deref(), Some("│ ─── ┼ ─── │"));
        let marked = boxed(vec![Span::raw("|---|---|··")], TableLine::Delimiter, Theme::default());
        assert_eq!(marked.iter().map(|s| s.content.as_ref()).collect::<String>(), "│───┼───│··", "trailing space marks are no cell");
    }

    #[test]
    fn lines_that_do_not_start_with_a_pipe_are_no_table() {
        assert_eq!(TableLine::parse("a | b"), None);
        assert_eq!(TableLine::parse("---"), None);
        assert_eq!(TableLine::parse(""), None);
    }

    #[test]
    fn box_lines_keep_every_width_and_every_style_but_the_glyph_colour() {
        let theme = Theme::default();
        let word = Style::default().bg(ratatui::style::Color::Red);
        let spans = vec![Span::raw("| a "), Span::styled("| b", word), Span::raw(" |")];
        let drawn = boxed(spans.clone(), TableLine::Cells, theme);
        let widths = |spans: &[Span<'_>]| spans.iter().map(Span::width).sum::<usize>();
        assert_eq!(widths(&drawn), widths(&spans));
        let glyph = drawn.iter().find(|s| s.content == "│" && s.style.bg == word.bg).unwrap();
        assert_eq!(glyph.style.fg, Some(theme.faded), "the pipe of a changed word keeps its fill");
    }

    fn wrapped_text(text: &str, width: usize) -> Option<Vec<String>> {
        let rows = wrapped(&[Span::raw(text.to_owned())], TableLine::parse(text)?, width, Style::default(), Theme::default())?;
        Some(rows.iter().map(|row| row.iter().map(|s| s.content.as_ref()).collect()).collect())
    }

    fn joints(row: &str) -> Vec<usize> {
        row.chars().enumerate().filter(|(_, c)| matches!(c, '│' | '┼')).map(|(i, _)| i).collect()
    }

    #[test]
    fn rows_with_the_same_pipes_wrap_to_the_same_columns_within_the_width() {
        let header = "| Plan     | Notes about the plan            |";
        let body = "| Team     | shared billing for every seat   |";
        let delimiter = "|:---------|:--------------------------------|";
        let drawn: Vec<Vec<String>> = [header, delimiter, body].iter().map(|row| wrapped_text(row, 30).unwrap()).collect();
        assert!(drawn.iter().flatten().all(|row| row.width() == 30), "{drawn:?}");
        assert!(drawn.iter().flatten().all(|row| joints(row) == joints(&drawn[0][0])), "{drawn:?}");
        assert_eq!(drawn[1], ["│──────────┼─────────────────│"], "the delimiter row shrinks to the same columns");
        assert_eq!(drawn[2], ["│ Team     │ shared billing  │", "│          │ for every seat  │"], "the widest column gives way first");
    }

    #[test]
    fn a_cell_breaks_at_a_space_in_reach_and_between_letters_without_one() {
        let chars = |text: &str| text.chars().map(|c| (c, Style::default())).collect::<Vec<_>>();
        let pieces =
            |text: &str, width| wrap_cell(&chars(text), width).iter().map(|p| p.iter().map(|(c, _)| c).collect()).collect::<Vec<String>>();
        assert_eq!(pieces("ab cd ef", 5), ["ab cd", "ef"]);
        assert_eq!(pieces("abc defg", 5), ["abc", "defg"]);
        assert_eq!(pieces("abcdefgh", 3), ["abc", "def", "gh"]);
        assert_eq!(pieces("", 3), Vec::<String>::new());
    }

    #[test]
    fn a_wrapped_cell_keeps_the_style_of_each_character() {
        let word = Style::default().bg(ratatui::style::Color::Red);
        let spans = [Span::raw("| a | one "), Span::styled("two", word), Span::raw(" three |")];
        let rows = wrapped(&spans, TableLine::Cells, 14, Style::default(), Theme::default()).unwrap();
        let two = rows.iter().flatten().find(|s| s.content.contains("two")).unwrap();
        assert_eq!((two.content.as_ref(), two.style), ("two", word));
        let pipe = rows[1].iter().find(|s| s.content == "│").unwrap();
        assert_eq!(pipe.style.fg, Some(Theme::default().faded), "continuation rows repeat the column lines");
    }

    #[test]
    fn a_line_that_fits_or_has_no_room_is_left_to_the_plain_wrap() {
        assert_eq!(wrapped_text("| a | b |", 20), None, "it fits");
        assert_eq!(wrapped_text("| alpha | beta | gamma |", 12), None, "three columns need more than twelve");
        assert_eq!(wrapped_text("| just one pipe here", 8), None);
    }
}
