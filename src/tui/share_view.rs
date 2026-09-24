//! The share modal: the target picker, the note box, then the exact message `y` will send.
use super::app::{App, ShareStage, Sharing};
use super::theme::Theme;
use super::thread_view::draw_compose;
use super::ui::pane;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};

const WIDTH: u16 = 72;
const NOTE_ROWS: u16 = 5;

pub fn draw(f: &mut Frame, app: &App, sharing: &Sharing, area: Rect) {
    let theme = app.theme;
    let width = WIDTH.min(area.width.saturating_sub(4)).max(20);
    let block = pane(theme, &sharing.title(), true)
        .title_bottom(Line::from(Span::styled(format!(" {} ", keys(sharing)), Style::default().fg(theme.faded))));
    let text_width = block.inner(Rect { width, height: 3, ..area }).width.max(1);
    let body = lines(sharing, theme, text_width);
    let note_rows = if matches!(sharing.stage, ShareStage::Note { .. }) { NOTE_ROWS } else { 0 };
    let body_rows: u16 = body.iter().map(|line| (line.width() as u16).div_ceil(text_width).max(1)).sum();
    let height = (body_rows + note_rows + 4).min(area.height);
    let popup = Rect { x: area.x + (area.width - width) / 2, y: area.y + area.height.saturating_sub(height) / 2, width, height };
    let inner = block.inner(popup).inner(ratatui::layout::Margin { horizontal: 0, vertical: 1 });
    f.render_widget(Clear, popup);
    f.render_widget(block.style(Style::default().bg(theme.surface)), popup);
    let text = Rect { height: inner.height.saturating_sub(note_rows), ..inner };
    f.render_widget(Paragraph::new(body).wrap(Wrap { trim: false }), text);
    if let ShareStage::Note { note, .. } = &sharing.stage {
        let area = Rect { y: text.y + text.height, height: note_rows.min(inner.height), ..inner };
        draw_compose(f, theme, "note, optional", note, area);
    }
}

fn keys(sharing: &Sharing) -> &'static str {
    match sharing.stage {
        ShareStage::Pick { .. } => "j k move · enter pick · esc cancel",
        ShareStage::Note { .. } => "enter preview · esc cancel",
        ShareStage::Preview { .. } => "y send · e edit the note · esc cancel",
    }
}

fn lines(sharing: &Sharing, theme: Theme, width: u16) -> Vec<Line<'static>> {
    let faded = Style::default().fg(theme.faded);
    match &sharing.stage {
        ShareStage::Pick { targets, selected } => {
            let mut lines = vec![Line::from(Span::styled(sharing.fields.title.clone(), faded)), Line::default()];
            lines.extend(targets.iter().enumerate().map(|(i, target)| {
                let name = target.name.clone().unwrap_or_else(|| "share".to_owned());
                let chosen = i == *selected;
                let bar = Span::styled(if chosen { "▎" } else { " " }, Style::default().fg(theme.accent));
                let style = if chosen { Style::default().add_modifier(Modifier::BOLD) } else { Style::default() };
                Line::from(vec![
                    bar,
                    Span::styled(format!("{} {name}", i + 1), style),
                    Span::styled(format!("  {}", target.command), faded),
                ])
            }));
            lines
        }
        ShareStage::Note { target, .. } => {
            vec![Line::from(Span::styled(format!("to `{}`", target.command), faded)), Line::default()]
        }
        ShareStage::Preview { target, message, .. } => {
            let rule = Span::styled("─".repeat(width as usize), faded);
            let mut lines = vec![Line::from(Span::styled(format!("to `{}`", target.command), faded)), Line::from(rule.clone())];
            lines.extend(message.lines().map(|line| Line::from(line.to_owned())));
            lines.push(Line::from(rule));
            lines
        }
    }
}
