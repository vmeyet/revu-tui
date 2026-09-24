//! Sticky headers: the file row, and the hunk row under the cursor, stay pinned above the diff
//! once they scroll off, so a long file never loses its name.
use crate::review::Row;

/// Below this many diff rows the pins would eat too much of the view.
pub const MIN_HEIGHT: usize = 20;

/// The rows pinned above the diff, as indexes into the review's rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pins {
    pub file: Option<usize>,
    pub hunk: Option<usize>,
}

impl Pins {
    /// How many screen rows the pins take.
    pub fn count(self) -> usize {
        usize::from(self.file.is_some()) + usize::from(self.hunk.is_some())
    }
}

/// The file header owning the top row, when it sits above the view; then the header of the
/// cursor's hunk, when that also sits above the view and the cursor is in the pinned file.
pub fn pins(rows: &[Row], scroll: usize, selected: usize) -> Pins {
    let Some(file) = (0..=scroll.min(rows.len().saturating_sub(1))).rev().find(|&i| matches!(rows[i], Row::File { .. })) else {
        return Pins::default();
    };
    if file >= scroll {
        return Pins::default();
    }
    Pins { file: Some(file), hunk: hunk_above(rows, file, scroll, selected) }
}

/// Where the view starts once the pins take their rows: the cursor always stays below them.
/// A few rounds settle it, since moving the view can change which headers are pinned.
pub fn settle(rows: &[Row], scroll: usize, selected: usize, height: usize) -> (usize, Pins) {
    let mut scroll = super::super::ui::settle_scroll(scroll, selected, height);
    let mut pinned = pins(rows, scroll, selected);
    for _ in 0..3 {
        let next = super::super::ui::settle_scroll(scroll, selected, height.saturating_sub(pinned.count()));
        let next_pins = pins(rows, next, selected);
        if next == scroll && next_pins == pinned {
            break;
        }
        (scroll, pinned) = (next, next_pins);
    }
    (scroll, pinned)
}

fn hunk_above(rows: &[Row], file: usize, scroll: usize, selected: usize) -> Option<usize> {
    let Row::File { index: file_index, .. } = rows[file] else { return None };
    let (in_file, hunk) = match rows.get(selected)? {
        Row::Line { file, hunk, .. } | Row::Pair { file, hunk, .. } | Row::Context { file, hunk, .. } => (*file, *hunk),
        _ => return None,
    };
    if in_file != file_index || selected < scroll {
        return None;
    }
    (file..scroll).find(|&i| matches!(rows[i], Row::Hunk { file: f, index, .. } if f == file_index && index == hunk))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    /// Two files: the first with two hunks of four lines, the second with one hunk of two.
    fn rows() -> Vec<Row> {
        let line = |file, hunk, index| Row::Line { file, hunk, index };
        vec![
            Row::Gap,                                    // 0
            Row::File { index: 0, open: true },          // 1
            Row::Hunk { file: 0, index: 0, open: true }, // 2
            line(0, 0, 0),                               // 3
            line(0, 0, 1),                               // 4
            line(0, 0, 2),                               // 5
            line(0, 0, 3),                               // 6
            Row::Hunk { file: 0, index: 1, open: true }, // 7
            line(0, 1, 0),                               // 8
            line(0, 1, 1),                               // 9
            line(0, 1, 2),                               // 10
            line(0, 1, 3),                               // 11
            Row::Gap,                                    // 12
            Row::File { index: 1, open: true },          // 13
            Row::Hunk { file: 1, index: 0, open: true }, // 14
            line(1, 0, 0),                               // 15
            line(1, 0, 1),                               // 16
        ]
    }

    #[test]
    fn nothing_is_pinned_while_the_file_header_is_on_screen() {
        assert_eq!(pins(&rows(), 0, 4), Pins::default());
        assert_eq!(pins(&rows(), 1, 4), Pins::default());
        assert_eq!(pins(&rows(), 13, 15), Pins::default(), "the next file's header at the top replaces the pin");
    }

    #[test]
    fn the_file_is_pinned_once_its_header_scrolls_off() {
        assert_eq!(pins(&rows(), 2, 3), Pins { file: Some(1), hunk: None }, "the hunk header is still visible");
        assert_eq!(pins(&rows(), 12, 13), Pins { file: Some(1), hunk: None }, "the gap before the next file still belongs to this one");
    }

    #[test]
    fn the_hunk_is_pinned_too_when_the_cursor_is_inside_it_and_its_header_is_gone() {
        assert_eq!(pins(&rows(), 4, 5), Pins { file: Some(1), hunk: Some(2) });
        assert_eq!(pins(&rows(), 9, 10), Pins { file: Some(1), hunk: Some(7) });
        assert_eq!(pins(&rows(), 9, 12), Pins { file: Some(1), hunk: None }, "the cursor on a gap is in no hunk");
        assert_eq!(pins(&rows(), 15, 16), Pins { file: Some(13), hunk: Some(14) });
    }

    #[test]
    fn a_cursor_on_the_file_or_hunk_row_never_pins_a_hunk() {
        assert_eq!(pins(&rows(), 3, 7), Pins { file: Some(1), hunk: None });
    }

    #[test]
    fn the_cursor_is_never_hidden_under_the_pins() {
        let rows = rows();
        for height in 3..10 {
            for selected in 0..rows.len() {
                for from in 0..rows.len() {
                    let (scroll, pinned) = settle(&rows, from, selected, height);
                    assert!(selected >= scroll, "cursor above the view: h={height} sel={selected} from={from}");
                    assert!(selected < scroll + height - pinned.count(), "cursor under the fold: h={height} sel={selected} from={from}");
                    assert_eq!(pinned, pins(&rows, scroll, selected), "the returned pins match the view");
                }
            }
        }
    }

    #[test]
    fn pins_count_their_rows() {
        assert_eq!(Pins::default().count(), 0);
        assert_eq!(Pins { file: Some(1), hunk: Some(2) }.count(), 2);
    }
}
