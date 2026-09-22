use super::app::{App, Focus};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph};
use std::time::Duration;

const QUEUE_W: u16 = 34;
const SIDE_W: u16 = 32;
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const SPINNER_FRAME: Duration = Duration::from_millis(80);

pub fn draw(f: &mut Frame, app: &App) {
    let [main, status] = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(f.area());
    let [queue, review, side] =
        Layout::horizontal([Constraint::Length(QUEUE_W), Constraint::Min(40), Constraint::Length(SIDE_W)]).areas(main);
    draw_pane(f, app, queue, "Queue", Focus::Queue, "nothing loaded yet\nthe queue arrives in M1");
    draw_pane(f, app, review, "Review", Focus::Review, "open a merge request\nto read its diff here");
    draw_pane(f, app, side, "Thread", Focus::Side, "");
    draw_status(f, app, status);
    if app.help {
        draw_help(f, app, main);
    }
}

fn draw_pane(f: &mut Frame, app: &App, area: Rect, title: &str, focus: Focus, empty: &str) {
    let focused = app.focus == focus && !app.help;
    let theme = app.theme;
    let title_style =
        if focused { Style::default().fg(theme.accent).add_modifier(Modifier::BOLD) } else { Style::default().fg(theme.faded) };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if focused { theme.accent } else { theme.border }))
        .title(Span::styled(format!(" {title} "), title_style))
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let text: Vec<Line> = empty.lines().map(|l| Line::from(Span::styled(l, Style::default().fg(theme.faded)))).collect();
    let top = inner.y + inner.height.saturating_sub(text.len() as u16) / 2;
    let centered = Rect { y: top, height: (text.len() as u16).min(inner.height), ..inner };
    f.render_widget(Paragraph::new(text).alignment(Alignment::Center), centered);
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme;
    let muted = Style::default().fg(theme.muted);
    let left = match &app.error {
        Some(message) => Line::from(Span::styled(format!(" ✗ {message}"), Style::default().fg(theme.danger))),
        None => Line::from(vec![
            Span::styled(format!(" {}", app.host), muted),
            Span::styled(" · ", Style::default().fg(theme.faded)),
            Span::styled(if app.me.is_empty() { "…".to_owned() } else { app.me.clone() }, muted),
        ]),
    };
    let right = if app.loading {
        Line::from(vec![
            Span::styled(spinner(app.now.duration_since(app.started)), Style::default().fg(theme.accent)),
            Span::styled(" loading · ? help ", muted),
        ])
    } else {
        Line::from(Span::styled("? help ", muted))
    };
    let [l, r] = Layout::horizontal([Constraint::Min(10), Constraint::Length(right.width() as u16)]).areas(area);
    f.render_widget(Paragraph::new(left), l);
    f.render_widget(Paragraph::new(right).alignment(Alignment::Right), r);
}

fn draw_help(f: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme;
    let rows = [("h l", "focus the pane to the left, right"), ("r", "refresh"), ("?", "this help"), ("q", "quit")];
    let lines: Vec<Line> = rows
        .iter()
        .map(|(key, what)| {
            Line::from(vec![
                Span::styled(format!("{key:>6}  "), Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
                Span::raw(*what),
            ])
        })
        .collect();
    let height = lines.len() as u16 + 2;
    let width = 44.min(area.width);
    let popup = Rect { x: area.x + (area.width - width) / 2, y: area.y + (area.height.saturating_sub(height)) / 2, width, height };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .title(Span::styled(" keys ", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)))
        .padding(Padding::horizontal(1));
    f.render_widget(Clear, popup);
    f.render_widget(Paragraph::new(lines).block(block).style(Style::default().bg(theme.surface)), popup);
}

fn spinner(elapsed: Duration) -> &'static str {
    SPINNER[(elapsed.as_millis() / SPINNER_FRAME.as_millis()) as usize % SPINNER.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme::Theme;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn render(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| (0..width).map(|x| buffer[(x, y)].symbol().to_owned()).collect::<String>().trim_end().to_owned())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn empty_shell_shows_three_panes_and_the_host() {
        let mut app = App::new(Theme::default(), "gitlab.com".into(), "nina".into());
        app.loading = false;
        let screen = render(&app, 120, 20);
        assert!(screen.contains("Queue") && screen.contains("Review") && screen.contains("Thread"), "{screen}");
        assert!(screen.contains("gitlab.com · nina"), "{screen}");
        assert!(screen.contains("? help"), "{screen}");
    }

    #[test]
    fn help_overlay_lists_every_key() {
        let mut app = App::new(Theme::default(), "gitlab.com".into(), String::new());
        app.help = true;
        let screen = render(&app, 120, 20);
        for key in ["h l", "r", "?", "q"] {
            assert!(screen.contains(key), "missing {key}:\n{screen}");
        }
    }

    #[test]
    fn errors_take_the_status_line() {
        let mut app = App::new(Theme::default(), "gitlab.com".into(), String::new());
        app.error = Some("offline".into());
        assert!(render(&app, 100, 10).contains("✗ offline"));
    }
}
