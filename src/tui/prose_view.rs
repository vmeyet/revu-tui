//! The prose view in the diff area: the file's title, then the rows in view of its rendered blocks.
use super::app::{App, Texts, Versions};
use super::theme::Theme;
use super::ui::{draw_empty, spinner};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

/// The title row and the empty row under it.
const TITLE_ROWS: u16 = 2;

pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    let (theme, elapsed) = (app.theme, app.now.duration_since(app.started));
    let Some(prose) = app.open.as_mut().and_then(|open| open.prose.as_mut()) else { return };
    let title = Line::styled(format!("{} · prose", prose.path()), Style::default().fg(theme.muted));
    f.render_widget(Paragraph::new(title), area);
    let body = Rect { y: area.y + TITLE_ROWS, height: area.height.saturating_sub(TITLE_ROWS), ..area };
    let versions = match &prose.texts {
        Texts::Waiting => return draw_empty(f, theme, body, &[spinner(elapsed), "", "reading the file at both commits"]),
        Texts::Failed(message) => return draw_empty(f, theme, body, &[message, "", "r to retry"]),
        Texts::Ready(versions) => versions,
    };
    let rows = app.kept.prose_rows(versions, body.width as usize, theme, prose.unfolded);
    prose.scroll = prose.scroll.min(rows.len().saturating_sub(body.height as usize));
    let shown: Vec<Line> = rows.iter().skip(prose.scroll).take(body.height as usize).cloned().collect();
    f.render_widget(Paragraph::new(shown), body);
}

#[cfg(feature = "prose")]
pub fn rows(versions: &Versions, width: usize, theme: Theme, unfolded: bool) -> Vec<Line<'static>> {
    super::prose::render(&versions.base, &versions.head, width, theme, unfolded)
}

#[cfg(not(feature = "prose"))]
pub fn rows(_: &Versions, _: usize, theme: Theme, _: bool) -> Vec<Line<'static>> {
    vec![Line::styled("this revu was built without the prose view (cargo feature `prose`)", Style::default().fg(theme.muted))]
}
