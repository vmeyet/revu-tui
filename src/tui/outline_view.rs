//! The outline pane: what breaks, what came and what was renamed, then the changed symbols by file.
use super::app::{App, Focus, List, ListRow, PaneLine, Symbols};
use super::theme::Theme;
use super::ui::{counts, draw_empty, settle_scroll, side_pane};
use crate::diff::words::Segment;
use crate::outline::{Change, State};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;
use vtui::text::truncate;

pub(crate) fn draw(f: &mut Frame<'_>, app: &mut App, area: Rect) {
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
    let rows = &outline.rows;
    if reading.changes.is_empty() {
        return draw_empty(f, theme, inner, &["no function or class changed"]);
    }
    let height = inner.height as usize;
    let scroll = settle_scroll(0, rows.line_of.get(outline.selected).copied().unwrap_or(0), height);
    let width = inner.width as usize;
    let window = &rows.lines[scroll.min(rows.lines.len())..];
    let list_rows: Vec<ListRow> = window
        .iter()
        .take(height)
        .zip(inner.y..)
        .filter_map(|(line, y)| match line {
            PaneLine::Change(index) => Some(ListRow { area: Rect { y, height: 1, ..inner }, list: List::Outline, index: *index }),
            _ => None,
        })
        .collect();
    let shown: Vec<Line<'_>> = window
        .iter()
        .take(height)
        .map(|line| match line {
            PaneLine::Counts(counted) => counts_line(*counted, theme),
            PaneLine::Blank => Line::default(),
            PaneLine::Path(path) => {
                Line::from(Span::styled(format!(" {}", truncate(path, width.saturating_sub(1))), Style::default().fg(theme.faded)))
            }
            PaneLine::Change(index) => change_line(&reading.changes[*index], *index == outline.selected, width, theme),
            PaneLine::Signature { indent, segments } => signature_line(segments, *indent, width, theme),
        })
        .collect();
    f.render_widget(Paragraph::new(shown), inner);
    app.list_rows.extend(list_rows);
}

/// `2 breaking · 3 added · 1 renamed`, zeros left out.
fn counts_line<'a>([breaking, added, renamed]: [usize; 3], theme: Theme) -> Line<'a> {
    counts(&[(breaking, "breaking", theme.danger), (added, "added", theme.success), (renamed, "renamed", theme.accent)], theme)
}

/// A changed symbol with its sign.
fn change_line<'a>(change: &Change, selected: bool, width: usize, theme: Theme) -> Line<'a> {
    let bar = Span::styled(if selected { "▎ " } else { "  " }, Style::default().fg(theme.accent));
    let spans = [vec![bar], symbol_spans(change, width.saturating_sub(4), theme)].concat();
    Line::from(spans)
}

fn symbol_spans<'a>(change: &Change, room: usize, theme: Theme) -> Vec<Span<'a>> {
    let (glyph, colour) = glyph(change.state, theme);
    let style = match change.state {
        _ if !change.public() => Style::default().fg(theme.muted),
        State::Removed | State::Signature => Style::default().add_modifier(Modifier::BOLD),
        _ => Style::default(),
    };
    vec![Span::styled(format!("{glyph} "), Style::default().fg(colour)), Span::styled(truncate(&name(change), room), style)]
}

/// Its name, the old one first when renamed.
fn name(change: &Change) -> String {
    match (&change.state, &change.before) {
        (State::Renamed, Some(before)) => format!("{} → {}", before.name, change.symbol.name),
        _ => change.symbol.name.clone(),
    }
}

/// The old signature turned into the new one, the dropped words struck through.
fn signature_line<'a>(segments: &[Segment], indent: usize, width: usize, theme: Theme) -> Line<'a> {
    let mut room = width.saturating_sub(indent);
    let mut spans = vec![Span::raw(" ".repeat(indent))];
    for segment in segments {
        let (text, style) = match segment {
            Segment::Same(text) => (text, Style::default().fg(theme.muted)),
            Segment::Old(text) => (text, Style::default().fg(theme.danger).add_modifier(Modifier::CROSSED_OUT)),
            Segment::New(text) => (text, Style::default().fg(theme.success)),
        };
        let text = truncate(text, room);
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
