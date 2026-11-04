//! The outline pane: the changed symbols by file.
use super::app::{App, Focus, PaneLine, Symbols};
use super::theme::Theme;
use super::ui::{draw_empty, side_pane};
use crate::outline::{Change, State};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
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
        return draw_empty(f, theme, inner, &["no function or class changed"]);
    }
    let width = inner.width as usize;
    let shown: Vec<Line<'_>> = outline
        .rows
        .iter()
        .take(inner.height as usize)
        .map(|line| match line {
            PaneLine::Blank => Line::default(),
            PaneLine::Path(path) => {
                Line::from(Span::styled(format!(" {}", truncate(path, width.saturating_sub(1))), Style::default().fg(theme.faded)))
            }
            PaneLine::Change(index) => change_line(&reading.changes[*index], width, theme),
        })
        .collect();
    f.render_widget(Paragraph::new(shown), inner);
}

/// A changed symbol with its sign.
fn change_line<'a>(change: &Change, width: usize, theme: Theme) -> Line<'a> {
    let (glyph, colour) = glyph(change.state, theme);
    let name = truncate(&change.symbol.name, width.saturating_sub(4));
    Line::from(vec![Span::raw("  "), Span::styled(format!("{glyph} "), Style::default().fg(colour)), Span::raw(name)])
}

fn glyph(state: State, theme: Theme) -> (&'static str, Color) {
    match state {
        State::Removed => ("-", theme.danger),
        State::Added => ("+", theme.success),
        State::Changed => ("·", theme.faded),
    }
}
