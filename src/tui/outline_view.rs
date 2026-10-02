//! The outline pane: what breaks, what came and what was renamed, then the call tree of the changed symbols, or their list by file.
use super::app::{App, Focus, Outline, OutlineEntry, Symbols};
use super::theme::Theme;
use super::ui::{counts, draw_empty, settle_scroll, side_pane, spinner, truncate};
use crate::diff::words::{Segment, text_segments};
use crate::outline::{Change, Direction, Item, State};
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
    let entries = outline.entries();
    if entries.is_empty() {
        let hint = if outline.all { "" } else { "a lists private ones too" };
        return draw_empty(f, theme, inner, &["no function or class changed", hint]);
    }
    let lines = lines(&outline.changes(), &entries, outline.selected, inner.width as usize, theme);
    let height = inner.height as usize;
    let cursor = lines.iter().position(|(entry, _)| *entry == Some(outline.selected)).unwrap_or(0);
    let scroll = settle_scroll(0, cursor, height);
    let lines: Vec<Line> = lines.into_iter().skip(scroll).take(height).map(|(_, line)| line).collect();
    f.render_widget(Paragraph::new(lines), inner);
}

/// `Outline · calls · public`: how the pane shows.
fn title(outline: &Outline) -> String {
    let shape = match (outline.flat, outline.direction) {
        (true, _) => "list",
        (false, Direction::Calls) => "calls",
        (false, Direction::CalledBy) => "called by",
    };
    format!("Outline · {shape} · {}", if outline.all { "all" } else { "public" })
}

/// Every row of the pane, each with the index of the entry it shows, if any; the flat list heads each file with its path.
fn lines<'a>(changes: &[&Change], entries: &[OutlineEntry], selected: usize, width: usize, theme: Theme) -> Vec<(Option<usize>, Line<'a>)> {
    let mut lines = vec![(None, badge(changes, theme)), (None, Line::default())];
    let mut path = None;
    for (index, entry) in entries.iter().enumerate() {
        if let Some(change) = entry.change.filter(|c| entry.branch.is_none() && path != Some(&c.path)) {
            if path.is_some() {
                lines.push((None, Line::default()));
            }
            path = Some(&change.path);
            let header = format!(" {}", truncate(&change.path, width.saturating_sub(1)));
            lines.push((None, Line::from(Span::styled(header, Style::default().fg(theme.faded)))));
        }
        lines.push((Some(index), entry_line(entry, index == selected, width, theme)));
        let signature = entry.change.filter(|c| entry.full() && c.state == State::Signature);
        if let Some((change, before)) = signature.and_then(|c| Some((c, c.before.as_ref()?))) {
            let indent = 4 + entry.lines.width();
            lines.push((None, signature_line(&before.signature, &change.symbol.signature, indent, width, theme)));
        }
    }
    lines
}

/// `2 breaking · 3 added · 1 renamed`, zeros left out.
fn badge<'a>(changes: &[&Change], theme: Theme) -> Line<'a> {
    let count = |wanted: fn(&Change) -> bool| changes.iter().filter(|c| wanted(c)).count();
    let parts = [
        (count(Change::breaking), "breaking", theme.danger),
        (count(|c| c.state == State::Added), "added", theme.success),
        (count(|c| c.state == State::Renamed), "renamed", theme.accent),
    ];
    counts(&parts, theme)
}

/// A changed symbol with its sign; a symbol shown in full elsewhere, a bridge, a fold or a cycle, dimmed.
fn entry_line<'a>(entry: &OutlineEntry, selected: bool, width: usize, theme: Theme) -> Line<'a> {
    let dim = Style::default().fg(theme.faded);
    let bar = Span::styled(if selected { "▎ " } else { "  " }, Style::default().fg(theme.accent));
    let mut spans = vec![bar, Span::styled(entry.lines.clone(), dim)];
    let room = width.saturating_sub(4 + entry.lines.width());
    let item = entry.branch.map(|b| &b.item);
    match (entry.change, item) {
        (Some(change), _) if entry.full() => spans.extend(symbol_spans(change, room, theme)),
        (Some(change), _) => spans.push(Span::styled(format!("{} {}", glyph(change.state, theme).0, truncate(&name(change), room)), dim)),
        (None, Some(Item::Bridge(name))) => spans.push(Span::styled(format!("· {}", truncate(&format!("{name}()"), room)), dim)),
        (None, Some(Item::Fold(hops))) => spans.push(Span::styled(format!("… {hops} calls"), dim)),
        (None, Some(Item::Cycle(name))) => spans.push(Span::styled(format!("↺ {}", truncate(name, room)), dim)),
        (None, _) => spans.push(Span::styled("unreached", dim)),
    }
    if entry.branch.is_some_and(|b| b.unsure) {
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
fn signature_line<'a>(old: &str, new: &str, indent: usize, width: usize, theme: Theme) -> Line<'a> {
    let mut room = width.saturating_sub(indent);
    let mut spans = vec![Span::raw(" ".repeat(indent))];
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
