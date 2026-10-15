//! The text a drag can select in each diff row, cell by cell.
use super::rows::context_line;
use super::side_by_side::{HALF_INDENT, Half, half_widths, halves};
use super::{Gutter, TAB};
use crate::diff::Line as DiffLine;
use crate::diff::words::{Segment, same_but_whitespace, segments};
use crate::review::{Review, Row};
use crate::tui::drag::{self, TextRow};
use ratatui::layout::{Position, Rect};
use std::ops::Range;
use unicode_width::UnicodeWidthStr;

/// The rows of text a drag can select in diff row `index`, drawn from the top of `area`.
pub(super) fn text_rows(review: &Review, row: &Row, index: usize, area: Rect, gutter: Gutter, wrap: bool) -> Vec<TextRow> {
    text_columns(review, row, gutter, usize::from(area.width)).into_iter().flat_map(|column| column.rows(area, index, wrap)).collect()
}

/// One column of text in a diff row: how far past the row's start its text is drawn, the cells it has, what it shows.
struct Column {
    x: usize,
    room: usize,
    text: String,
    cells: Vec<Range<usize>>,
}

impl Column {
    /// Its screen rows from the top of `area`, cut or wrapped as the row is.
    fn rows(self, area: Rect, line: usize, wrap: bool) -> Vec<TextRow> {
        let (x, width) = (area.x + self.x as u16, self.room as u16);
        let rows = screen_rows(&self.cells, self.room, wrap).into_iter().zip(area.y..);
        rows.map(|(cells, y)| TextRow { at: Position::new(x, y), width, line, text: self.text.clone(), cells: cells.to_vec() }).collect()
    }
}

/// The inline text of a diff row, or each half side by side.
fn text_columns(review: &Review, row: &Row, gutter: Gutter, width: usize) -> Vec<Column> {
    let Some(halves) = halves(review, row).filter(|_| review.shows_side_by_side()) else {
        let indent = gutter.wrap_indent();
        let column = |(text, cells)| Column { x: indent, room: width.saturating_sub(indent), text, cells };
        return inline_text(review, row).map(column).into_iter().collect();
    };
    let (old_w, new_w) = half_widths(width);
    let spans = [(1, old_w), (1 + old_w, new_w)];
    let column = |half: Option<Half<'_>>, (x, w): (usize, usize)| {
        let text = half?.line.text;
        Some(Column { x: x + HALF_INDENT, room: w.saturating_sub(HALF_INDENT), cells: drag::cells_of(&text, TAB.width()), text })
    };
    halves.into_iter().zip(spans).filter_map(|(half, span)| column(half, span)).collect()
}

/// The raw text of an inline diff row, and the bytes under each cell of its text column.
fn inline_text(review: &Review, row: &Row) -> Option<(String, Vec<Range<usize>>)> {
    let text = match row {
        Row::Line { file, hunk, index } => review.files[*file].hunks[*hunk].lines[*index].text.clone(),
        Row::Pair { file, hunk, removed, added } => {
            let lines = &review.files[*file].hunks[*hunk].lines;
            let (old, new) = (&lines[*removed], &lines[*added]);
            if !(review.quiet_whitespace && same_but_whitespace(&old.text, &new.text)) {
                return Some((new.text.clone(), pair_cells(old, new)));
            }
            new.text.clone()
        }
        Row::Context { file, old, new, .. } => context_line(review, *file, *old, *new).text,
        Row::Header | Row::File { .. } | Row::Hunk { .. } | Row::Thread { .. } | Row::Gap => return None,
    };
    let cells = drag::cells_of(&text, TAB.width());
    Some((text, cells))
}

/// A removed line and its added twin on one row copy as the added line: the struck old words stand for nothing.
fn pair_cells(old: &DiffLine, new: &DiffLine) -> Vec<Range<usize>> {
    let mut cells = vec![];
    let mut at = 0;
    for part in segments(old, new) {
        match part {
            Segment::Same(text) | Segment::New(text) => {
                cells.extend(drag::cells_of(&text, TAB.width()).into_iter().map(|bytes| bytes.start + at..bytes.end + at));
                at += text.len();
            }
            Segment::Old(text) => cells.extend(drag::cells_of(&text, TAB.width()).into_iter().map(|_| at..at)),
        }
    }
    cells
}

/// `cells` as the screen rows they take: cut to `room`, or wrapped every `room` cells.
fn screen_rows(cells: &[Range<usize>], room: usize, wrap: bool) -> Vec<&[Range<usize>]> {
    if !wrap || cells.len() <= room {
        return vec![&cells[..cells.len().min(room)]];
    }
    cells.chunks(room.max(1)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::diff_view::test_support::{long_added_line, total_pair};

    #[test]
    fn a_drag_starts_where_the_text_does_whatever_the_gutter() {
        let review = long_added_line();
        let row = Row::Line { file: 0, hunk: 0, index: 1 };
        let column = |gutter| text_columns(&review, &row, gutter, 30).into_iter().map(|column| (column.x, column.room)).collect::<Vec<_>>();
        assert_eq!(column(Gutter::Both), [(14, 16)]);
        assert_eq!(column(Gutter::One), [(9, 21)]);
    }

    #[test]
    fn an_inline_pair_selects_as_the_added_line_its_struck_words_standing_for_nothing() {
        let hunk = total_pair();
        let cells = pair_cells(&hunk.lines[0], &hunk.lines[1]);
        assert_eq!(cells.len(), "const total = 1;2;".len());
        assert_eq!(cells[14..], [14..14, 14..14, 14..15, 15..16]);
    }

    #[test]
    fn a_text_column_is_cut_to_its_room_or_wrapped_every_room_cells() {
        let cells = drag::cells_of("abcde", 4);
        assert_eq!(screen_rows(&cells, 3, false), [&cells[..3]]);
        assert_eq!(screen_rows(&cells, 3, true), [&cells[..3], &cells[3..]]);
        assert_eq!(screen_rows(&[], 3, true).len(), 1, "an empty line still takes its row");
    }
}
