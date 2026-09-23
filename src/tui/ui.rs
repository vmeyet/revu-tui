use super::app::{App, Badge, Focus, QueueRow};
use super::theme::Theme;
use super::{brief_view, diff_view, publish_view, thread_view};
use crate::forge::Kind;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph};
use std::time::Duration;
use unicode_width::UnicodeWidthStr;

const QUEUE_W: u16 = 34;
const SIDE_W: u16 = 32;
const SIDE_PCT: u16 = 40;
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const SPINNER_FRAME: Duration = Duration::from_millis(80);
const SKELETON_ROWS: usize = 3;

pub const HELP: [(&str, &str); 37] = [
    ("j k", "move"),
    ("g G", "first, last"),
    ("^d ^u", "half page"),
    ("h l", "pane to the left; open the MR, pane to the right"),
    ("enter", "open the MR, the thread, or toggle the fold"),
    ("esc", "back: close the thread, then the queue"),
    ("/", "filter the queue"),
    ("*", "in the queue: this repo only, or every project"),
    ("i", "the MR description"),
    ("r", "refresh"),
    ("o", "open in the browser (the line, in a diff)"),
    ("y", "copy the URL"),
    ("click !42", "open the MR, in terminals that follow links"),
    ("tab S-tab", "next, previous file"),
    ("]c [c", "next, previous hunk"),
    ("]n [n", "next, previous thread"),
    ("]f [f", "next, previous file with an open thread"),
    ("za", "toggle the fold under the cursor"),
    ("D", "changed words inline, or every line split"),
    ("zc zo", "close, open"),
    ("zM zR", "fold, unfold every file"),
    ("zo", "in the queue: show the done section"),
    ("c", "comment on the line, as a draft"),
    ("C", "on a changed pair: comment on the old side"),
    ("V", "select lines: c comments on them, y copies them"),
    ("E", "write the comment in $EDITOR"),
    ("s", "suggestion in the editor, prefilled with the lines"),
    ("enter d", "on a draft: edit, delete"),
    ("P", "publish the drafts (a to also approve)"),
    ("A", "approve, unapprove"),
    ("r", "in a thread: reply, as a draft"),
    ("R", "in a thread: resolve, unresolve"),
    ("u", "in a thread: open its first link"),
    ("esc", "drop the selection, close the input"),
    ("?", "this help"),
    ("q", "quit"),
    ("^c", "quit, always"),
];

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
    let input_rows = u16::from(app.filtering || app.input.is_some());
    let [main, input, status] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(input_rows), Constraint::Length(1)]).areas(f.area());
    let side_open = app.open.as_ref().is_some_and(|o| o.thread.is_some());
    let side_w = if side_open { SIDE_W.max(main.width * SIDE_PCT / 100) } else { 0 };
    let [queue, review, side] =
        Layout::horizontal([Constraint::Length(QUEUE_W), Constraint::Min(40), Constraint::Length(side_w)]).areas(main);
    draw_queue(f, app, queue);
    diff_view::draw(f, app, review);
    if side_open {
        thread_view::draw(f, app, side);
    }
    if app.input.is_some() {
        draw_input(f, app, input);
    } else if app.filtering {
        draw_filter(f, app, input);
    }
    draw_status(f, app, status);
    let modal = app.help || app.publish.is_some() || app.brief.is_some();
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
    if let Some(publish) = app.publish.clone() {
        publish_view::draw(f, app, &publish, main);
    }
    if app.brief.is_some() {
        brief_view::draw(f, app, main);
    }
    if app.help {
        draw_help(f, app, main);
    }
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

fn draw_queue(f: &mut Frame, app: &mut App, area: Rect) {
    let theme = app.theme;
    let scope = app.scope().unwrap_or_else(|| "all".to_owned());
    let title = truncate(&format!("Queue · {scope}"), area.width.saturating_sub(4) as usize);
    let block = pane(theme, &title, app.focus == Focus::Queue);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if app.sections.is_none() {
        return draw_skeleton(f, theme, inner);
    }
    if app.queue_is_empty() {
        return draw_empty(f, theme, inner, &["✓", "", "nothing waits on you", "r to refresh · / to filter"]);
    }
    let height = inner.height as usize;
    app.queue_scroll = settle_scroll(app.queue_scroll, app.queue_selected, height);
    let rows = app.queue_rows();
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(app.queue_scroll)
        .take(height)
        .map(|(i, row)| queue_line(app, row, i == app.queue_selected, inner.width as usize))
        .collect();
    let links = queue_links(&rows, app.kind, app.queue_scroll, inner);
    f.render_widget(Paragraph::new(lines), inner);
    app.links.extend(links);
}

/// Where each visible `!iid` lands: two cells in, after the cursor bar.
pub fn queue_links(rows: &[QueueRow<'_>], kind: Kind, scroll: usize, inner: Rect) -> Vec<Link> {
    rows.iter()
        .enumerate()
        .skip(scroll)
        .take(inner.height as usize)
        .filter_map(|(i, row)| match row {
            QueueRow::Mr(mr) => {
                let text = format!("{}{}", kind.sigil(), mr.number);
                Some(Link { x: inner.x + 2, y: inner.y + (i - scroll) as u16, text, url: mr.web_url.clone() })
            }
            QueueRow::Section { .. } => None,
        })
        .collect()
}

fn queue_line<'a>(app: &App, row: &QueueRow<'_>, selected: bool, width: usize) -> Line<'a> {
    let theme = app.theme;
    match row {
        QueueRow::Section { name, count, open } => {
            let count = count.to_string();
            let mark = if *open { "" } else { " ▸" };
            let pad = width.saturating_sub(name.width() + mark.width() + count.width() + 2);
            Line::from(vec![
                Span::styled(format!("  {name}{mark}"), Style::default().fg(theme.faded)),
                Span::styled(format!("{}{count}", " ".repeat(pad)), Style::default().fg(theme.faded)),
            ])
        }
        QueueRow::Mr(mr) => {
            let badge = app.badge(mr).map(|b| badge_span(app, b));
            let iid = format!("{}{} ", app.kind.sigil(), mr.number);
            let room = width.saturating_sub(2 + iid.width() + 2);
            let title = truncate(&mr.title, room);
            let pad = room.saturating_sub(title.width()) + 1;
            let title_style = if selected { Style::default().add_modifier(Modifier::BOLD) } else { Style::default() };
            let mut spans = vec![
                Span::styled(if selected { "▎ " } else { "  " }, Style::default().fg(theme.accent)),
                Span::styled(iid, Style::default().fg(theme.muted)),
                Span::styled(title, title_style),
                Span::raw(" ".repeat(pad)),
            ];
            spans.extend(badge);
            Line::from(spans)
        }
    }
}

fn badge_span<'a>(app: &App, badge: Badge) -> Span<'a> {
    let theme = app.theme;
    match badge {
        Badge::Failed => Span::styled("✗", Style::default().fg(theme.danger)),
        Badge::Running => Span::styled(spinner(app.now.duration_since(app.started)), Style::default().fg(theme.muted)),
        Badge::Activity => Span::styled("●", Style::default().fg(theme.accent)),
        Badge::Approved => Span::styled("✓", Style::default().fg(theme.success)),
        Badge::Draft => Span::styled("D", Style::default().fg(theme.muted)),
    }
}

fn draw_skeleton(f: &mut Frame, theme: Theme, area: Rect) {
    let faded = Style::default().fg(theme.faded);
    let mut lines = vec![];
    for name in ["TO REVIEW", "MINE", "WATCHING", "OPEN"] {
        lines.push(Line::from(Span::styled(format!("  {name}"), faded)));
        for _ in 0..SKELETON_ROWS {
            lines.push(Line::from(Span::styled("   ▁▁▁ ▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁", faded)));
        }
    }
    f.render_widget(Paragraph::new(lines), area);
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

fn draw_input(f: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme;
    let (before, under, after) = app.buffer.split();
    let caret = if under.is_empty() { " " } else { under };
    let line = Line::from(vec![
        Span::styled(format!(" {} ", app.input_label()), Style::default().fg(theme.accent)),
        Span::raw(before.to_owned()),
        Span::styled(caret.to_owned(), Style::default().add_modifier(Modifier::REVERSED)),
        Span::raw(after.to_owned()),
        Span::styled("  enter save · esc cancel", Style::default().fg(theme.faded)),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme;
    let muted = Style::default().fg(theme.muted);
    let dot = Span::styled(" · ", Style::default().fg(theme.faded));
    let left = match (app.live_toast(), app.offline, &app.open) {
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
    let right = if app.loading() {
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

fn draw_help(f: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme;
    let lines: Vec<Line> = HELP
        .iter()
        .map(|(key, what)| {
            Line::from(vec![
                Span::styled(format!("{key:>10}  "), Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
                Span::raw(*what),
            ])
        })
        .collect();
    let height = (lines.len() as u16 + 2).min(area.height);
    let width = 68.min(area.width);
    let popup = Rect { x: area.x + (area.width - width) / 2, y: area.y + (area.height - height) / 2, width, height };
    let block = pane(theme, "keys", true);
    f.render_widget(Clear, popup);
    f.render_widget(Paragraph::new(lines).block(block).style(Style::default().bg(theme.surface)), popup);
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

    #[test]
    fn queue_links_point_at_each_visible_iid() {
        let sections = crate::forge::gitlab::fixture::queue(include_str!("../forge/gitlab/fixtures/queue.json")).sections(&[]);
        let rows = vec![
            QueueRow::Section { name: "TO REVIEW", count: 1, open: true },
            QueueRow::Mr(&sections.to_review[0]),
            QueueRow::Section { name: "MINE", count: 1, open: true },
            QueueRow::Mr(&sections.mine[0]),
        ];
        let inner = Rect { x: 2, y: 1, width: 30, height: 3 };
        let links = queue_links(&rows, Kind::GitLab, 1, inner);
        assert_eq!(links.len(), 2, "sections carry no link and the window stops at the height");
        assert_eq!((links[0].x, links[0].y, links[0].text.as_str()), (4, 1, "!42"));
        assert_eq!(links[0].url, "https://gitlab.com/acme/widgets/-/merge_requests/42");
        assert_eq!((links[1].y, links[1].text.as_str()), (3, "!41"));
    }
}
