//! The description modal, centered over the panes, the body in the same light markdown as notes.
use super::app::{App, Brief};
use super::theme::Theme;
use super::thread_view::body_lines;
use super::ui::pane;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};

const SIZE_PCT: u16 = 80;

pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    let theme = app.theme;
    let Some(brief) = app.brief.as_mut() else { return };
    let width = (area.width * SIZE_PCT / 100).max(40).min(area.width);
    let height = (area.height * SIZE_PCT / 100).max(10).min(area.height);
    let popup = Rect { x: area.x + (area.width - width) / 2, y: area.y + (area.height - height) / 2, width, height };
    let block = pane(theme, &format!("!{} {}", brief.iid, brief.title), true)
        .title_bottom(Line::from(Span::styled(" j k scroll · o open in the browser · esc close ", Style::default().fg(theme.faded))));
    let inner = block.inner(popup);
    let lines = lines(brief, theme);
    let rows = wrapped_rows(&lines, inner.width);
    brief.scroll = brief.scroll.min(rows.saturating_sub(inner.height as usize));
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false }).scroll((brief.scroll as u16, 0));
    f.render_widget(Clear, popup);
    f.render_widget(paragraph.block(block).style(Style::default().bg(theme.surface)), popup);
}

fn lines<'a>(brief: &Brief, theme: Theme) -> Vec<Line<'a>> {
    let muted = Style::default().fg(theme.muted);
    let mut head = vec![
        Span::styled(brief.author.clone(), Style::default().fg(theme.user(&brief.author)).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" · {} → {}", brief.source_branch, brief.target_branch), muted),
    ];
    if !brief.labels.is_empty() {
        head.push(Span::styled(format!(" · {}", brief.labels.join(", ")), Style::default().fg(theme.faded)));
    }
    let mut lines = vec![Line::from(head), Line::default()];
    match brief.description.trim() {
        "" => lines.push(Line::from(Span::styled("no description", Style::default().fg(theme.faded)))),
        text => lines.extend(body_lines(text, theme)),
    }
    lines
}

/// Rows the wrapped text takes, close enough to stop the scroll at the last page.
fn wrapped_rows(lines: &[Line], width: u16) -> usize {
    let width = usize::from(width.max(1));
    lines.iter().map(|l| l.width().max(1).div_ceil(width)).sum()
}
