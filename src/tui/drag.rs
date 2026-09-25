//! A drag selects text in one pane, the way a terminal would, but only the text: never a gutter,
//! a sign or the pane beside it. What it copies is the raw text under the drawing.
use ratatui::buffer::Buffer;
use ratatui::layout::Position;
use ratatui::style::Modifier;
use std::ops::Range;
use unicode_width::UnicodeWidthChar;

/// One screen row of text a drag can select, as the last frame drew it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextRow {
    /// The cell its text starts at; the rows of one column all start at the same `x`.
    pub at: Position,
    /// The cells the column gives its text.
    pub width: u16,
    /// Rows of one line share it: a wrapped line copies back as one.
    pub line: usize,
    /// The raw text of the whole line.
    pub text: String,
    /// The bytes of `text` under each cell of this row; empty where the drawing adds a glyph of its own.
    pub cells: Vec<Range<usize>>,
}

impl TextRow {
    /// The bytes `cells` of this row stand for; the first row of a line reaches back to its start
    /// and the last reaches on to its end, past what the drawing cut or dropped.
    fn bytes(&self, cells: Range<usize>, starts_line: bool, ends_line: bool) -> Range<usize> {
        let end_of_row = if ends_line { self.text.len() } else { self.cells.last().map_or(0, |c| c.end) };
        let from = if cells.start == 0 && starts_line { 0 } else { self.cells.get(cells.start).map_or(end_of_row, |c| c.start) };
        let to = if cells.end >= self.cells.len() { end_of_row } else { self.cells[cells.end - 1].end };
        from..to.max(from)
    }
}

/// The bytes under each cell of `text` drawn as the diff draws it, a tab `tab` cells wide.
pub fn cells_of(text: &str, tab: usize) -> Vec<Range<usize>> {
    let mut cells: Vec<Range<usize>> = vec![];
    for (at, c) in text.char_indices() {
        let bytes = at..at + c.len_utf8();
        match if c == '\t' { tab } else { c.width().unwrap_or(0) } {
            0 => match cells.last_mut() {
                Some(last) => last.end = bytes.end,
                None => cells.push(bytes),
            },
            width => cells.extend(std::iter::repeat_n(bytes, width)),
        }
    }
    cells
}

/// What a drag covers: from the cell it started on to the cell under the pointer, in one column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drag {
    column: u16,
    from: Position,
    to: Position,
}

impl Drag {
    /// A press on a row of text starts a drag there; anywhere else it starts nothing.
    pub fn start(rows: &[TextRow], at: Position) -> Option<Self> {
        let row = rows.iter().find(|row| row.at.y == at.y && (row.at.x..row.at.x + row.width).contains(&at.x))?;
        Some(Self { column: row.at.x, from: at, to: at })
    }

    /// The pointer moved to `at`: the drag follows it, held inside its column.
    #[must_use]
    pub fn moved_to(self, rows: &[TextRow], at: Position) -> Self {
        let column: Vec<&TextRow> = self.rows(rows).collect();
        let (Some(first), Some(last)) = (column.first(), column.last()) else { return self };
        let right = self.column + first.width.saturating_sub(1);
        let to = match at.y {
            y if y < first.at.y => Position::new(self.column, first.at.y),
            y if y > last.at.y => Position::new(right, last.at.y),
            y => Position::new(at.x.clamp(self.column, right), y),
        };
        Self { to, ..self }
    }

    /// A press and a release on the same cell is a click, not a drag.
    pub fn is_click(self) -> bool {
        self.from == self.to
    }

    /// The raw text selected: rows of one line joined as they were, lines by a newline. A row
    /// selected from its first cell or through its last takes the whole start or end of its line.
    pub fn text(self, rows: &[TextRow]) -> String {
        let column: Vec<&TextRow> = self.rows(rows).collect();
        let mut text = String::new();
        let mut line = None;
        for (i, row) in column.iter().enumerate() {
            let Some(cells) = self.cells_on(row) else { continue };
            if line.is_some_and(|line| line != row.line) {
                text.push('\n');
            }
            line = Some(row.line);
            let starts_line = i == 0 || column[i - 1].line != row.line;
            let ends_line = column.get(i + 1).is_none_or(|next| next.line != row.line);
            text.push_str(row.text.get(row.bytes(cells, starts_line, ends_line)).unwrap_or_default());
        }
        text
    }

    /// Reverses the selected cells of `buffer`.
    pub fn paint(self, rows: &[TextRow], buffer: &mut Buffer) {
        for row in self.rows(rows) {
            let Some(cells) = self.cells_on(row) else { continue };
            for cell in cells.start..cells.end.min(row.cells.len()) {
                let at = Position::new(row.at.x + cell as u16, row.at.y);
                if let Some(cell) = buffer.cell_mut(at) {
                    cell.modifier.insert(Modifier::REVERSED);
                }
            }
        }
    }

    fn rows(self, rows: &[TextRow]) -> impl Iterator<Item = &TextRow> {
        rows.iter().filter(move |row| row.at.x == self.column)
    }

    /// The cells of `row` the drag covers, the last one open-ended when the drag goes on below.
    fn cells_on(self, row: &TextRow) -> Option<Range<usize>> {
        let (top, bottom) = if (self.from.y, self.from.x) <= (self.to.y, self.to.x) { (self.from, self.to) } else { (self.to, self.from) };
        if !(top.y..=bottom.y).contains(&row.at.y) {
            return None;
        }
        let start = if row.at.y == top.y { usize::from(top.x - self.column) } else { 0 };
        let end = if row.at.y == bottom.y { usize::from(bottom.x - self.column) + 1 } else { usize::MAX };
        Some(start..end)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    /// Rows of plain text in a column at x 10, one line per row unless `line` says they wrap.
    fn row(y: u16, line: usize, text: &str) -> TextRow {
        TextRow { at: Position::new(10, y), width: 20, line, text: text.to_owned(), cells: cells_of(text, 4) }
    }

    fn selected(rows: &[TextRow], from: (u16, u16), to: (u16, u16)) -> String {
        let drag = Drag::start(rows, Position::new(from.0, from.1)).unwrap();
        drag.moved_to(rows, Position::new(to.0, to.1)).text(rows)
    }

    #[test]
    fn a_tab_takes_its_cells_and_a_wide_character_two() {
        assert_eq!(cells_of("\ta", 4), [0..1, 0..1, 0..1, 0..1, 1..2]);
        assert_eq!(cells_of("é漢", 4), [0..2, 2..5, 2..5]);
        assert_eq!(cells_of("e\u{301}x", 4), [0..3, 3..4], "a combining accent stays on its letter");
    }

    #[test]
    fn a_drag_on_one_row_copies_the_characters_under_it() {
        let rows = [row(0, 0, "let total = 1;")];
        assert_eq!(selected(&rows, (14, 0), (18, 0)), "total");
        assert_eq!(selected(&rows, (18, 0), (14, 0)), "total", "a drag to the left selects the same cells");
    }

    #[test]
    fn a_drag_over_several_lines_joins_them_by_newlines_and_takes_whole_middles() {
        let rows = [row(0, 0, "one two"), row(1, 1, "three"), row(2, 2, "four five")];
        assert_eq!(selected(&rows, (14, 0), (13, 2)), "two\nthree\nfour");
    }

    #[test]
    fn a_wrapped_line_copies_back_as_one_line() {
        let text = "abcdefgh";
        let wrapped = |y: u16, cells: Range<usize>| TextRow { cells: cells_of(text, 4)[cells].to_vec(), ..row(y, 0, text) };
        let rows = [wrapped(0, 0..4), wrapped(1, 4..8)];
        assert_eq!(selected(&rows, (12, 0), (11, 1)), "cdef");
        assert_eq!(selected(&rows, (10, 0), (19, 1)), "abcdefgh");
    }

    #[test]
    fn a_row_selected_to_its_edge_takes_the_rest_of_the_raw_line() {
        let text = "| a | b |";
        let drawn = TextRow { cells: vec![2..3, 3..4, 4..4, 5..6, 6..7], ..row(0, 0, text) };
        let rows = [drawn];
        assert_eq!(selected(&rows, (10, 0), (19, 0)), text, "the pipes the drawing dropped come back");
        assert_eq!(selected(&rows, (11, 0), (11, 0)), " ");
    }

    #[test]
    fn a_drag_stays_in_the_column_it_started_in() {
        let beside = TextRow { at: Position::new(40, 1), ..row(1, 9, "other pane") };
        let rows = [row(0, 0, "old"), row(1, 1, "new"), beside];
        let drag = Drag::start(&rows, Position::new(11, 0)).unwrap().moved_to(&rows, Position::new(45, 1));
        assert_eq!(drag.text(&rows), "ld\nnew");
        let above = Drag::start(&rows, Position::new(11, 1)).unwrap().moved_to(&rows, Position::new(3, 0));
        assert_eq!(above.text(&rows), "old\nne", "a drag out of the column clamps to its edge");
        assert_eq!(Drag::start(&rows, Position::new(5, 0)), None, "a press on the gutter starts nothing");
    }

    #[test]
    fn rows_without_text_between_lines_contribute_nothing() {
        let rows = [row(0, 0, "fn a() {"), row(3, 1, "}")];
        assert_eq!(selected(&rows, (10, 0), (10, 3)), "fn a() {\n}");
    }

    #[test]
    fn a_press_without_a_drag_is_a_click() {
        let rows = [row(0, 0, "text")];
        let drag = Drag::start(&rows, Position::new(11, 0)).unwrap();
        assert!(drag.is_click());
        assert!(!drag.moved_to(&rows, Position::new(12, 0)).is_click());
    }

    #[test]
    fn painting_reverses_only_the_selected_text_cells() {
        let rows = [row(0, 0, "ab"), row(1, 1, "cd")];
        let mut buffer = Buffer::empty(ratatui::layout::Rect::new(0, 0, 30, 2));
        Drag::start(&rows, Position::new(11, 0)).unwrap().moved_to(&rows, Position::new(25, 1)).paint(&rows, &mut buffer);
        let reversed: Vec<(u16, u16)> = buffer
            .content
            .iter()
            .enumerate()
            .filter(|(_, c)| c.modifier.contains(Modifier::REVERSED))
            .map(|(i, _)| buffer.pos_of(i))
            .collect();
        assert_eq!(reversed, [(11, 0), (10, 1), (11, 1)]);
    }
}
