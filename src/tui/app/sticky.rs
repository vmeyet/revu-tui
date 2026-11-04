//! Sticky headers: the file row, and the hunk row under the cursor, stay on top of the diff
//! once they scroll off, so a long file never loses its name.
use crate::review::Row;

/// Below this many diff rows sticky headers would eat too much of the view.
pub(crate) const MIN_HEIGHT: usize = 20;

/// The rows kept on top of the diff, as indexes into the review's rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct StickyRows {
    pub(crate) file: Option<usize>,
    pub(crate) hunk: Option<usize>,
}

impl StickyRows {
    /// How many screen rows they take.
    pub(crate) fn count(self) -> usize {
        usize::from(self.file.is_some()) + usize::from(self.hunk.is_some())
    }
}

/// The file header owning the top row, when it sits above the view; then the header of the
/// cursor's hunk, when that also sits above the view and the cursor is in the sticky file.
pub(crate) fn sticky_rows(rows: &[Row], scroll: usize, selected: usize) -> StickyRows {
    let Some(file) = (0..=scroll.min(rows.len().saturating_sub(1))).rev().find(|&i| matches!(rows[i], Row::File { .. })) else {
        return StickyRows::default();
    };
    if file >= scroll {
        return StickyRows::default();
    }
    StickyRows { file: Some(file), hunk: hunk_above(rows, file, scroll, selected) }
}

/// Where the view starts once sticky headers take their rows: the cursor always stays below them.
/// A few rounds settle it, since moving the view can change which headers stick.
pub(crate) fn settle(rows: &[Row], scroll: usize, selected: usize, height: usize) -> (usize, StickyRows) {
    let mut scroll = super::super::ui::settle_scroll(scroll, selected, height);
    let mut sticky = sticky_rows(rows, scroll, selected);
    for _ in 0..3 {
        let next = super::super::ui::settle_scroll(scroll, selected, height.saturating_sub(sticky.count()));
        let next_sticky = sticky_rows(rows, next, selected);
        if next == scroll && next_sticky == sticky {
            break;
        }
        (scroll, sticky) = (next, next_sticky);
    }
    (scroll, sticky)
}

fn hunk_above(rows: &[Row], file: usize, scroll: usize, selected: usize) -> Option<usize> {
    let Row::File { index: file_index, .. } = rows[file] else { return None };
    let (in_file, hunk) = match rows.get(selected)? {
        Row::Line { file, hunk, .. } | Row::Pair { file, hunk, .. } | Row::Context { file, hunk, .. } | Row::Thread { file, hunk, .. } => {
            (*file, *hunk)
        }
        _ => return None,
    };
    if in_file != file_index || selected < scroll {
        return None;
    }
    (file..scroll).find(|&i| matches!(rows[i], Row::Hunk { file: f, index, .. } if f == file_index && index == hunk))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::test_support::*;

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
    fn nothing_sticks_while_the_file_header_is_on_screen() {
        assert_eq!(sticky_rows(&rows(), 0, 4), StickyRows::default());
        assert_eq!(sticky_rows(&rows(), 1, 4), StickyRows::default());
        assert_eq!(sticky_rows(&rows(), 13, 15), StickyRows::default(), "the next file's header at the top replaces the sticky one");
    }

    #[test]
    fn the_file_sticks_once_its_header_scrolls_off() {
        assert_eq!(sticky_rows(&rows(), 2, 3), StickyRows { file: Some(1), hunk: None }, "the hunk header is still visible");
        assert_eq!(
            sticky_rows(&rows(), 12, 13),
            StickyRows { file: Some(1), hunk: None },
            "the gap before the next file still belongs to this one"
        );
    }

    #[test]
    fn the_hunk_sticks_too_when_the_cursor_is_inside_it_and_its_header_is_gone() {
        assert_eq!(sticky_rows(&rows(), 4, 5), StickyRows { file: Some(1), hunk: Some(2) });
        assert_eq!(sticky_rows(&rows(), 9, 10), StickyRows { file: Some(1), hunk: Some(7) });
        assert_eq!(sticky_rows(&rows(), 9, 12), StickyRows { file: Some(1), hunk: None }, "the cursor on a gap is in no hunk");
        assert_eq!(sticky_rows(&rows(), 15, 16), StickyRows { file: Some(13), hunk: Some(14) });
    }

    #[test]
    fn a_cursor_on_the_file_or_hunk_row_never_makes_a_hunk_stick() {
        assert_eq!(sticky_rows(&rows(), 3, 7), StickyRows { file: Some(1), hunk: None });
    }

    #[test]
    fn the_cursor_is_never_hidden_under_sticky_headers() {
        let rows = rows();
        for height in 3..10 {
            for selected in 0..rows.len() {
                for from in 0..rows.len() {
                    let (scroll, sticky) = settle(&rows, from, selected, height);
                    assert!(selected >= scroll, "cursor above the view: h={height} sel={selected} from={from}");
                    assert!(selected < scroll + height - sticky.count(), "cursor under the fold: h={height} sel={selected} from={from}");
                    assert_eq!(sticky, sticky_rows(&rows, scroll, selected), "the returned sticky rows match the view");
                }
            }
        }
    }

    #[test]
    fn sticky_rows_count_their_rows() {
        assert_eq!(StickyRows::default().count(), 0);
        assert_eq!(StickyRows { file: Some(1), hunk: Some(2) }.count(), 2);
    }

    /// The text of the line under the cursor, as the diff draws it.
    fn cursor_text(app: &App) -> String {
        let open = app.open.as_ref().unwrap();
        match open.row() {
            Some(Row::Line { file, hunk, index }) => open.review.files[*file].hunks[*hunk].lines[*index].text.trim().to_owned(),
            _ => String::new(),
        }
    }

    #[test]
    fn the_cursor_line_stays_on_screen_below_sticky_headers() {
        let mut app = with_long_review();
        for _ in 0..70 {
            press(&mut app, "j");
            let screen = render(&mut app, 120, 30);
            let text = cursor_text(&app);
            assert!(text.is_empty() || screen.contains(&text), "`{text}` is hidden:\n{screen}");
        }
    }

    #[test]
    fn za_under_a_sticky_file_folds_that_file_and_lands_on_its_row() {
        let mut app = with_long_review();
        to_step(&mut app, 40);
        render(&mut app, 120, 30);
        assert!(app.open.as_ref().unwrap().sticky_file.is_some());
        press(&mut app, "za");
        let open = app.open.as_ref().unwrap();
        assert!(!open.review.fold.file_is_open("src/pay/charge.rs"));
        assert!(matches!(open.row(), Some(Row::File { index: 0, .. })), "{:?}", open.row());
        assert_eq!(open.sticky_file, None);
    }

    #[test]
    fn zo_under_a_sticky_file_keeps_its_usual_meaning() {
        let mut app = with_long_review();
        to_step(&mut app, 40);
        render(&mut app, 120, 30);
        press(&mut app, "zo");
        assert!(app.open.as_ref().unwrap().review.fold.file_is_open("src/pay/charge.rs"));
    }
}
