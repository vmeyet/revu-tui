use super::app::{App, Badge, Focus, Mark, QueueRow};
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
const SKELETON_ROWS: usize = 3;

pub const HELP: [(&str, &str); 54] = [
    ("j k", "move"),
    ("g G", "first, last"),
    ("^d ^u", "half page"),
    ("h l", "pane to the left; open the MR, or a marked line's threads"),
    ("enter", "open the MR, a marked line's threads, or toggle the fold"),
    ("esc x", "close the right pane; esc again goes back to the queue"),
    ("/", "filter the queue"),
    ("*", "in the queue: this repo only, or every project"),
    ("i", "the MR description"),
    ("r", "refresh"),
    ("o", "open in the browser (the line, in a diff)"),
    ("y", "copy the URL"),
    ("click !42", "open the MR, in terminals that follow links"),
    ("tab S-tab", "next, previous file"),
    ("]c [c", "next, previous hunk"),
    ("]n [n", "next, previous line with a conversation; the pane follows"),
    ("]f [f", "next, previous file with an open thread"),
    ("za", "toggle the fold under the cursor"),
    ("D", "changed words inline, or every line split"),
    ("zc zo", "close, open"),
    ("zM zR", "fold, unfold every file"),
    ("zo zc", "in the queue: open, fold the section (enter too)"),
    ("zh", "fold the MR header to one row"),
    ("t", "file tree: enter jumps to a file or folds a folder, t closes"),
    ("p", "pipeline: jobs by stage, failures first; o opens a job, r refreshes, p closes"),
    ("zv", "mark the file viewed: it folds, and comes back if it changes"),
    ("zz", "reading mode: the diff alone, centered; h brings the queue back"),
    ("w", "wrap long lines under their text"),
    ("W", "hide changes that are only whitespace (shown as ≈)"),
    ("+", "ten more unchanged lines above and below the hunk"),
    ("v", "the file after the change in your program, at this line ([open] in config)"),
    ("c", "new thread on the line: a compose box opens in the pane"),
    ("C", "on a changed pair: comment on the old side"),
    ("V", "select lines: c comments on them, y copies them"),
    ("E", "write the comment in $EDITOR"),
    ("s", "new thread prefilled with a suggestion of the lines"),
    ("⌥enter ^o", "in the box: newline, move the text to $EDITOR"),
    ("e d", "in the pane: edit, delete my draft"),
    ("J K", "in the pane: next, previous thread on the line"),
    ("P", "publish: enter sends, e edits, m moves a lost draft to the MR"),
    ("a e r s", "ask Claude: explain the hunk, risks of the file, summary of the MR"),
    ("a t c a", "ask Claude: this thread, a comment about the lines, anything"),
    ("c ⏎ R y", "in an answer: make it a draft, follow up, ask again, copy"),
    ("A", "approve, unapprove"),
    ("r", "in a thread: reply, as a draft"),
    ("R", "resolve, unresolve: in the pane, or on a marked line"),
    ("S", "in the pane: commit the note's suggestion on the MR branch, after a y"),
    ("u", "in a thread: open its first link"),
    ("esc", "drop the selection; in the box, leave it, the text stays"),
    (":", "command line: :go !42 · :view old · :set theme=nord · tab completes"),
    ("^k", "jump to a file of the MR, or to another MR"),
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
    let input_rows = u16::from(app.filtering || app.palette.is_some());
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
        draw_queue(f, app, queue);
    }
    if shown.diff {
        diff_view::draw(f, app, review);
    }
    if app.open.as_ref().is_some_and(|o| o.answer.is_some()) && side_open {
        super::answer_view::draw(f, app, side);
    } else if app.open.as_ref().is_some_and(|o| o.pipeline.is_some()) && side_open {
        super::pipeline_view::draw(f, app, side);
    } else if app.open.as_ref().is_some_and(|o| o.tree.is_some()) && side_open {
        super::tree_view::draw(f, app, side);
    } else if side_open {
        thread_view::draw(f, app, side);
    }
    if let Some(palette) = &app.palette {
        draw_palette(f, app, palette, input);
    } else if app.filtering {
        draw_filter(f, app, input);
    }
    draw_status(f, app, status);
    let modal = app.help.is_some() || app.publish.is_some() || app.brief.is_some() || app.jump.is_some();
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
    if let Some(jump) = &app.jump {
        super::jump::draw(f, jump, main, app.theme);
    }
    if let Some(scroll) = app.help {
        draw_help(f, app, main, scroll);
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
    let queue = if reading { 0 } else { QUEUE_W };
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
            let mark = app.triaged().then(|| mark_span(app, app.mark(mr)));
            let iid = format!("{}{} ", app.kind.sigil(), mr.number);
            let room = width.saturating_sub(2 + mark.as_ref().map_or(0, Span::width) + iid.width() + 2);
            let title = truncate(&mr.title, room);
            let pad = room.saturating_sub(title.width()) + 1;
            let title_style = if selected { Style::default().add_modifier(Modifier::BOLD) } else { Style::default() };
            let mut spans = vec![Span::styled(if selected { "▎ " } else { "  " }, Style::default().fg(theme.accent))];
            spans.extend(mark);
            spans.extend([
                Span::styled(iid, Style::default().fg(theme.muted)),
                Span::styled(title, title_style),
                Span::raw(" ".repeat(pad)),
            ]);
            spans.extend(badge);
            Line::from(spans)
        }
    }
}

/// The activity dot breathes: bright one second, faded the next.
fn pulse_on(app: &App) -> bool {
    app.now.duration_since(app.started).as_secs().is_multiple_of(2)
}

/// Jev's mark, two cells wide so titles stay aligned whether a row has one or not.
fn mark_span<'a>(app: &App, mark: Option<Mark>) -> Span<'a> {
    let theme = app.theme;
    match mark {
        Some(Mark::WaitsOnMe) => Span::styled("◆ ", Style::default().fg(if pulse_on(app) { theme.warn } else { theme.faded })),
        Some(Mark::Urgent) => Span::styled("! ", Style::default().fg(theme.danger).add_modifier(Modifier::BOLD)),
        Some(Mark::Sprawling) => Span::styled("~ ", Style::default().fg(theme.muted)),
        None => Span::raw("  "),
    }
}

fn badge_span<'a>(app: &App, badge: Badge) -> Span<'a> {
    let theme = app.theme;
    match badge {
        Badge::Failed => Span::styled("✗", Style::default().fg(theme.danger)),
        Badge::Running => Span::styled(spinner(app.now.duration_since(app.started)), Style::default().fg(theme.muted)),
        Badge::Activity => Span::styled("●", Style::default().fg(if pulse_on(app) { theme.accent } else { theme.faded })),
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

/// The `:` line: what is typed, the ghost of the best completion, and the options while tab cycles.
fn draw_palette(f: &mut Frame, app: &App, palette: &super::palette::Palette, area: Rect) {
    let theme = app.theme;
    let candidates = app.completions_for(&palette.input);
    let mut spans =
        vec![Span::styled(":", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)), Span::raw(palette.input.clone())];
    match (palette.hint(), palette.ghost(&candidates)) {
        (Some(hint), _) => spans.push(Span::styled(format!("   {hint}"), Style::default().fg(theme.muted))),
        (None, Some(ghost)) => spans.push(Span::styled(ghost, Style::default().fg(theme.faded))),
        (None, None) => {}
    }
    spans.insert(2, Span::styled(" ", Style::default().add_modifier(Modifier::REVERSED)));
    spans.push(Span::styled("  tab completes · enter runs · esc", Style::default().fg(theme.faded)));
    f.render_widget(Paragraph::new(Line::from(spans)), area);
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

/// The key list, scrolled by `scroll` rows when it does not fit; the title says how to move.
fn draw_help(f: &mut Frame, app: &App, area: Rect, scroll: usize) {
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
    let visible = usize::from(height.saturating_sub(2));
    let overflow = lines.len() > visible;
    let top = scroll.min(lines.len().saturating_sub(visible));
    let title =
        if overflow { format!("keys · {}–{} of {} · j k scroll", top + 1, top + visible, lines.len()) } else { "keys".to_owned() };
    let block = pane(theme, &title, true);
    f.render_widget(Clear, popup);
    f.render_widget(Paragraph::new(lines).block(block).style(Style::default().bg(theme.surface)).scroll((top as u16, 0)), popup);
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
        assert_eq!(columns(160, true, false, false), Columns { queue: QUEUE_W, side: 64, diff: true }, "three columns from 150");
        assert_eq!(columns(130, true, true, false), Columns { queue: 0, side: 52, diff: true }, "the queue steps aside from 120");
        assert_eq!(columns(100, true, true, false), Columns { queue: 0, side: 100, diff: false }, "a page of its own below 120");
        assert_eq!(columns(100, true, false, false), Columns { queue: 0, side: 0, diff: true }, "h goes back to the diff at the same line");
        assert_eq!(columns(90, true, true, false).side, 90);
        assert_eq!(columns(80, false, false, false), Columns { queue: QUEUE_W, side: 0, diff: true });
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
