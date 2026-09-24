use super::app::{App, Focus};
use super::theme::Theme;
use super::{brief_view, diff_view, publish_view, thread_view};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Padding, Paragraph};
use std::time::Duration;
use unicode_width::UnicodeWidthStr;

const QUEUE_W: u16 = 34;
/// The queue on a wide terminal: ten more columns show about twice the title.
const WIDE_QUEUE_W: u16 = 44;
/// From this terminal width the queue takes `WIDE_QUEUE_W`.
const WIDE_QUEUE_FROM: u16 = 160;
/// How wide the diff reads in reading mode: a comfortable line of code with both gutters.
const READING_W: u16 = 120;
const SIDE_W: u16 = 36;
/// From this width the queue, the diff and the right pane sit side by side.
const WIDE: u16 = 150;
/// From this width the diff and the right pane share the screen; below, the pane is a page of its own.
const MEDIUM: u16 = 120;
const SIDE_PCT: u16 = 40;
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const SPINNER_FRAME: Duration = Duration::from_millis(80);
/// A `!42` on screen: the loop prints it again as a terminal hyperlink to `url`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub x: u16,
    pub y: u16,
    pub text: String,
    pub url: String,
}

pub fn draw(f: &mut Frame, app: &mut App) {
    app.links.clear();
    let input_rows = u16::from(app.filtering);
    let [main, input, status] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(input_rows), Constraint::Length(1)]).areas(f.area());
    let side_open = app.open.as_ref().is_some_and(|o| o.pane.is_some() || o.tree.is_some() || o.answer.is_some() || o.pipeline.is_some());
    let shown = columns(main.width, side_open, app.focus == Focus::Side, app.reading);
    let diff = if shown.diff { Constraint::Min(1) } else { Constraint::Length(0) };
    let side_width = if shown.diff { Constraint::Length(shown.side) } else { Constraint::Min(0) };
    let [queue, review, side] = Layout::horizontal([Constraint::Length(shown.queue), diff, side_width]).areas(main);
    let review = if app.reading && shown.side == 0 { centered(review, READING_W) } else { review };
    let side_open = side_open && shown.side > 0;
    if shown.queue > 0 {
        super::queue_view::draw(f, app, queue);
    }
    if shown.diff {
        diff_view::draw(f, app, review);
    }
    let mut pictures = vec![];
    if app.open.as_ref().is_some_and(|o| o.answer.is_some()) && side_open {
        super::answer_view::draw(f, app, side);
    } else if app.open.as_ref().is_some_and(|o| o.pipeline.is_some()) && side_open {
        super::pipeline_view::draw(f, app, side);
    } else if app.open.as_ref().is_some_and(|o| o.tree.is_some()) && side_open {
        super::tree_view::draw(f, app, side);
    } else if side_open {
        pictures = thread_view::draw(f, app, side);
    }
    if app.filtering {
        draw_filter(f, app, input);
    }
    draw_status(f, app, status);
    let modal = app.help.is_some() || app.publish.is_some() || app.brief.is_some() || app.palette.is_some() || app.sharing.is_some();
    if modal {
        app.links.clear();
    }
    if app.focus != Focus::Queue || modal {
        fade(f, queue, app.theme.faded);
    }
    if app.focus != Focus::Review || modal {
        fade(f, review, if modal { app.theme.faded } else { app.theme.muted });
    }
    if side_open && (app.focus != Focus::Side || modal) {
        fade(f, side, app.theme.faded);
    }
    if !modal {
        draw_pictures(f, app, &pictures);
    }
    if let Some(publish) = app.publish.clone() {
        publish_view::draw(f, app, &publish, main);
    }
    if app.brief.is_some() {
        brief_view::draw(f, app, main);
    }
    if let Some(sharing) = &app.sharing {
        super::share_view::draw(f, app, sharing, main);
    }
    if let Some(palette) = &app.palette {
        super::palette_view::draw(f, app, palette, main);
    }
    if let Some(scroll) = app.help {
        super::help::draw(f, app, main, scroll);
    }
}

/// Pictures go on last, after the fades: the terminal paints them as pixels or placeholder cells,
/// and both must stay as the protocol wrote them. Under a modal they are left out, since pixels
/// would show through it.
fn draw_pictures(f: &mut Frame, app: &mut App, pictures: &[thread_view::Placement]) {
    for picture in pictures {
        if let Some(super::images::Thumb::Ready(protocol, _)) = app.thumbs.get_mut(&picture.url) {
            let widget = ratatui_image::StatefulImage::<ratatui_image::protocol::StatefulProtocol>::default()
                .resize(ratatui_image::Resize::Fit(Some(image::imageops::FilterType::Triangle)));
            f.render_stateful_widget(widget, picture.area, protocol.as_mut());
        }
    }
}

/// Which columns show, and how wide: zero hides the queue or the right pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Columns {
    queue: u16,
    side: u16,
    diff: bool,
}

/// The right pane's width rules: all three columns from 150, the queue steps aside from 120,
/// and below that, or in reading mode, the pane is a page of its own while it has the keys.
fn columns(width: u16, side_open: bool, side_focused: bool, reading: bool) -> Columns {
    let side = SIDE_W.max(width * SIDE_PCT / 100);
    let queue = match (reading, width) {
        (true, _) => 0,
        (false, WIDE_QUEUE_FROM..) => WIDE_QUEUE_W,
        (false, _) => QUEUE_W,
    };
    match (side_open, width) {
        (false, _) => Columns { queue, side: 0, diff: true },
        (true, WIDE..) if !reading => Columns { queue, side, diff: true },
        (true, MEDIUM..) if !reading => Columns { queue: 0, side, diff: true },
        (true, _) if side_focused => Columns { queue: 0, side: width, diff: false },
        (true, _) => Columns { queue: 0, side: 0, diff: true },
    }
}

/// `area` narrowed to `width` columns in its middle, for reading mode.
fn centered(area: Rect, width: u16) -> Rect {
    let width = width.min(area.width);
    Rect { x: area.x + (area.width - width) / 2, width, ..area }
}

pub fn pane(theme: Theme, title: &str, focused: bool) -> Block<'static> {
    let title_style =
        if focused { Style::default().fg(theme.accent).add_modifier(Modifier::BOLD) } else { Style::default().fg(theme.faded) };
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if focused { theme.accent } else { theme.border }))
        .title(Span::styled(format!(" {title} "), title_style))
        .padding(Padding::horizontal(1))
}

pub fn draw_empty(f: &mut Frame, theme: Theme, area: Rect, lines: &[&str]) {
    let text: Vec<Line> = lines.iter().map(|l| Line::from(Span::styled((*l).to_owned(), Style::default().fg(theme.faded)))).collect();
    let top = area.y + area.height.saturating_sub(text.len() as u16) / 2;
    let centered = Rect { y: top, height: (text.len() as u16).min(area.height), ..area };
    f.render_widget(Paragraph::new(text).alignment(Alignment::Center), centered);
}

fn draw_filter(f: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme;
    let line = Line::from(vec![
        Span::styled(" / ", Style::default().fg(theme.accent)),
        Span::raw(app.filter.clone()),
        Span::styled("▏", Style::default().fg(theme.accent)),
        Span::styled("  enter keep · esc clear", Style::default().fg(theme.faded)),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme;
    let muted = Style::default().fg(theme.muted);
    let dot = Span::styled(" · ", Style::default().fg(theme.faded));
    if let Some(confirm) = &app.confirm {
        let line = Line::from(Span::styled(format!(" ? {}", confirm.question()), Style::default().fg(theme.warn)));
        f.render_widget(Paragraph::new(line), area);
        return;
    }
    if app.pending == Some('\'') {
        let line = Line::from(Span::styled(format!(" ' {}", app.views_hint()), Style::default().fg(theme.accent)));
        f.render_widget(Paragraph::new(line), area);
        return;
    }
    let left = match (app.live_toast(), app.offline, &app.open) {
        (None, None, Some(_)) if app.news.is_some() => {
            Line::from(Span::styled(format!(" {}", app.news.clone().unwrap_or_default()), Style::default().fg(theme.accent)))
        }
        (Some(toast), _, _) => {
            let colour = if toast.danger { theme.danger } else { theme.accent };
            let glyph = if toast.danger { "✗" } else { "✓" };
            Line::from(Span::styled(format!(" {glyph} {}", toast.text), Style::default().fg(colour)))
        }
        (None, Some(_), Some(open)) => {
            let age = open.staleness(app.now).map_or_else(|| "now".into(), short_age);
            Line::from(Span::styled(format!(" offline · last refresh {age} ago"), Style::default().fg(theme.warn)))
        }
        _ => {
            let mut spans = vec![Span::styled(format!(" {}", app.host), muted)];
            if !app.me.is_empty() {
                spans.push(dot.clone());
                spans.push(Span::styled(app.me.clone(), muted));
            }
            if let Some(reason) = app.selected_reason().filter(|_| app.focus == Focus::Queue) {
                spans.push(dot.clone());
                spans.push(Span::styled(reason, Style::default().fg(theme.faded)));
            }
            if let Some(open) = &app.open {
                let unresolved = open.review.unresolved();
                if unresolved > 0 {
                    spans.push(dot.clone());
                    spans.push(Span::styled(format!("{unresolved} unresolved"), Style::default().fg(theme.warn)));
                }
                let drafts = open.review.drafts.len();
                if drafts > 0 {
                    spans.push(dot.clone());
                    spans.push(Span::styled(
                        format!("{drafts} draft{}", if drafts == 1 { "" } else { "s" }),
                        Style::default().fg(theme.warn),
                    ));
                    let unsaved = app.unsaved_drafts();
                    if unsaved > 0 {
                        spans.push(Span::styled(format!(" ({unsaved} unsaved)"), Style::default().fg(theme.danger)));
                    }
                    spans.push(Span::styled(" · P to publish", muted));
                }
            }
            Line::from(spans)
        }
    };
    let rate = match (app.rate.wait, app.rate.remaining) {
        (Some(wait), _) => Some(Span::styled(format!("⏳ {}s", wait.as_secs()), Style::default().fg(theme.warn))),
        (None, Some(left)) if app.rate.is_low() => Some(Span::styled(format!("{left} requests left"), Style::default().fg(theme.warn))),
        _ => None,
    };
    let right = if let Some(rate) = rate {
        Line::from(vec![rate, dot.clone(), Span::styled("? help ", muted)])
    } else if app.loading() {
        Line::from(vec![
            Span::styled(spinner(app.now.duration_since(app.started)), Style::default().fg(theme.accent)),
            Span::styled(" loading", muted),
            dot,
            Span::styled("? help ", muted),
        ])
    } else {
        Line::from(Span::styled("? help ", muted))
    };
    let [l, r] = Layout::horizontal([Constraint::Min(10), Constraint::Length(right.width() as u16)]).areas(area);
    f.render_widget(Paragraph::new(left), l);
    f.render_widget(Paragraph::new(right).alignment(Alignment::Right), r);
}

/// Everything in `area` takes one colour, so the eye finds the focused pane without a border change.
pub fn fade(f: &mut Frame, area: Rect, color: Color) {
    let buf = f.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_fg(color);
                cell.modifier.remove(Modifier::BOLD);
            }
        }
    }
}

/// The viewport only moves when the selection leaves it.
pub fn settle_scroll(scroll: usize, selected: usize, height: usize) -> usize {
    if height == 0 {
        return 0;
    }
    if selected < scroll {
        selected
    } else if selected >= scroll + height {
        selected + 1 - height
    } else {
        scroll
    }
}

pub fn spinner(elapsed: Duration) -> &'static str {
    SPINNER[(elapsed.as_millis() / SPINNER_FRAME.as_millis()) as usize % SPINNER.len()]
}

pub fn truncate(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    let mut out = String::new();
    for c in text.chars() {
        if out.width() + 1 >= width {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

pub fn short_age(age: Duration) -> String {
    let secs = age.as_secs();
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    }
}

#[cfg(test)]
mod columns_tests {
    use super::*;

    #[test]
    fn the_pane_takes_what_the_width_allows() {
        assert_eq!(columns(160, true, false, false), Columns { queue: WIDE_QUEUE_W, side: 64, diff: true }, "three columns from 150");
        assert_eq!(columns(155, true, false, false), Columns { queue: QUEUE_W, side: 62, diff: true });
        assert_eq!(columns(130, true, true, false), Columns { queue: 0, side: 52, diff: true }, "the queue steps aside from 120");
        assert_eq!(columns(100, true, true, false), Columns { queue: 0, side: 100, diff: false }, "a page of its own below 120");
        assert_eq!(columns(100, true, false, false), Columns { queue: 0, side: 0, diff: true }, "h goes back to the diff at the same line");
        assert_eq!(columns(90, true, true, false).side, 90);
        assert_eq!(columns(80, false, false, false), Columns { queue: QUEUE_W, side: 0, diff: true });
    }

    #[test]
    fn the_queue_widens_from_160_columns() {
        assert_eq!(columns(WIDE_QUEUE_FROM - 1, false, false, false).queue, QUEUE_W);
        assert_eq!(columns(WIDE_QUEUE_FROM, false, false, false).queue, WIDE_QUEUE_W);
        assert_eq!(columns(240, false, false, false).queue, WIDE_QUEUE_W);
    }

    #[test]
    fn reading_mode_never_puts_the_pane_beside_the_diff() {
        assert_eq!(columns(200, false, false, true), Columns { queue: 0, side: 0, diff: true });
        assert_eq!(columns(200, true, true, true), Columns { queue: 0, side: 200, diff: false }, "the pane opens the narrow way");
        assert_eq!(columns(200, true, false, true), Columns { queue: 0, side: 0, diff: true });
    }

    #[test]
    fn the_pane_is_never_narrower_than_36_columns() {
        assert_eq!(columns(80, true, false, false).side, 0);
        assert_eq!(SIDE_W.max(80 * SIDE_PCT / 100), 36);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn settle_scroll_only_moves_when_the_selection_leaves_the_window() {
        assert_eq!(settle_scroll(0, 3, 10), 0);
        assert_eq!(settle_scroll(0, 10, 10), 1);
        assert_eq!(settle_scroll(5, 2, 10), 2);
        assert_eq!(settle_scroll(5, 30, 0), 0);
    }

    #[test]
    fn truncate_keeps_the_width_and_ends_with_an_ellipsis() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("a rather long title", 8), "a rathe…");
        assert_eq!(truncate("héllo wörld", 6).width(), 6);
    }

    #[test]
    fn ages_are_one_unit() {
        assert_eq!(short_age(Duration::from_secs(5)), "5s");
        assert_eq!(short_age(Duration::from_secs(600)), "10m");
        assert_eq!(short_age(Duration::from_secs(7200)), "2h");
        assert_eq!(short_age(Duration::from_secs(200_000)), "2d");
    }
}
