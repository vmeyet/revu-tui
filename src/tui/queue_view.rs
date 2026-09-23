//! The queue pane: sections of MRs, two lines per MR by default, one with `[tui] queue = "compact"`.
use super::app::{App, Badge, Focus, Mark, QueueRow};
use super::theme::Theme;
use super::ui::{Link, draw_empty, pane, short_age, spinner, truncate};
use crate::config::QueueLayout;
use crate::forge::QueueMr;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

const SKELETON_ROWS: usize = 3;
/// The cursor bar and the space after it, in front of every row.
const BAR_W: usize = 2;
/// The author keeps at least this many cells before the size or the host may show.
const MIN_AUTHOR: usize = 6;

/// Conventional-commit kinds shown as a chip in front of the title.
const KINDS: [&str; 12] = ["feat", "fix", "docs", "refactor", "test", "chore", "perf", "ci", "build", "style", "tech", "revert"];

pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    let theme = app.theme;
    let scope = app.scope().unwrap_or_else(|| "all".to_owned());
    let view = app.queue_view_label().map(|label| format!(" · {label}")).unwrap_or_default();
    let title = truncate(&format!("Queue · {scope}{view}"), area.width.saturating_sub(4) as usize);
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
    let rows = app.queue_rows();
    let heights: Vec<usize> = rows.iter().map(|row| height_of(app.queue_layout, row)).collect();
    let scroll = settle(app.queue_scroll, app.queue_selected, &heights, height);
    let mut lines: Vec<Line<'static>> = vec![];
    let mut links = vec![];
    for (i, row) in rows.iter().enumerate().skip(scroll) {
        if lines.len() >= height {
            break;
        }
        let top = lines.len();
        if let Some(link) = link_of(app, row).filter(|link| top + link.line < height) {
            links.push(Link { x: inner.x + link.x, y: inner.y + (top + link.line) as u16, text: link.text, url: link.url });
        }
        lines.extend(row_lines(app, row, i == app.queue_selected, inner.width as usize));
    }
    lines.truncate(height);
    drop(rows);
    app.queue_scroll = scroll;
    f.render_widget(Paragraph::new(lines), inner);
    app.links.extend(links);
}

/// Where an MR row's `!iid` sits: which of its lines, and how many cells in.
struct RowLink {
    line: usize,
    x: u16,
    text: String,
    url: String,
}

fn link_of(app: &App, row: &QueueRow<'_>) -> Option<RowLink> {
    let QueueRow::Mr(mr) = row else { return None };
    let text = format!("{}{}", app.hosts.kind_of(&mr.key()).sigil(), mr.number);
    let url = mr.web_url.clone();
    Some(match app.queue_layout {
        QueueLayout::Comfortable => RowLink { line: 1, x: BAR_W as u16, text, url },
        QueueLayout::Compact => RowLink { line: 0, x: (BAR_W + mark_w(app)) as u16, text, url },
    })
}

fn height_of(layout: QueueLayout, row: &QueueRow<'_>) -> usize {
    match (layout, row) {
        (QueueLayout::Comfortable, QueueRow::Mr(_)) => 2,
        _ => 1,
    }
}

/// The first row to draw so the selected one shows whole: the window only moves when the
/// selection leaves it, rows being one or two lines tall.
fn settle(scroll: usize, selected: usize, heights: &[usize], height: usize) -> usize {
    if heights.is_empty() || height == 0 {
        return 0;
    }
    let selected = selected.min(heights.len() - 1);
    let mut scroll = scroll.min(selected);
    while scroll < selected && heights[scroll..=selected].iter().sum::<usize>() > height {
        scroll += 1;
    }
    scroll
}

fn row_lines(app: &App, row: &QueueRow<'_>, selected: bool, width: usize) -> Vec<Line<'static>> {
    let theme = app.theme;
    match row {
        QueueRow::Section { name, count, open } => {
            let mark = if *open { "" } else { " ▸" };
            vec![header(theme, &format!("  {name}{mark}"), *count, width)]
        }
        QueueRow::Author { name, count } => vec![header(theme, &format!("    {name}"), *count, width)],
        QueueRow::Mr(mr) => match app.queue_layout {
            QueueLayout::Comfortable => comfortable(app, mr, selected, width),
            QueueLayout::Compact => vec![compact(app, mr, selected, width)],
        },
    }
}

/// A section or author header: its name faded, its count at the right edge.
fn header(theme: Theme, name: &str, count: usize, width: usize) -> Line<'static> {
    let count = count.to_string();
    let pad = width.saturating_sub(name.width() + count.width() + 2);
    Line::from(vec![
        Span::styled(name.to_owned(), Style::default().fg(theme.faded)),
        Span::styled(format!("{}{count}", " ".repeat(pad)), Style::default().fg(theme.faded)),
    ])
}

fn bar(theme: Theme, selected: bool) -> Span<'static> {
    Span::styled(if selected { "▎ " } else { "  " }, Style::default().fg(theme.accent))
}

/// Line one: the kind as a chip, the title, the badge. Line two, dim: number, author, age, size.
fn comfortable(app: &App, mr: &QueueMr, selected: bool, width: usize) -> Vec<Line<'static>> {
    let theme = app.theme;
    let (kind, rest) = conventional(&mr.title);
    let chip = kind.map(|k| Span::styled(format!("{k} "), Style::default().fg(kind_colour(theme, k)).add_modifier(Modifier::BOLD)));
    let mark = app.triaged().then(|| mark_span(app, app.mark(mr)));
    let badge = app.badge(mr).map(|b| badge_span(app, b));
    let used = BAR_W + mark_w(app) + chip.as_ref().map_or(0, Span::width) + 2;
    let title = truncate(rest, width.saturating_sub(used));
    let pad = width.saturating_sub(used + title.width()) + 1;
    let title_style = if selected { Style::default().add_modifier(Modifier::BOLD) } else { Style::default() };
    let mut first = vec![bar(theme, selected)];
    first.extend(mark);
    first.extend(chip);
    first.extend([Span::styled(title, title_style), Span::raw(" ".repeat(pad))]);
    first.extend(badge);
    let mut second = vec![bar(theme, selected)];
    second.extend(meta(app, mr, width.saturating_sub(BAR_W)));
    vec![Line::from(first), Line::from(second)]
}

/// `!1788 · loic · 2d · +120 −4`, and the host when the queue mixes several. When the pane is
/// too narrow, the host goes first, then the size, then the author shortens.
fn meta(app: &App, mr: &QueueMr, room: usize) -> Vec<Span<'static>> {
    let theme = app.theme;
    let dim = Style::default().fg(theme.faded);
    let number = format!("{}{}", app.hosts.kind_of(&mr.key()).sigil(), mr.number);
    let age = short_age((app.today - mr.updated_at).to_std().unwrap_or_default());
    let size = format!("+{} −{}", mr.additions, mr.deletions);
    let host = app.host_tag(mr).unwrap_or_default();
    let base = number.width() + " · ".width() * 2 + age.width();
    let with = |extra: &str| if extra.is_empty() { 0 } else { " · ".width() + extra.width() };
    let fits = |parts: usize| base + mr.author.width().min(MIN_AUTHOR) + parts <= room;
    let show_size = fits(with(&size));
    let show_host = show_size && fits(with(&size) + with(&host));
    let tail = with(if show_size { &size } else { "" }) + with(if show_host { &host } else { "" });
    let author = truncate(&mr.author, room.saturating_sub(base + tail).max(1));
    let mut spans = vec![Span::styled(number, Style::default().fg(theme.muted)), Span::styled(format!(" · {author} · {age}"), dim)];
    if show_size {
        spans.extend([
            Span::styled(" · ", dim),
            Span::styled(format!("+{}", mr.additions), Style::default().fg(theme.success)),
            Span::styled(format!(" −{}", mr.deletions), Style::default().fg(theme.danger)),
        ]);
    }
    if show_host && !host.is_empty() {
        spans.push(Span::styled(format!(" · {host}"), dim));
    }
    spans
}

/// One line per MR: `!iid title`, the host when mixed, the badge.
fn compact(app: &App, mr: &QueueMr, selected: bool, width: usize) -> Line<'static> {
    let theme = app.theme;
    let badge = app.badge(mr).map(|b| badge_span(app, b));
    let mark = app.triaged().then(|| mark_span(app, app.mark(mr)));
    let iid = format!("{}{} ", app.hosts.kind_of(&mr.key()).sigil(), mr.number);
    let tag = app.host_tag(mr).map(|t| format!("{t} "));
    let tag_w = tag.as_ref().map_or(0, |t| t.width());
    let room = width.saturating_sub(BAR_W + mark_w(app) + iid.width() + tag_w + 2);
    let title = truncate(&mr.title, room);
    let pad = room.saturating_sub(title.width()) + 1;
    let title_style = if selected { Style::default().add_modifier(Modifier::BOLD) } else { Style::default() };
    let mut spans = vec![bar(theme, selected)];
    spans.extend(mark);
    spans.extend([Span::styled(iid, Style::default().fg(theme.muted)), Span::styled(title, title_style), Span::raw(" ".repeat(pad))]);
    spans.extend(tag.map(|t| Span::styled(t, Style::default().fg(theme.faded))));
    spans.extend(badge);
    Line::from(spans)
}

/// `feat(notion): sync users` is `feat` and `sync users`; a draft prefix goes, since the row's
/// `D` already says it. A title in no known shape comes back whole.
pub fn conventional(title: &str) -> (Option<&str>, &str) {
    let title = ["Draft: ", "draft: ", "WIP: "].iter().find_map(|p| title.strip_prefix(p)).unwrap_or(title).trim_start();
    let Some((head, rest)) = title.split_once(':') else { return (None, title) };
    let kind = head.split('(').next().unwrap_or(head).trim_end_matches('!');
    let well_formed = head.ends_with(')') == head.contains('(') && !head.contains(' ');
    match KINDS.iter().find(|k| **k == kind) {
        Some(known) if well_formed && !rest.trim().is_empty() => (Some(known), rest.trim_start()),
        _ => (None, title),
    }
}

fn kind_colour(theme: Theme, kind: &str) -> Color {
    match kind {
        "feat" => theme.success,
        "fix" | "revert" => theme.danger,
        "docs" => theme.link,
        "refactor" | "tech" => theme.mention,
        "test" => theme.code,
        "perf" => theme.warn,
        _ => theme.muted,
    }
}

/// Jev's mark takes two cells when Jev is on, so titles stay aligned whether a row has one or not.
fn mark_w(app: &App) -> usize {
    if app.triaged() { 2 } else { 0 }
}

/// The activity dot breathes: bright one second, faded the next.
fn pulse_on(app: &App) -> bool {
    app.now.duration_since(app.started).as_secs().is_multiple_of(2)
}

fn mark_span(app: &App, mark: Option<Mark>) -> Span<'static> {
    let theme = app.theme;
    match mark {
        Some(Mark::WaitsOnMe) => Span::styled("◆ ", Style::default().fg(if pulse_on(app) { theme.warn } else { theme.faded })),
        Some(Mark::Urgent) => Span::styled("! ", Style::default().fg(theme.danger).add_modifier(Modifier::BOLD)),
        Some(Mark::Sprawling) => Span::styled("~ ", Style::default().fg(theme.muted)),
        None => Span::raw("  "),
    }
}

fn badge_span(app: &App, badge: Badge) -> Span<'static> {
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn a_conventional_title_splits_into_its_kind_and_the_rest() {
        let cases = [
            ("feat(notion): sync users", Some("feat"), "sync users"),
            ("fix: stop the leak", Some("fix"), "stop the leak"),
            ("feat!: drop v1", Some("feat"), "drop v1"),
            ("Draft: docs(agent): write it down", Some("docs"), "write it down"),
            ("tech(slack): batch pages", Some("tech"), "batch pages"),
            ("Update the README", None, "Update the README"),
            ("Draft: Update the README", None, "Update the README"),
            ("note: not a kind", None, "note: not a kind"),
            ("feat:", None, "feat:"),
            ("feat (spaced): nope", None, "feat (spaced): nope"),
        ];
        for (title, kind, rest) in cases {
            assert_eq!(conventional(title), (kind, rest), "{title}");
        }
    }

    #[test]
    fn the_window_keeps_a_two_line_row_whole() {
        let heights = [1, 2, 2, 2, 1, 2];
        assert_eq!(settle(0, 3, &heights, 8), 0, "rows 0 to 3 take 7 lines");
        assert_eq!(settle(0, 4, &heights, 8), 0, "rows 0 to 4 fill the 8 lines exactly");
        assert_eq!(settle(0, 5, &heights, 8), 2, "row 5 pushes the first two out");
        assert_eq!(settle(3, 1, &heights, 8), 1, "moving up pulls the window with it");
        assert_eq!(settle(0, 9, &heights, 4), 4, "a selection past the end settles on the last row");
        assert_eq!(settle(2, 0, &[], 5), 0);
    }
}
