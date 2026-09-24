//! The queue pane: sections under faded rules, two lines per MR by default (one with
//! `[tui] queue = "compact"`), and one author's chained MRs folded into a stack.
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
/// The rule `│` and a space in front of an unfolded stack's MRs.
const INDENT: usize = 2;

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
    let heights: Vec<usize> = rows.iter().enumerate().map(|(i, row)| height_of(app.queue_layout, row, i == 0)).collect();
    let scroll = settle(app.queue_scroll, app.queue_selected, &heights, height);
    let mut lines: Vec<Line<'static>> = vec![];
    let mut links = vec![];
    for (i, row) in rows.iter().enumerate().skip(scroll) {
        if lines.len() >= height {
            break;
        }
        let top = lines.len();
        let (drawn, link) = row_lines(app, row, i == app.queue_selected, i == 0, inner.width as usize);
        if let Some(link) = link.filter(|link| top + link.line < height) {
            links.push(Link { x: inner.x + link.x, y: inner.y + (top + link.line) as u16, text: link.text, url: link.url });
        }
        lines.extend(drawn);
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

/// A header takes a blank line above it, except at the very top; an MR or a stack takes two
/// lines when rows are comfortable.
fn height_of(layout: QueueLayout, row: &QueueRow<'_>, first: bool) -> usize {
    match (layout, row) {
        (_, QueueRow::Section { .. }) if !first => 2,
        (QueueLayout::Comfortable, QueueRow::Mr(_) | QueueRow::Stack { .. } | QueueRow::Stacked(_)) => 2,
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

fn row_lines(app: &App, row: &QueueRow<'_>, selected: bool, first: bool, width: usize) -> (Vec<Line<'static>>, Option<RowLink>) {
    let theme = app.theme;
    match row {
        QueueRow::Section { name, count, open } => {
            let mut lines = if first { vec![] } else { vec![Line::default()] };
            lines.push(rule(theme, name, *count, *open, selected, width));
            (lines, None)
        }
        QueueRow::Author { name, count } => (vec![author_header(theme, name, *count)], None),
        QueueRow::Mr(mr) => mr_lines(app, mr, selected, 0, width),
        QueueRow::Stacked(mr) => mr_lines(app, mr, selected, INDENT, width),
        QueueRow::Stack { mrs, open, .. } => (stack_lines(app, mrs, *open, selected, width), None),
    }
}

fn mr_lines(app: &App, mr: &QueueMr, selected: bool, indent: usize, width: usize) -> (Vec<Line<'static>>, Option<RowLink>) {
    match app.queue_layout {
        QueueLayout::Comfortable => comfortable(app, mr, selected, indent, width),
        QueueLayout::Compact => compact(app, mr, selected, indent, width),
    }
}

/// `── OPEN · 28 ─────────`: a faded rule, so a section reads as a break, not as one more row.
/// A folded section shows `▸`; the cursor on it brightens the name.
fn rule(theme: Theme, name: &str, count: usize, open: bool, selected: bool, width: usize) -> Line<'static> {
    let faded = Style::default().fg(theme.faded);
    let fold = if open { "" } else { "▸ " };
    let label = format!(" {fold}{name} · {count} ");
    let lead = "──";
    let tail = width.saturating_sub(BAR_W + lead.width() + label.width());
    let label_style = if selected { Style::default().fg(theme.muted).add_modifier(Modifier::BOLD) } else { faded };
    Line::from(vec![
        bar(theme, selected),
        Span::styled(lead, faded),
        Span::styled(label, label_style),
        Span::styled("─".repeat(tail), faded),
    ])
}

/// One author's sub-header when `S` groups: quieter than a section, no rule.
fn author_header(theme: Theme, name: &str, count: usize) -> Line<'static> {
    Line::from(Span::styled(format!("{}{} · {count}", " ".repeat(BAR_W + 1), short_name(name)), Style::default().fg(theme.faded)))
}

fn bar(theme: Theme, selected: bool) -> Span<'static> {
    Span::styled(if selected { "▎ " } else { "  " }, Style::default().fg(theme.accent))
}

/// The line under an unfolded stack's row joins its MRs to it.
fn indent_span(theme: Theme, indent: usize) -> Option<Span<'static>> {
    (indent > 0).then(|| Span::styled(format!("{:<indent$}", "│"), Style::default().fg(theme.faded)))
}

/// Line one: the kind as a chip, the title, then Jev's mark, the approval mark and the badge at the right edge.
/// Line two, all faded: who, the number, the size when it fits, and the age at the right edge.
fn comfortable(app: &App, mr: &QueueMr, selected: bool, indent: usize, width: usize) -> (Vec<Line<'static>>, Option<RowLink>) {
    let theme = app.theme;
    let (kind, rest) = conventional(&mr.title);
    let chip = kind.map(|k| Span::styled(format!("{k} "), Style::default().fg(kind_colour(theme, k)).add_modifier(Modifier::BOLD)));
    let tail = right_marks(app, app.triaged().then(|| app.mark(mr)).flatten(), app.approved(mr), app.badge(mr));
    let used = BAR_W + indent + chip.as_ref().map_or(0, Span::width) + tail_w(&tail) + 1;
    let title = truncate(rest, width.saturating_sub(used));
    let pad = width.saturating_sub(used + title.width()) + 1;
    let title_style = if selected { Style::default().add_modifier(Modifier::BOLD) } else { Style::default() };
    let mut first = vec![bar(theme, selected)];
    first.extend(indent_span(theme, indent));
    first.extend(chip);
    first.extend([Span::styled(title, title_style), Span::raw(" ".repeat(pad))]);
    first.extend(tail);
    let (meta, number_at) = meta(app, mr, width.saturating_sub(BAR_W + indent));
    let mut second = vec![bar(theme, selected)];
    second.extend(indent_span(theme, indent));
    second.extend(meta);
    let link = RowLink { line: 1, x: (BAR_W + indent + number_at) as u16, text: number_of(app, mr), url: mr.web_url.clone() };
    (vec![Line::from(first), Line::from(second)], Some(link))
}

/// `romain · !1797 · +7 −5` then the age flush right, one faded colour so the eye goes from
/// title to title. The size, then the host, drop first when the pane is narrow. Also where the
/// number starts, for its link.
fn meta(app: &App, mr: &QueueMr, room: usize) -> (Vec<Span<'static>>, usize) {
    let dim = Style::default().fg(app.theme.faded);
    let who = short_name(&mr.author);
    let number = number_of(app, mr);
    let age = short_age((app.today - mr.updated_at).to_std().unwrap_or_default());
    let host = app.host_tag(mr).map(|h| format!(" · {h}")).unwrap_or_default();
    let size = format!(" · +{} −{}", mr.additions, mr.deletions);
    let started = app.viewed_count(&mr.key()).filter(|_| mr.files > 0).map(|n| format!(" · {n}/{}", mr.files)).unwrap_or_default();
    let number = format!("{number}{started}");
    let base = number.width() + 3 + 1 + age.width();
    let who = truncate(who, room.saturating_sub(base).max(1));
    let left = |extra: &str| who.width() + 3 + number.width() + extra.width();
    let host = if left(&host) + 1 + age.width() <= room { host } else { String::new() };
    let size = if left(&(host.clone() + &size)) + 1 + age.width() <= room { size } else { String::new() };
    let text = format!("{who} · {number}{host}{size}");
    let pad = room.saturating_sub(text.width() + age.width());
    let number_at = who.width() + 3;
    (vec![Span::styled(text, dim), Span::styled(format!("{}{age}", " ".repeat(pad)), dim)], number_at)
}

fn number_of(app: &App, mr: &QueueMr) -> String {
    format!("{}{}", app.hosts.kind_of(&mr.key()).sigil(), mr.number)
}

/// One line per MR: `!iid title`, the host when mixed, Jev's mark, the approval mark and the badge.
fn compact(app: &App, mr: &QueueMr, selected: bool, indent: usize, width: usize) -> (Vec<Line<'static>>, Option<RowLink>) {
    let theme = app.theme;
    let tail = right_marks(app, app.triaged().then(|| app.mark(mr)).flatten(), app.approved(mr), app.badge(mr));
    let iid = format!("{} ", number_of(app, mr));
    let tag = app.host_tag(mr).map(|t| format!("{t} "));
    let tag_w = tag.as_ref().map_or(0, |t| t.width());
    let room = width.saturating_sub(BAR_W + indent + iid.width() + tag_w + tail_w(&tail) + 1);
    let title = truncate(&mr.title, room);
    let pad = room.saturating_sub(title.width()) + 1;
    let title_style = if selected { Style::default().add_modifier(Modifier::BOLD) } else { Style::default() };
    let mut spans = vec![bar(theme, selected)];
    spans.extend(indent_span(theme, indent));
    spans.extend([
        Span::styled(iid.clone(), Style::default().fg(theme.muted)),
        Span::styled(title, title_style),
        Span::raw(" ".repeat(pad)),
    ]);
    spans.extend(tag.map(|t| Span::styled(t, Style::default().fg(theme.faded))));
    spans.extend(tail);
    let link = RowLink { line: 0, x: (BAR_W + indent) as u16, text: iid.trim_end().to_owned(), url: mr.web_url.clone() };
    (vec![Line::from(spans)], Some(link))
}

/// A folded stack as one row: `▸ feat read shared PDFs` then `romain · 7 MRs` with the newest
/// age. Unfolded, `▾`, and its MRs follow indented.
fn stack_lines(app: &App, mrs: &[&QueueMr], open: bool, selected: bool, width: usize) -> Vec<Line<'static>> {
    let theme = app.theme;
    let dim = Style::default().fg(theme.faded);
    let Some(base) = mrs.first() else { return vec![] };
    let fold = Span::styled(if open { "▾ " } else { "▸ " }, Style::default().fg(theme.muted));
    let (kind, title) = stack_title(mrs);
    let chip = kind.map(|k| Span::styled(format!("{k} "), Style::default().fg(kind_colour(theme, k)).add_modifier(Modifier::BOLD)));
    let tail = right_marks(app, None, app.stack_approved(mrs), app.stack_badge(mrs));
    let who = short_name(&base.author);
    let count = format!("{} MRs", mrs.len());
    let title_style = if selected { Style::default().add_modifier(Modifier::BOLD) } else { Style::default() };
    match app.queue_layout {
        QueueLayout::Comfortable => {
            let used = BAR_W + 2 + chip.as_ref().map_or(0, Span::width) + tail_w(&tail) + 1;
            let title = truncate(&title, width.saturating_sub(used));
            let pad = width.saturating_sub(used + title.width()) + 1;
            let mut first = vec![bar(theme, selected), fold];
            first.extend(chip);
            first.extend([Span::styled(title, title_style), Span::raw(" ".repeat(pad))]);
            first.extend(tail);
            let newest = mrs.iter().map(|mr| mr.updated_at).max().unwrap_or(base.updated_at);
            let age = short_age((app.today - newest).to_std().unwrap_or_default());
            let room = width.saturating_sub(BAR_W + 2);
            let text = format!("{who} · {count} · stack");
            let pad = room.saturating_sub(text.width() + age.width());
            let second = vec![
                bar(theme, selected),
                Span::raw("  "),
                Span::styled(text, dim),
                Span::styled(format!("{}{age}", " ".repeat(pad)), dim),
            ];
            vec![Line::from(first), Line::from(second)]
        }
        QueueLayout::Compact => {
            let head = format!("{who} · {count} · ");
            let room = width.saturating_sub(BAR_W + 2 + head.width() + tail_w(&tail) + 1);
            let title = truncate(&title, room);
            let pad = room.saturating_sub(title.width()) + 1;
            let mut spans =
                vec![bar(theme, selected), fold, Span::styled(head, dim), Span::styled(title, title_style), Span::raw(" ".repeat(pad))];
            spans.extend(tail);
            vec![Line::from(spans)]
        }
    }
}

/// What a stack is about: the words its titles share when they share two or more, else the
/// base MR's title. The kind chip shows when every MR has the same one.
fn stack_title<'m>(mrs: &[&'m QueueMr]) -> (Option<&'m str>, String) {
    let parts: Vec<(Option<&str>, &str)> = mrs.iter().map(|mr| conventional(&mr.title)).collect();
    let kind = parts.first().and_then(|(k, _)| *k).filter(|k| parts.iter().all(|(other, _)| *other == Some(*k)));
    let words: Vec<Vec<&str>> = parts.iter().map(|(_, rest)| rest.split_whitespace().collect()).collect();
    let shared = words.first().map_or(0, |first| (0..first.len()).take_while(|&i| words.iter().all(|w| w.get(i) == first.get(i))).count());
    let title = match (shared, words.first()) {
        (2.., Some(first)) => first[..shared].join(" "),
        _ => parts.first().map_or_else(String::new, |(_, rest)| (*rest).to_owned()),
    };
    (kind, title)
}

/// `romain.courtois` is `romain`: the handle up to its first `.`, `_` or `-`, the rest being a
/// surname or a company suffix the eye does not need.
pub fn short_name(author: &str) -> &str {
    match author.split(['.', '_', '-']).next() {
        Some(first) if !first.is_empty() => first,
        _ => author,
    }
}

/// Jev's mark (two cells when Jev is on), the approval mark (two cells) then the badge, flush
/// right, so titles keep one width.
fn right_marks(app: &App, mark: Option<Mark>, approved: bool, badge: Option<Badge>) -> Vec<Span<'static>> {
    let mut spans = vec![];
    if app.triaged() {
        spans.push(mark_span(app, mark));
    }
    spans.push(if approved { Span::styled("✓ ", Style::default().fg(app.theme.success)) } else { Span::raw("  ") });
    spans.push(badge.map_or_else(|| Span::raw(" "), |b| badge_span(app, b)));
    spans
}

fn tail_w(tail: &[Span<'_>]) -> usize {
    tail.iter().map(Span::width).sum()
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
    fn a_handle_shortens_to_its_first_part() {
        let cases = [
            ("romain.courtois", "romain"),
            ("cyrille_enroll", "cyrille"),
            ("adumoulin-enroll", "adumoulin"),
            ("vivien11", "vivien11"),
            ("nina", "nina"),
            ("_hidden", "_hidden"),
            (".dot", ".dot"),
            ("", ""),
        ];
        for (handle, short) in cases {
            assert_eq!(short_name(handle), short, "{handle}");
        }
    }

    fn titled(titles: &[&str]) -> Vec<QueueMr> {
        let seed: QueueMr =
            serde_json::from_value(serde_json::json!({"number": 1, "project": "a/b", "title": "", "draft": false, "web_url": "",
                "updated_at": "2026-09-01T00:00:00Z", "created_at": "2026-09-01T00:00:00Z", "source_branch": "", "target_branch": "",
                "conflicts": false, "author": "nina", "author_name": "Nina", "approved": false, "approved_by": [], "reviewers": [],
                "pipeline": null, "additions": 0, "deletions": 0, "files": 0, "unresolved": 0, "labels": [], "notes": 0}))
            .unwrap();
        titles.iter().map(|t| QueueMr { title: (*t).to_owned(), ..seed.clone() }).collect()
    }

    #[test]
    fn a_stack_is_named_by_the_words_its_titles_share_else_by_its_base() {
        let shared = titled(&["feat: read shared PDFs from the drive", "feat: read shared PDFs page by page"]);
        let refs: Vec<&QueueMr> = shared.iter().collect();
        assert_eq!(stack_title(&refs), (Some("feat"), "read shared PDFs".to_owned()));
        let apart = titled(&["feat: store the page", "fix: label PDFs"]);
        let refs: Vec<&QueueMr> = apart.iter().collect();
        assert_eq!(stack_title(&refs), (None, "store the page".to_owned()), "one shared word is not a name; mixed kinds show no chip");
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
