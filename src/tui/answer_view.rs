//! The right pane while it holds Claude's answer: the text as it streams, then how it ended.
use super::app::{Answer, AnswerState, App, Focus, Input};
use super::theme::Theme;
use super::thread_view::{body_lines, box_height, draw_compose, wrap};
use super::ui::{side_pane, spinner, truncate};
use crate::ai::anthropic::{Outcome, Stop};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    let theme = app.theme;
    let focused = app.focus == Focus::Side;
    let elapsed = app.now.duration_since(app.started);
    let compose = matches!(app.input, Some(Input::Ask { .. } | Input::FollowUp)).then(|| (app.input_label(), app.buffer.clone()));
    let Some(answer) = app.open.as_ref().and_then(|o| o.answer.clone()) else { return };
    let title = if answer.cached { format!("Claude · {} · cached", answer.label) } else { format!("Claude · {}", answer.label) };
    let block = side_pane(theme, &title, focused, app.zen);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let inner = match &compose {
        Some((label, field)) => {
            let [list, box_area] = Layout::vertical([Constraint::Min(1), Constraint::Length(box_height(field, inner))]).areas(inner);
            draw_compose(f, theme, label, field, box_area);
            list
        }
        None => inner,
    };
    let [body, footer] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
    let width = body.width.saturating_sub(1) as usize;
    let lines: Vec<Line> = body_lines(&answer.text, theme).into_iter().flat_map(|line| wrap(line, width)).collect();
    let height = body.height as usize;
    let scroll = scroll_of(&answer, lines.len(), height);
    let shown: Vec<Line> = lines.into_iter().skip(scroll).take(height).map(|l| indent(l)).collect();
    f.render_widget(Paragraph::new(shown), body);
    f.render_widget(Paragraph::new(footer_line(&answer, theme, spinner(elapsed), footer.width as usize)), footer);
}

/// While it streams the pane follows the end; once done the reader's scroll holds.
fn scroll_of(answer: &Answer, lines: usize, height: usize) -> usize {
    let last = lines.saturating_sub(height);
    match answer.state {
        AnswerState::Streaming => last,
        _ => answer.scroll.min(last),
    }
}

fn indent(line: Line<'static>) -> Line<'static> {
    Line::from(std::iter::once(Span::raw(" ")).chain(line.spans).collect::<Vec<_>>())
}

/// How the answer stands: streaming, cut, declined, failed, or which model answered and what the cache saved.
fn footer_line(answer: &Answer, theme: Theme, spinner: &str, width: usize) -> Line<'static> {
    let faded = Style::default().fg(theme.faded);
    match &answer.state {
        AnswerState::Streaming => Line::from(vec![
            Span::styled(format!(" {spinner} "), Style::default().fg(theme.accent)),
            Span::styled("answering · esc closes", faded),
        ]),
        AnswerState::Failed(message) => {
            Line::from(Span::styled(format!(" ✗ {message} · R to ask again"), Style::default().fg(theme.danger)))
        }
        AnswerState::Done(Outcome { stop: Stop::Refused(why), .. }) => {
            let why = why.as_deref().map(|w| format!(": {w}")).unwrap_or_default();
            Line::from(Span::styled(format!(" declined{why}"), Style::default().fg(theme.danger)))
        }
        AnswerState::Done(Outcome { stop: Stop::Cut, .. }) => {
            Line::from(Span::styled(" cut at the token limit · enter to ask for the rest", Style::default().fg(theme.warn)))
        }
        AnswerState::Done(outcome) if outcome.model.is_empty() => Line::from(Span::styled(" type your question below", faded)),
        AnswerState::Done(outcome) => {
            let facts =
                format!(" {} · {} cached · {} out", outcome.model, thousands(outcome.usage.cache_read), thousands(outcome.usage.output));
            let hints = " · c draft · ⏎ follow up · R again";
            let text = if facts.width() + hints.width() <= width { facts + hints } else { truncate(&facts, width) };
            Line::from(Span::styled(text, faded))
        }
    }
}

/// `1.2k` past a thousand: a token count is read at a glance, not to the unit.
fn thousands(n: u64) -> String {
    if n >= 1000 { format!("{}.{}k", n / 1000, n % 1000 / 100) } else { n.to_string() }
}
