use super::app::{App, Focus};
use super::theme::Theme;
use super::{brief_view, diff_view, publish_view, thread_view};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Padding, Paragraph};
use std::time::Duration;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// What marks a review app: where the MR's branch runs.
pub const DEPLOYED: &str = "⧉ ";

const QUEUE_W: u16 = 34;
/// The queue on a wide terminal: ten more columns show about twice the title.
const WIDE_QUEUE_W: u16 = 44;
/// From this terminal width the queue takes `WIDE_QUEUE_W`.
const WIDE_QUEUE_FROM: u16 = 160;
const SIDE_W: u16 = 36;
/// From this width the queue, the diff and the right pane sit side by side.
const WIDE: u16 = 150;
/// From this width the diff and the right pane share the screen; below, the pane is a page of its own.
const MEDIUM: u16 = 120;
const SIDE_PCT: u16 = 40;
/// Zen's column, unless `[tui] zen_width` fixes it: this share of the screen, never under `ZEN_MIN_W`.
const ZEN_PCT: u32 = 70;
/// A comfortable line of code with both gutters and the sign.
const ZEN_MIN_W: u16 = 120;
/// From this many rows zen's column holds the diff over the thread pane; shorter, the pane is a page of its own.
const ZEN_SPLIT_FROM: u16 = 24;
/// The thread pane's share of zen's column under the diff, and while a comment is being written.
const ZEN_PANE_PCT: u16 = 25;
const ZEN_COMPOSE_PCT: u16 = 50;
const ZEN_PANE_MIN: u16 = 8;
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
    app.text_rows.clear();
    app.list_rows.clear();
    let input_rows = u16::from(app.filtering);
    let status_rows = u16::from(
        !app.zen
            || app.confirm.is_some()
            || app.pending == Some('\'')
            || app.quit_prompt().is_some()
            || app.react.is_some()
            || app.search.is_some(),
    );
    let [main, input, status] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(input_rows), Constraint::Length(status_rows)]).areas(f.area());
    let side_open = app.open.as_ref().is_some_and(super::app::Open::side_open);
    let shown = columns(main.width, side_open, app.focus == Focus::Side, app.zen);
    let diff = if shown.diff { Constraint::Min(1) } else { Constraint::Length(0) };
    let side_width = if shown.diff { Constraint::Length(shown.side) } else { Constraint::Min(0) };
    let [queue, review, side] = Layout::horizontal([Constraint::Length(shown.queue), diff, side_width]).areas(main);
    let (shown, review, side) = if app.zen { zen_areas(app, main, shown, [review, side]) } else { (shown, review, side) };
    let side_open = side_open && shown.side > 0;
    let split = app.zen && shown.diff && side_open;
    let hidden = Rect::default();
    app.areas = super::app::Areas {
        queue: if shown.queue > 0 { queue } else { hidden },
        review: if shown.diff { review } else { hidden },
        side: if side_open { side } else { hidden },
    };
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
    } else if app.open.as_ref().is_some_and(|o| o.outline.is_some()) && side_open {
        super::outline_view::draw(f, app, side);
    } else if app.open.as_ref().is_some_and(|o| o.tree.is_some()) && side_open {
        super::tree_view::draw(f, app, side);
    } else if side_open {
        pictures = thread_view::draw(f, app, side, split);
    }
    if app.filtering {
        draw_filter(f, app, input);
    }
    if status_rows > 0 {
        draw_status(f, app, status);
    }
    let modal = app.modal();
    if modal {
        app.links.clear();
    }
    if app.focus != Focus::Queue || modal {
        fade(f, queue, app.theme, app.theme.faded);
    }
    if app.focus != Focus::Review || modal {
        fade(f, review, app.theme, if modal { app.theme.faded } else { app.theme.muted });
    }
    if side_open && (app.focus != Focus::Side || modal) {
        fade(f, side, app.theme, app.theme.faded);
    }
    if !modal {
        draw_pictures(f, app, &pictures);
    }
    if let Some(drag) = app.drag {
        drag.paint(&app.text_rows, f.buffer_mut());
    }
    if app.zen {
        draw_zen_overlays(f, app, main);
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
    if let Some(help) = app.help {
        super::help::draw(f, app, main, help);
    }
}

/// In zen: the MR just switched to, on top for a moment, and a fresh toast on the bottom row.
fn draw_zen_overlays(f: &mut Frame, app: &App, area: Rect) {
    let theme = app.theme;
    if let Some(text) = app.zen_banner(app.now) {
        let text = truncate(text, usize::from(area.width.saturating_sub(4)));
        let line = Line::from(Span::styled(format!(" {text} "), Style::default().fg(theme.accent).bg(theme.surface)));
        let width = u16::try_from(line.width()).unwrap_or(area.width).min(area.width);
        let top = Rect { x: area.x + (area.width - width) / 2, y: area.y, width, height: 1 };
        f.render_widget(Paragraph::new(line), top);
    }
    if let Some(toast) = app.live_toast().filter(|t| t.fresh(app.now)) {
        let colour = if toast.danger { theme.danger } else { theme.muted };
        let line = Line::from(Span::styled(truncate(&toast.text, usize::from(area.width)), Style::default().fg(colour)));
        let bottom = Rect { y: area.bottom().saturating_sub(1), height: 1, ..area };
        f.render_widget(Paragraph::new(line).alignment(Alignment::Center), bottom);
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
/// and below that, or in a zen column too short to split, the pane is a page of its own while it has the keys.
fn columns(width: u16, side_open: bool, side_focused: bool, zen: bool) -> Columns {
    let side = SIDE_W.max(width * SIDE_PCT / 100);
    let queue = match (zen, width) {
        (true, _) => 0,
        (false, WIDE_QUEUE_FROM..) => WIDE_QUEUE_W,
        (false, _) => QUEUE_W,
    };
    match (side_open, width) {
        (false, _) => Columns { queue, side: 0, diff: true },
        (true, WIDE..) if !zen => Columns { queue, side, diff: true },
        (true, MEDIUM..) if !zen => Columns { queue: 0, side, diff: true },
        (true, _) if side_focused => Columns { queue: 0, side: width, diff: false },
        (true, _) => Columns { queue: 0, side: 0, diff: true },
    }
}

/// Zen's column for the diff and the right pane: the diff over the thread pane when the column is
/// tall enough, else the one `shown` gives it.
fn zen_areas(app: &App, main: Rect, shown: Columns, [review, side]: [Rect; 2]) -> (Columns, Rect, Rect) {
    let side_by_side = app.open.as_ref().is_some_and(|o| o.review.side_by_side);
    let width = if side_by_side { main.width } else { zen_width(app.zen_width, main.width) };
    let column = zen_column(main, width);
    let threads = app.open.as_ref().is_some_and(super::app::Open::shows_threads);
    let needs = threads.then(|| thread_view::rows_needed(app, column.width));
    match needs.and_then(|needs| zen_split(column, needs, app.input.is_some())) {
        Some([diff, pane]) => (Columns { queue: 0, side: column.width, diff: true }, diff, pane),
        None => (shown, zen_column(review, width), zen_column(side, width)),
    }
}

/// `column` as the diff over the thread pane: the pane takes what it `needs`, up to a quarter of
/// the column (half while `composing`) and never fewer than `ZEN_PANE_MIN` rows for that share.
/// A column too short for both gives none.
fn zen_split(column: Rect, needs: u16, composing: bool) -> Option<[Rect; 2]> {
    if column.height < ZEN_SPLIT_FROM {
        return None;
    }
    let share = if composing { ZEN_COMPOSE_PCT } else { ZEN_PANE_PCT };
    let most = (column.height * share / 100).max(ZEN_PANE_MIN);
    Some(Layout::vertical([Constraint::Min(1), Constraint::Length(needs.min(most))]).areas(column))
}

/// Zen's column: `fixed` columns when `[tui] zen_width` sets it, else a share of the screen that grows with it.
fn zen_width(fixed: Option<u16>, screen: u16) -> u16 {
    let share = u16::try_from(u32::from(screen) * ZEN_PCT / 100).unwrap_or(screen);
    fixed.unwrap_or(share.max(ZEN_MIN_W)).min(screen)
}

/// `area` narrowed to `width` columns in its middle, below the row the zen banner takes.
fn zen_column(area: Rect, width: u16) -> Rect {
    let width = width.min(area.width);
    Rect { x: area.x + (area.width - width) / 2, y: area.y + 1, width, height: area.height.saturating_sub(1) }
}

pub fn pane(theme: Theme, title: &str, focused: bool) -> Block<'static> {
    let title_style =
        if focused { Style::default().fg(theme.accent).add_modifier(Modifier::BOLD) } else { Style::default().fg(theme.faded) };
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if focused { theme.border_focus } else { theme.border }))
        .title(Span::styled(format!(" {title} "), title_style))
        .padding(Padding::horizontal(1))
}

/// A right pane's frame; in zen, only a faded title line over the content, both where the diff's text starts.
pub fn side_pane(theme: Theme, title: &str, focused: bool, zen: bool) -> Block<'static> {
    if !zen {
        return pane(theme, title, focused);
    }
    Block::default().title(Span::styled(format!(" {title}"), Style::default().fg(theme.faded))).padding(Padding::new(1, 1, 1, 0))
}

/// Zen's thread pane under the diff: a faded rule carrying its title, no frame.
pub fn rule_pane(theme: Theme, title: &str) -> Block<'static> {
    let title = Line::from(vec![
        Span::styled("─ ", Style::default().fg(theme.border)),
        Span::styled(format!("{title} "), Style::default().fg(theme.faded)),
    ]);
    Block::new().borders(Borders::TOP).border_style(Style::default().fg(theme.border)).title(title).padding(Padding::horizontal(1))
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
    if let Some(prompt) = app.quit_prompt() {
        f.render_widget(Paragraph::new(Line::from(Span::styled(format!(" {prompt}"), Style::default().fg(theme.warn)))), area);
        return;
    }
    if app.offer.is_some() {
        let line = Line::from(vec![
            Span::styled(" next MR that needs you: ", muted),
            Span::styled("enter", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
            Span::styled(" next MR · ", muted),
            Span::styled("esc", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
            Span::styled(" stay", muted),
        ]);
        f.render_widget(Paragraph::new(line), area);
        return;
    }
    if let Some(search) = &app.search {
        let count = app.search_count().map(|c| format!("  {c}")).unwrap_or_default();
        let (cursor, hint) = if search.typing { ("▏", "  enter keep · esc back") } else { ("", "  n N next, previous · esc") };
        let line = Line::from(vec![
            Span::styled(format!(" /{}{cursor}", search.query), Style::default().fg(theme.accent)),
            Span::styled(count, Style::default().fg(theme.muted)),
            Span::styled(hint, muted),
        ]);
        f.render_widget(Paragraph::new(line), area);
        return;
    }
    if let Some(prompt) = app.react_prompt() {
        let mut spans = vec![Span::styled(" react ", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD))];
        if let Some(query) = &prompt.search {
            spans.push(Span::styled(format!("/{query}▏"), Style::default().fg(theme.accent)));
        }
        let hint = match (&prompt.search, prompt.more) {
            (Some(_), _) if prompt.choices.is_empty() => "  no emoji by that name · esc",
            (Some(_), _) => "  ← → enter · esc",
            (None, true) => "  1-8 or h l enter · / any emoji · esc",
            (None, false) => "  1-8 or h l enter · esc",
        };
        for (text, mine, selected) in prompt.choices {
            let colour = if mine { theme.accent } else { theme.muted };
            let style = if selected { Style::default().fg(colour).add_modifier(Modifier::REVERSED) } else { Style::default().fg(colour) };
            spans.extend([Span::raw(" "), Span::styled(format!(" {text} "), style)]);
        }
        spans.push(Span::styled(hint, muted));
        f.render_widget(Paragraph::new(Line::from(spans)), area);
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

/// The text in `area` takes one colour and its frames the quiet `border`: `color` is brighter than any
/// frame, so fading a frame would light the unfocused pane's box above the focused one's.
pub fn fade(f: &mut Frame, area: Rect, theme: Theme, color: Color) {
    let buf = f.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                let frame = cell.fg == theme.border || cell.fg == theme.border_focus;
                cell.set_fg(if frame { theme.border } else { color });
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

/// `text` cut to `width` columns, an ellipsis in the last one when it does not fit.
/// `2 passed · 1 failed`: each count in its colour, zeros left out.
pub fn counts<'a>(parts: &[(usize, &str, Color)], theme: Theme) -> Line<'a> {
    let mut spans = vec![Span::raw(" ")];
    for (count, label, colour) in parts.iter().filter(|(count, _, _)| *count > 0) {
        if spans.len() > 1 {
            spans.push(Span::styled(" · ", Style::default().fg(theme.faded)));
        }
        spans.push(Span::styled(format!("{count} {label}"), Style::default().fg(*colour)));
    }
    Line::from(spans)
}

pub fn truncate(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    with_ellipsis(text.chars(), width)
}

/// `text` with each tab drawn as `tab`, cut as `truncate` cuts, without drawing the tabs of a long line past the cut.
pub fn truncate_tabs(text: &str, tab: &str, width: usize) -> String {
    let drawn = text.split('\t').map(UnicodeWidthStr::width).sum::<usize>() + text.matches('\t').count() * tab.width();
    if drawn <= width {
        return text.replace('\t', tab);
    }
    let chars = text.split('\t').enumerate().flat_map(|(i, piece)| if i == 0 { "" } else { tab }.chars().chain(piece.chars()));
    with_ellipsis(chars, width)
}

/// The first `chars` that fit in `width - 1` columns, then `…`.
fn with_ellipsis(chars: impl Iterator<Item = char>, width: usize) -> String {
    let room = width.saturating_sub(1);
    let mut used = 0;
    let mut out: String = chars
        .take_while(|&c| {
            used += UnicodeWidthChar::width(c).unwrap_or(0);
            used <= room
        })
        .collect();
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
    fn zen_never_puts_the_pane_beside_the_diff() {
        assert_eq!(columns(200, false, false, true), Columns { queue: 0, side: 0, diff: true });
        assert_eq!(columns(200, true, true, true), Columns { queue: 0, side: 200, diff: false }, "the pane opens the narrow way");
        assert_eq!(columns(200, true, false, true), Columns { queue: 0, side: 0, diff: true });
    }

    #[test]
    fn zen_takes_70_percent_of_the_screen_unless_the_config_fixes_it() {
        assert_eq!(zen_width(None, 200), 140);
        assert_eq!(zen_width(None, 138), 120, "never under 120");
        assert_eq!(zen_width(None, 120), 120, "the whole screen up to 120");
        assert_eq!(zen_width(None, 80), 80, "never wider than the screen");
        assert_eq!(zen_width(Some(90), 200), 90, "the config wins");
        assert_eq!(zen_width(Some(120), 100), 100);
    }

    #[test]
    fn the_zen_column_sits_in_the_middle_under_the_banner_row() {
        assert_eq!(zen_column(Rect::new(0, 0, 160, 40), 112), Rect::new(24, 1, 112, 39));
        assert_eq!(zen_column(Rect::new(0, 0, 80, 40), 100), Rect::new(0, 1, 80, 39));
    }

    fn pane_rows(height: u16, needs: u16, composing: bool) -> Option<u16> {
        zen_split(Rect::new(9, 1, 120, height), needs, composing).map(|[_, pane]| pane.height)
    }

    #[test]
    fn zen_puts_the_diff_over_the_thread_pane_a_quarter_of_the_column_at_most() {
        let split = zen_split(Rect::new(9, 1, 120, 39), 30, false);
        assert_eq!(split, Some([Rect::new(9, 1, 120, 30), Rect::new(9, 31, 120, 9)]), "a quarter of 39 rows");
        assert_eq!(pane_rows(59, 30, false), Some(14));
        assert_eq!(pane_rows(99, 60, false), Some(24));
    }

    #[test]
    fn the_pane_under_the_diff_never_takes_more_than_it_needs() {
        assert_eq!(pane_rows(39, 5, false), Some(5));
        assert_eq!(pane_rows(59, 9, false), Some(9));
    }

    #[test]
    fn the_pane_under_the_diff_may_take_8_rows_when_a_quarter_is_fewer() {
        assert_eq!(pane_rows(24, 30, false), Some(8));
        assert_eq!(pane_rows(29, 30, false), Some(8));
        assert_eq!(pane_rows(24, 6, false), Some(6), "still no more than it needs");
    }

    #[test]
    fn composing_lets_the_pane_under_the_diff_grow_to_half_the_column() {
        assert_eq!(pane_rows(39, 30, true), Some(19));
        assert_eq!(pane_rows(39, 12, true), Some(12), "past the quarter, only as far as the box needs");
        assert_eq!(pane_rows(39, 4, true), Some(4));
    }

    #[test]
    fn a_column_under_24_rows_leaves_the_pane_a_page_of_its_own() {
        assert_eq!(pane_rows(23, 5, false), None);
        assert_eq!(pane_rows(23, 5, true), None);
        assert_eq!(pane_rows(ZEN_SPLIT_FROM, 5, false), Some(5));
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
        assert_eq!(truncate("漢漢漢", 4), "漢…", "a wide character that would cross the cut stays out");
    }

    proptest::proptest! {
        #[test]
        fn a_cut_text_never_takes_more_than_its_width(text in "[a-zé漢🦀 ]{0,20}", width in 1..24usize) {
            proptest::prop_assert!(truncate(&text, width).width() <= width);
        }

        #[test]
        fn tabs_drawn_while_cutting_read_as_tabs_drawn_first(text in "[a-z漢\t ]{0,20}", width in 1..24usize) {
            proptest::prop_assert_eq!(truncate_tabs(&text, "→   ", width), truncate(&text.replace('\t', "→   "), width));
        }
    }

    #[test]
    fn ages_are_one_unit() {
        assert_eq!(short_age(Duration::from_secs(5)), "5s");
        assert_eq!(short_age(Duration::from_secs(600)), "10m");
        assert_eq!(short_age(Duration::from_secs(7200)), "2h");
        assert_eq!(short_age(Duration::from_secs(200_000)), "2d");
    }
}
