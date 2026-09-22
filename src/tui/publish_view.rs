//! The publish modal: every draft on one screen, the approval tick, one key to send them all.
use super::app::{App, Publish};
use super::ui::{pane, spinner, truncate};
use crate::review::{Draft, Side};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

const WIDTH: u16 = 64;
const CHROME_ROWS: u16 = 9;

pub fn draw(f: &mut Frame, app: &App, publish: &Publish, area: Rect) {
    let theme = app.theme;
    let Some(open) = &app.open else { return };
    let drafts = &open.review.drafts;
    let files = drafts.iter().filter_map(|d| d.anchor.as_ref().map(|a| a.path.as_str())).collect::<std::collections::BTreeSet<_>>().len();
    let width = WIDTH.min(area.width);
    let height = (drafts.len() as u16 + CHROME_ROWS).min(area.height);
    let popup = Rect { x: area.x + (area.width - width) / 2, y: area.y + (area.height - height) / 2, width, height };
    let block = pane(theme, "Publish review", true);
    let inner = block.inner(popup);
    let room = inner.width as usize;
    let muted = Style::default().fg(theme.muted);
    let mut lines = vec![
        Line::default(),
        Line::from(Span::styled(
            format!("{} draft{} on {files} file{}", drafts.len(), plural(drafts.len()), plural(files)),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::default(),
    ];
    lines.extend(drafts.iter().enumerate().map(|(i, draft)| draft_line(draft, i == publish.selected, room, theme)));
    lines.push(Line::default());
    let tick = if publish.approve { "[x]" } else { "[ ]" };
    lines.push(Line::from(vec![
        Span::styled(format!("{tick} approve"), Style::default().fg(if publish.approve { theme.success } else { theme.muted })),
        Span::styled("  a to toggle", Style::default().fg(theme.faded)),
    ]));
    lines.push(Line::default());
    let footer = if publish.busy {
        Line::from(vec![
            Span::styled(spinner(app.now.duration_since(app.started)), Style::default().fg(theme.accent)),
            Span::styled(" publishing", muted),
        ])
    } else {
        let on_footer = publish.selected >= drafts.len();
        Line::from(vec![
            Span::styled(if on_footer { "▎" } else { " " }, Style::default().fg(theme.accent)),
            Span::styled("enter publish", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
            Span::styled(" · d delete · esc back", Style::default().fg(theme.faded)),
        ])
    };
    lines.push(footer);
    f.render_widget(Clear, popup);
    f.render_widget(Paragraph::new(lines).block(block).style(Style::default().bg(theme.surface)), popup);
}

fn draft_line<'a>(draft: &Draft, selected: bool, room: usize, theme: super::theme::Theme) -> Line<'a> {
    let place = match (&draft.anchor, &draft.reply_to) {
        (Some(anchor), _) => {
            let name = anchor.path.rsplit('/').next().unwrap_or(&anchor.path);
            let sign = if anchor.side == Side::Old { "-" } else { "" };
            format!("{name}:{sign}{}", anchor.line)
        }
        (None, Some(_)) => "reply".to_owned(),
        (None, None) => "MR".to_owned(),
    };
    let place = format!("{place:<22}");
    let body = truncate(draft.body.lines().next().unwrap_or_default(), room.saturating_sub(place.width() + 2));
    Line::from(vec![
        Span::styled(if selected { "▎" } else { " " }, Style::default().fg(theme.accent)),
        Span::styled(place, Style::default().fg(theme.muted)),
        Span::styled(body, if selected { Style::default().add_modifier(Modifier::BOLD) } else { Style::default() }),
    ])
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}
