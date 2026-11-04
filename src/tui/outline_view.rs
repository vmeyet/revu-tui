//! The outline pane: the changed functions.
use super::app::{App, Focus, Symbols};
use super::ui::{draw_empty, side_pane};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use vtui::text::truncate;

pub(crate) fn draw(f: &mut Frame<'_>, app: &App, area: Rect) {
    let theme = app.theme;
    let Some(open) = &app.open else { return };
    let Some(outline) = &open.outline else { return };
    let block = side_pane(theme, "Outline", app.focus == Focus::Side, app.zen);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let tick = app.spinner();
    match &outline.symbols {
        Symbols::Waiting => return draw_empty(f, theme, inner, &[tick, "", "reading the symbols"]),
        Symbols::Failed(message) => return draw_empty(f, theme, inner, &["outline unreadable", message, "", "r to retry"]),
        Symbols::Ready(_) => {}
    }
    let Symbols::Ready(reading) = &outline.symbols else { return };
    if reading.changes.is_empty() {
        return draw_empty(f, theme, inner, &["no function changed"]);
    }
    let width = inner.width as usize;
    let shown: Vec<Line<'_>> = reading
        .changes
        .iter()
        .take(inner.height as usize)
        .map(|change| Line::raw(truncate(&format!(" {}: {}", change.path, change.name), width)))
        .collect();
    f.render_widget(Paragraph::new(shown), inner);
}
