//! The outline pane: what breaks, what came and what was renamed, then the call tree of the changed symbols, or their list by file.
use super::app::{App, Focus, Outline, OutlineEntry, PaneLine, Symbols};
use super::queue_view::rule;
use super::theme::Theme;
use super::ui::{counts, draw_empty, settle_scroll, side_pane, spinner, truncate};
use crate::diff::words::Segment;
use crate::outline::{Change, Direction, Item, Section, State};
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
    let block = side_pane(theme, &title(outline), app.focus == Focus::Side, app.zen);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let tick = spinner(app.now.duration_since(app.started));
    match &outline.symbols {
        Symbols::Waiting => return draw_empty(f, theme, inner, &[tick, "", "reading the symbols"]),
        Symbols::Failed(message) => return draw_empty(f, theme, inner, &["outline unreadable", message, "", "r to retry"]),
        Symbols::Ready(_) => {}
    }
    let Symbols::Ready(reading) = &outline.symbols else { return };
    let rows = &outline.rows;
    if rows.entries.is_empty() {
        let hint = if outline.all { "" } else { "a lists private ones too" };
        return draw_empty(f, theme, inner, &["no function or class changed", hint]);
    }
    let height = inner.height as usize;
    let scroll = settle_scroll(0, rows.line_of.get(outline.selected).copied().unwrap_or(0), height);
    let width = inner.width as usize;
    let shown: Vec<Line> = rows.lines[scroll.min(rows.lines.len())..]
        .iter()
        .take(height)
        .map(|line| match line {
            PaneLine::Counts(counted) => counts_line(*counted, theme),
            PaneLine::Blank => Line::default(),
            PaneLine::Path(path) => {
                Line::from(Span::styled(format!(" {}", truncate(path, width.saturating_sub(1))), Style::default().fg(theme.faded)))
            }
            PaneLine::Entry(index) => match rows.entries[*index].header {
                Some((section, count)) => {
                    let name = if section == Section::Code { "CODE" } else { "TESTS" };
                    rule(theme, name, count, !rows.entries[*index].folded, *index == outline.selected, width)
                }
                None => entry_line(&rows.entries[*index], &reading.changes, *index == outline.selected, width, theme),
            },
            PaneLine::Signature { indent, segments } => signature_line(segments, *indent, width, theme),
        })
        .collect();
    f.render_widget(Paragraph::new(shown), inner);
}

/// `Outline · calls · public`: how the pane shows.
fn title(outline: &Outline) -> String {
    let shape = match (outline.flat, outline.direction) {
        (true, _) => "list",
        (false, Direction::Calls) => "calls",
        (false, Direction::CalledBy) => "called by",
    };
    let stack = if outline.stack { " · stack" } else { "" };
    format!("Outline · {shape} · {}{stack}", if outline.all { "all" } else { "public" })
}

/// `2 breaking · 3 added · 1 renamed`, zeros left out.
fn counts_line<'a>([breaking, added, renamed]: [usize; 3], theme: Theme) -> Line<'a> {
    counts(&[(breaking, "breaking", theme.danger), (added, "added", theme.success), (renamed, "renamed", theme.accent)], theme)
}

/// A changed symbol with its sign; a symbol shown in full elsewhere, a bridge, a fold or a cycle, dimmed.
fn entry_line<'a>(entry: &OutlineEntry, changes: &[Change], selected: bool, width: usize, theme: Theme) -> Line<'a> {
    let dim = Style::default().fg(theme.faded);
    let bar = Span::styled(if selected { "▎ " } else { "  " }, Style::default().fg(theme.accent));
    let mut spans = vec![bar, Span::styled(entry.lines.clone(), dim)];
    let room = width.saturating_sub(4 + entry.lines.width());
    match (entry.change.map(|c| &changes[c]), &entry.item) {
        (Some(change), _) if entry.full() => spans.extend(symbol_spans(change, room, theme)),
        (Some(change), _) => spans.push(Span::styled(format!("{} {}", glyph(change.state, theme).0, truncate(&name(change), room)), dim)),
        (None, Some(Item::Bridge(name))) => spans.push(Span::styled(format!("· {}", truncate(&format!("{name}()"), room)), dim)),
        (None, Some(Item::Fold(hops))) => spans.push(Span::styled(format!("… {hops} calls"), dim)),
        (None, Some(Item::Cycle(name))) => spans.push(Span::styled(format!("↺ {}", truncate(name, room)), dim)),
        (None, _) => {}
    }
    if entry.unsure {
        spans.push(Span::styled(" ?", Style::default().fg(theme.warn)));
    }
    if entry.folded {
        spans.push(Span::styled(" …", dim));
    }
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
