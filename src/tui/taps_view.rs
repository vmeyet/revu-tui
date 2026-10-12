//! The tap bar's row: the labels of the page side by side, each in a cell wide enough for a thumb.
use super::app::{App, Tap, TapSpot};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

/// A blank column on each side of a label, part of what a tap hits.
const PAD: usize = 1;

fn cell_width(tap: &Tap) -> usize {
    tap.label.width() + 2 * PAD
}

/// The labels that fit `width`: the last one, which opens every command, and as many of those before it as go in
/// front of it.
fn fitting(taps: &'static [Tap], width: usize) -> Vec<&'static Tap> {
    let Some((last, rest)) = taps.split_last() else { return vec![] };
    let room = width.saturating_sub(cell_width(last));
    let mut used = 0;
    let kept = rest.iter().take_while(|tap| {
        used += cell_width(tap);
        used <= room
    });
    kept.chain([last]).collect()
}

/// A cell per label, left to right from the start of `area`, each sharing what the row has to spare.
fn spots(taps: &[&'static Tap], area: Rect) -> Vec<TapSpot> {
    let natural: usize = taps.iter().map(|tap| cell_width(tap)).sum();
    let spare = usize::from(area.width).saturating_sub(natural) / taps.len().max(1);
    let spot = |x: &mut u16, tap: &&'static Tap| {
        let width = (cell_width(tap) + spare) as u16;
        let spot = TapSpot { area: Rect { x: *x, width, ..area }, keys: tap.keys };
        *x += width;
        Some(spot)
    };
    taps.iter().scan(area.x, spot).collect()
}

pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    let taps = fitting(app.page().taps(), usize::from(area.width));
    let spots = spots(&taps, area);
    let style = Style::default().fg(app.theme.muted).bg(app.theme.surface);
    let cells =
        taps.iter().zip(&spots).map(|(tap, spot)| Span::raw(format!("{:^width$}", tap.label, width = usize::from(spot.area.width))));
    f.render_widget(Paragraph::new(Line::from(cells.collect::<Vec<_>>())).style(style), area);
    app.taps = spots;
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAPS: [Tap; 3] = [
        Tap { label: "open", keys: "enter", does: "" },
        Tap { label: "filter", keys: "/", does: "" },
        Tap { label: "more", keys: ":", does: "" },
    ];

    fn labels(width: usize) -> Vec<&'static str> {
        fitting(&TAPS, width).into_iter().map(|tap| tap.label).collect()
    }

    #[test]
    fn the_last_label_stays_and_the_ones_before_it_go_when_the_row_is_short() {
        assert_eq!(labels(20), ["open", "filter", "more"], "6 + 8 + 6 columns");
        assert_eq!(labels(19), ["open", "more"]);
        assert_eq!(labels(11), ["more"]);
        assert_eq!(labels(2), ["more"], "even cut, it is the way to every command");
    }

    #[test]
    fn the_cells_touch_and_share_the_spare_columns() {
        let cells = spots(&fitting(&TAPS, 30), Rect::new(2, 9, 30, 1));
        let places: Vec<(u16, u16)> = cells.iter().map(|spot| (spot.area.x, spot.area.width)).collect();
        assert_eq!(places, [(2, 9), (11, 11), (22, 9)], "ten spare columns, three each");
        assert!(cells.iter().all(|spot| (spot.area.y, spot.area.height) == (9, 1)));
        assert_eq!(cells.iter().map(|spot| spot.keys).collect::<Vec<_>>(), ["enter", "/", ":"]);
    }
}
