//! Markdown tables in note and description bodies: columns as wide as their widest cell, aligned
//! as the delimiter row says, the widest ones cut with `…` when the pane is narrower.
//! In the diff, a markdown file's table lines keep their text and only draw their pipes as box lines,
//! or, wrapped, wrap each cell inside its column.
use super::theme::Theme;
use super::thread_view::inline;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// A shrunk column keeps at least a letter and its `…`.
const MIN_COLUMN: usize = 2;
/// A wrapped column keeps room for a short word.
const MIN_WRAPPED: usize = 3;
/// The outer `│ ` and ` │` around a wrapped row's cells.
const EDGES: usize = 4;
const GAP: &str = " │ ";

type Cell = Vec<Span<'static>>;
type Styled = [(char, Style)];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Align {
    Left,
    Center,
    Right,
}

/// The table the `rows` spell, drawn within `width`: `None` when the second row is no delimiter
/// row, or when the table cannot fit even with every column cut down.
pub(super) fn table_lines(rows: &[&str], width: usize, theme: Theme) -> Option<Vec<Line<'static>>> {
    let [header, delimiter, body @ ..] = rows else { return None };
    let header = cells(header);
    let aligns = aligns(&cells(delimiter)).filter(|aligns| aligns.len() == header.len())?;
    let styled = |row: Vec<String>| -> Vec<Cell> { row.iter().map(|cell| inline(cell, theme)).collect() };
    let header: Vec<Cell> = styled(header).into_iter().map(|cell| bold(&cell)).collect();
    let body: Vec<Vec<Cell>> = body.iter().map(|row| styled(fill(cells(row), aligns.len()))).collect();
    let widths = fit(&natural_widths(&header, &body), width, MIN_COLUMN)?;
    let border = Style::default().fg(theme.faded);
    let rule = widths.iter().map(|w| "─".repeat(*w)).collect::<Vec<_>>().join("─┼─");
    let mut lines = vec![row_line(&header, &widths, &aligns, border), Line::from(Span::styled(rule, border))];
    lines.extend(body.iter().map(|row| row_line(row, &widths, &aligns, border)));
    Some(lines)
}

/// A row's cells, split on the pipes a backslash does not escape, the outer pipes dropped.
fn cells(row: &str) -> Vec<String> {
    let row = row.trim();
    let row = row.strip_prefix('|').unwrap_or(row);
    let row = if row.ends_with("\\|") { row } else { row.strip_suffix('|').unwrap_or(row) };
    let mut cells = vec![];
    let mut cell = String::new();
    let mut chars = row.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.next_if_eq(&'|').is_some() {
            cell.push('|');
        } else if c == '|' {
            cells.push(std::mem::take(&mut cell));
        } else {
            cell.push(c);
        }
    }
    cells.push(cell);
    cells.into_iter().map(|cell| cell.trim().to_owned()).collect()
}

/// `---` left, `:---:` centre, `---:` right; `None` when a cell is not dashes between optional colons.
fn aligns(delimiter: &[String]) -> Option<Vec<Align>> {
    delimiter
        .iter()
        .map(|cell| {
            let (left, rest) = cell.strip_prefix(':').map_or((false, cell.as_str()), |rest| (true, rest));
            let (right, dashes) = rest.strip_suffix(':').map_or((false, rest), |dashes| (true, dashes));
            let align = match (left, right) {
                (true, true) => Align::Center,
                (false, true) => Align::Right,
                _ => Align::Left,
            };
            (!dashes.is_empty() && dashes.chars().all(|c| c == '-')).then_some(align)
        })
        .collect()
}

fn fill(row: Vec<String>, columns: usize) -> Vec<String> {
    row.into_iter().chain(std::iter::repeat(String::new())).take(columns).collect()
}

fn bold(cell: &Cell) -> Cell {
    cell.iter().map(|span| span.clone().patch_style(Modifier::BOLD)).collect()
}

fn natural_widths(header: &[Cell], body: &[Vec<Cell>]) -> Vec<usize> {
    (0..header.len()).map(|i| body.iter().map(|row| cell_width(&row[i])).fold(cell_width(&header[i]), usize::max)).collect()
}

/// Column widths that fit `width`, taking one column from the widest until they do.
fn fit(natural: &[usize], width: usize, min: usize) -> Option<Vec<usize>> {
    let gaps = GAP.width() * natural.len().saturating_sub(1);
    let mut widths = natural.to_vec();
    while widths.iter().sum::<usize>() + gaps > width {
        let widest = widths.iter().enumerate().max_by_key(|(_, w)| **w).map(|(i, _)| i)?;
        if widths[widest] <= min {
            return None;
        }
        widths[widest] -= 1;
    }
    Some(widths)
}

fn row_line(row: &[Cell], widths: &[usize], aligns: &[Align], border: Style) -> Line<'static> {
    let cells = row.iter().zip(widths).zip(aligns).map(|((cell, width), align)| place(cell, *width, *align));
    let spans = cells.enumerate().flat_map(|(i, cell)| (i > 0).then(|| Span::styled(GAP, border)).into_iter().chain(cell));
    Line::from(spans.collect::<Vec<_>>())
}

/// A cell cut to `width`, then padded on the side its alignment leaves open.
fn place(cell: &Cell, width: usize, align: Align) -> Cell {
    let cell = cut(cell, width);
    let room = width.saturating_sub(cell_width(&cell));
    let (before, after) = match align {
        Align::Left => (0, room),
        Align::Center => (room / 2, room - room / 2),
        Align::Right => (room, 0),
    };
    std::iter::once(Span::raw(" ".repeat(before))).chain(cell).chain(std::iter::once(Span::raw(" ".repeat(after)))).collect()
}

fn cut(cell: &Cell, width: usize) -> Cell {
    if cell_width(cell) <= width {
        return cell.clone();
    }
    let mut room = width.saturating_sub(1);
    let mut kept = vec![];
    for span in cell {
        let text = prefix(&span.content, room);
        room -= text.width();
        let whole = text.len() == span.content.len();
        kept.push(Span::styled(text, span.style));
        if !whole {
            break;
        }
    }
    kept.push(Span::raw("…"));
    kept
}

/// The longest start of `text` at most `room` columns wide.
fn prefix(text: &str, room: usize) -> String {
    text.chars()
        .scan(0, |used, c| {
            *used += c.width().unwrap_or(0);
            (*used <= room).then_some(c)
        })
        .collect()
}

fn cell_width(cell: &Cell) -> usize {
    cell.iter().map(Span::width).sum()
}

/// A raw markdown table line as the diff shows it, its text untouched: only its drawing changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum TableLine {
    Cells,
    Delimiter,
}

impl TableLine {
    pub(super) fn of(text: &str) -> Option<Self> {
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
pub(super) fn wrapped(spans: &[Span], line: TableLine, width: usize, fill: Style, theme: Theme) -> Option<Vec<Vec<Span<'static>>>> {
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
    let widths = fit(&natural, width.checked_sub(width_of(lead) + width_of(tail) + EDGES)?, MIN_WRAPPED)?;
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
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn text(lines: &[Line]) -> Vec<String> {
        lines.iter().map(|l| l.spans.iter().map(|s| s.content.to_string()).collect()).collect()
    }

    fn draw(rows: &str, width: usize) -> Option<Vec<String>> {
        table_lines(&rows.lines().collect::<Vec<_>>(), width, Theme::default()).map(|lines| text(&lines))
    }

    #[test]
    fn cells_split_on_unescaped_pipes_without_the_outer_ones() {
        assert_eq!(cells("| a | `b` |"), ["a", "`b`"]);
        assert_eq!(cells("|a \\| b|c"), ["a | b", "c"]);
        assert_eq!(cells("| a | b \\|"), ["a", "b |"]);
        assert_eq!(cells("|  | x |"), ["", "x"]);
    }

    #[test]
    fn the_delimiter_row_gives_each_column_its_alignment() {
        let row = cells("| --- | :-: | --: | :-- |");
        assert_eq!(aligns(&row), Some(vec![Align::Left, Align::Center, Align::Right, Align::Left]));
        assert_eq!(aligns(&cells("| --- | abc |")), None);
        assert_eq!(aligns(&cells("| : |")), None);
    }

    #[test]
    fn without_a_delimiter_row_the_lines_are_no_table() {
        assert_eq!(draw("| a | b |\n| c | d |", 80), None);
        assert_eq!(draw("| a | b |\n| --- |", 80), None, "the delimiter row has one cell per header");
        assert_eq!(draw("| a | b |", 80), None);
    }

    #[test]
    fn short_rows_are_padded_and_long_ones_cut_to_the_header() {
        let lines = draw("| a | b |\n|---|---|\n| 1 |\n| 1 | 2 | 3 |", 80).unwrap();
        assert_eq!(lines[2], "1 │  ");
        assert_eq!(lines[3], "1 │ 2");
    }

    #[test]
    fn a_table_wider_than_the_pane_cuts_its_widest_columns() {
        let rows = "| name | note |\n|---|---|\n| alpha | a rather long sentence |";
        assert_eq!(draw(rows, 20).unwrap(), ["name  │ note        ", "──────┼─────────────", "alpha │ a rather lo…"]);
        assert!(draw(rows, 20).unwrap().iter().all(|line| line.width() <= 20));
        assert_eq!(draw(rows, 6), None, "two columns cannot fit in six");
    }

    fn boxed_text(text: &str) -> Option<String> {
        let line = TableLine::of(text)?;
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
        assert_eq!(TableLine::of("a | b"), None);
        assert_eq!(TableLine::of("---"), None);
        assert_eq!(TableLine::of(""), None);
    }

    #[test]
    fn box_lines_keep_every_width_and_every_style_but_the_glyph_colour() {
        let theme = Theme::default();
        let word = Style::default().bg(ratatui::style::Color::Red);
        let spans = vec![Span::raw("| a "), Span::styled("| b", word), Span::raw(" |")];
        let drawn = boxed(spans.clone(), TableLine::Cells, theme);
        let widths = |spans: &[Span]| spans.iter().map(Span::width).sum::<usize>();
        assert_eq!(widths(&drawn), widths(&spans));
        let glyph = drawn.iter().find(|s| s.content == "│" && s.style.bg == word.bg).unwrap();
        assert_eq!(glyph.style.fg, Some(theme.faded), "the pipe of a changed word keeps its fill");
    }

    fn wrapped_text(text: &str, width: usize) -> Option<Vec<String>> {
        let rows = wrapped(&[Span::raw(text.to_owned())], TableLine::of(text)?, width, Style::default(), Theme::default())?;
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

    #[test]
    fn a_rendered_table() {
        let rows = "| Name | Count | State |\n| :--- | ---: | :-: |\n| alpha | 3 | `ok` |\n| beta | 12 | failed |";
        insta::assert_snapshot!("rendered_table", draw(rows, 80).unwrap().join("\n"));
    }
}
