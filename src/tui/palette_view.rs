//! The palette popup: the prompt with its mode, then MRs or files to pick, or the commands that
//! match what is typed.
use super::app::App;
use super::palette::{Mode, Palette, Target};
use super::ui::{pane, truncate};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

pub fn draw(f: &mut Frame, app: &App, palette: &Palette, area: Rect) {
    let theme = app.theme;
    let width = (area.width * 3 / 5).clamp(40.min(area.width), area.width);
    let height = (super::palette::MAX_SHOWN as u16 + 4).min(area.height);
    let popup = Rect { x: area.x + (area.width - width) / 2, y: area.y + area.height.saturating_sub(height) / 3, width, height };
    let (sign, what) = palette.mode.prompt();
    let block = pane(theme, &format!("search · {what}"), true);
    let inner = block.inner(popup);
    let room = inner.width.saturating_sub(4) as usize;
    let mut lines = vec![prompt(app, palette, sign), Line::default()];
    lines.extend(match palette.mode {
        Mode::Commands => command_lines(app, palette, room),
        Mode::Mrs | Mode::Files => list_lines(app, palette, room),
    });
    lines.push(Line::from(Span::styled(footer(palette.mode), Style::default().fg(theme.faded))));
    f.render_widget(Clear, popup);
    f.render_widget(Paragraph::new(lines).block(block).style(Style::default().bg(theme.surface)), popup);
}

/// `› text▌`, or `> text▌` and `/ text▌` in their modes, the ghost of a completion or the options
/// tab is cycling through.
fn prompt(app: &App, palette: &Palette, sign: &str) -> Line<'static> {
    let theme = app.theme;
    let mut spans = vec![
        Span::styled(
            format!("{} ", if sign.is_empty() { "›" } else { sign }),
            Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
        ),
        Span::raw(palette.input.clone()),
        Span::styled("▌", Style::default().fg(theme.accent)),
    ];
    if palette.mode == Mode::Commands {
        let candidates = app.completions_for(&palette.input);
        match (palette.hint(), palette.ghost(&candidates)) {
            (Some(hint), _) => spans.push(Span::styled(format!("   {hint}"), Style::default().fg(theme.muted))),
            (None, Some(ghost)) => spans.push(Span::styled(ghost, Style::default().fg(theme.faded))),
            (None, None) => {}
        }
    }
    Line::from(spans)
}

fn list_lines(app: &App, palette: &Palette, room: usize) -> Vec<Line<'static>> {
    let theme = app.theme;
    let candidates = app.palette_candidates(palette);
    if candidates.is_empty() {
        let why = match palette.mode {
            Mode::Files if app.open.is_none() => "open an MR first: / searches its files",
            _ => "no match",
        };
        return vec![Line::from(Span::styled(format!("  {why}"), Style::default().fg(theme.muted)))];
    }
    candidates
        .iter()
        .enumerate()
        .map(|(i, candidate)| {
            let selected = i == palette.selected;
            let (icon, colour) = match candidate.target {
                Target::Mr(_) => ("◆", theme.accent),
                Target::File(_) => ("·", theme.muted),
            };
            let detail = if candidate.detail.is_empty() { String::new() } else { format!("  {}", candidate.detail) };
            let label = truncate(&candidate.label, room.saturating_sub(detail.width()));
            let text = if selected { Style::default().add_modifier(Modifier::BOLD) } else { Style::default() };
            Line::from(vec![
                Span::styled(if selected { "▎" } else { " " }, Style::default().fg(theme.accent)),
                Span::styled(format!("{icon} "), Style::default().fg(colour)),
                Span::styled(label, text),
                Span::styled(detail, Style::default().fg(theme.faded)),
            ])
        })
        .collect()
}

/// The verbs that match the first word typed, each with what it does.
fn command_lines(app: &App, palette: &Palette, room: usize) -> Vec<Line<'static>> {
    let theme = app.theme;
    let naming = palette.naming_command();
    super::palette::verbs_for(&palette.input)
        .into_iter()
        .enumerate()
        .map(|(i, (verb, what))| {
            let here = naming && i == palette.selected;
            let line = Line::from(vec![
                Span::styled(format!("{}{verb:<9}", if here { "▸ " } else { "  " }), Style::default().fg(theme.accent)),
                Span::styled(truncate(what, room.saturating_sub(11)), Style::default().fg(theme.muted)),
            ]);
            if here { line.style(Style::default().bg(theme.highlight.unwrap_or(theme.base))) } else { line }
        })
        .collect()
}

fn footer(mode: Mode) -> &'static str {
    match mode {
        Mode::Mrs => "  ↑↓ pick · enter opens · > commands · / files · esc",
        Mode::Files => "  ↑↓ pick · enter jumps · ⌫ back to MRs · esc",
        Mode::Commands => "  ↑↓ pick · tab completes · ^p history · esc",
    }
}
