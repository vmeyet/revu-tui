//! The outline pane: what breaks, what came and what was renamed, then each file and its changed symbols.
use super::app::{App, Focus, Symbols};
use super::theme::Theme;
use super::ui::{counts, draw_empty, settle_scroll, side_pane, spinner, truncate};
use crate::diff::words::{Segment, text_segments};
use crate::outline::{Change, State};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme;
    let Some(open) = &app.open else { return };
    let Some(outline) = &open.outline else { return };
    let title = if outline.all { "Outline · all" } else { "Outline · public" };
    let block = side_pane(theme, title, app.focus == Focus::Side, app.zen);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let tick = spinner(app.now.duration_since(app.started));
    match &outline.symbols {
        Symbols::Waiting => return draw_empty(f, theme, inner, &[tick, "", "reading the symbols"]),
        Symbols::Failed(message) => return draw_empty(f, theme, inner, &["outline unreadable", message, "", "r to retry"]),
        Symbols::Ready(_) => {}
    }
    let shown = outline.shown();
    if shown.is_empty() {
        let hint = if outline.all { "" } else { "a lists private ones too" };
        return draw_empty(f, theme, inner, &["no function or class changed", hint]);
    }
    let lines = lines(&shown, outline.selected, inner.width as usize, theme);
    let height = inner.height as usize;
    let cursor = lines.iter().position(|(symbol, _)| *symbol == Some(outline.selected)).unwrap_or(0);
    let scroll = settle_scroll(0, cursor, height);
    let lines: Vec<Line> = lines.into_iter().skip(scroll).take(height).map(|(_, line)| line).collect();
    f.render_widget(Paragraph::new(lines), inner);
}

/// Every row of the pane, each with the index of the symbol it shows, if any.
fn lines<'a>(shown: &[&Change], selected: usize, width: usize, theme: Theme) -> Vec<(Option<usize>, Line<'a>)> {
    let mut lines = vec![(None, badge(shown, theme)), (None, Line::default())];
    for (index, change) in shown.iter().enumerate() {
        if index == 0 || shown[index - 1].path != change.path {
            if index > 0 {
                lines.push((None, Line::default()));
            }
            lines.push((
                None,
                Line::from(Span::styled(format!(" {}", truncate(&change.path, width.saturating_sub(1))), Style::default().fg(theme.faded))),
            ));
        }
        lines.push((Some(index), symbol_line(change, index == selected, width, theme)));
        if let (State::Signature, Some(before)) = (change.state, &change.before) {
            lines.push((None, signature_line(&before.signature, &change.symbol.signature, width, theme)));
        }
    }
    lines
}

/// `2 breaking · 3 added · 1 renamed`, zeros left out.
fn badge<'a>(shown: &[&Change], theme: Theme) -> Line<'a> {
    let count = |wanted: fn(&Change) -> bool| shown.iter().filter(|c| wanted(c)).count();
    let parts = [
        (count(Change::breaking), "breaking", theme.danger),
        (count(|c| c.state == State::Added), "added", theme.success),
        (count(|c| c.state == State::Renamed), "renamed", theme.accent),
    ];
    counts(&parts, theme)
}

fn symbol_line<'a>(change: &Change, selected: bool, width: usize, theme: Theme) -> Line<'a> {
    let bar = Span::styled(if selected { "▎" } else { " " }, Style::default().fg(theme.accent));
    let (glyph, colour) = glyph(change.state, theme);
    let name = match (&change.state, &change.before) {
        (State::Renamed, Some(before)) => format!("{} → {}", before.name, change.symbol.name),
        _ => change.symbol.name.clone(),
    };
    let style = match change.state {
        _ if !change.symbol.public => Style::default().fg(theme.muted),
        State::Removed | State::Signature => Style::default().add_modifier(Modifier::BOLD),
        _ => Style::default(),
    };
    Line::from(vec![
        bar,
        Span::styled(format!(" {glyph} "), Style::default().fg(colour)),
        Span::styled(truncate(&name, width.saturating_sub(4)), style),
    ])
}

/// The old signature turned into the new one, the dropped words struck through.
fn signature_line<'a>(old: &str, new: &str, width: usize, theme: Theme) -> Line<'a> {
    let mut room = width.saturating_sub(5);
    let mut spans = vec![Span::raw("     ")];
    for segment in text_segments(old, new) {
        let (text, style) = match segment {
            Segment::Same(text) => (text, Style::default().fg(theme.muted)),
            Segment::Old(text) => (text, Style::default().fg(theme.danger).add_modifier(Modifier::CROSSED_OUT)),
            Segment::New(text) => (text, Style::default().fg(theme.success)),
        };
        let text = truncate(&text, room);
        room = room.saturating_sub(text.width());
        spans.push(Span::styled(text, style));
    }
    Line::from(spans)
}

fn glyph(state: State, theme: Theme) -> (&'static str, Color) {
    match state {
        State::Removed => ("-", theme.danger),
        State::Signature => ("~", theme.warn),
        State::Added => ("+", theme.success),
        State::Renamed => ("→", theme.accent),
        State::Body => ("·", theme.faded),
    }
}
