//! The review pane: rows from `Review::rows()` turned into styled lines, only for the visible window.
use super::app::{App, Focus, Open};
use super::theme::Theme;
use super::ui::{draw_empty, pane, settle_scroll, short_age, spinner, truncate};
use crate::diff::{Line as DiffLine, LineKind};
use crate::review::{File, FileKind, Review, Row, Thread};
use chrono::{DateTime, Utc};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

const GUTTER_W: usize = 4;
const SURFACE_PCT: u32 = 8;
const WORD_PCT: u32 = 20;
const LINE_PCT: u32 = 60;
const TAB: &str = "→   ";
const INDENT: &str = "   ";
const MIN_BRANCH_W: usize = 12;

pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    let theme = app.theme;
    let title = match &app.open {
        Some(open) => format!("{} · {}", mr_ref(&open.review), truncate(&open.review.mr.title, area.width.saturating_sub(30) as usize)),
        None => "Review".to_owned(),
    };
    let block = pane(theme, &title, app.focus == Focus::Review);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if app.open.is_none() {
        let lines: &[&str] = match app.opening {
            Some(_) => &[spinner(app.now.duration_since(app.started)), "", "fetching the merge request"],
            None => &["open a merge request", "to read its diff here"],
        };
        return draw_empty(f, theme, inner, lines);
    }
    let today = app.today;
    let me = app.me.clone();
    let Some(open) = app.open.as_mut() else { return };
    let header = header_lines(open, theme, today, inner.width as usize);
    let body = Rect { y: inner.y + header.len() as u16, height: inner.height.saturating_sub(header.len() as u16), ..inner };
    let height = body.height as usize;
    open.scroll = settle_scroll(open.scroll, open.selected, height);
    let width = body.width as usize;
    let lines: Vec<Line> = open
        .rows
        .iter()
        .enumerate()
        .skip(open.scroll)
        .take(height)
        .map(|(i, row)| row_line(&open.review, row, i == open.selected, open.is_selected(i), width, theme, today, &me))
        .collect();
    f.render_widget(Paragraph::new(header), inner);
    f.render_widget(Paragraph::new(lines), body);
}

pub fn mr_ref(review: &Review) -> String {
    let project = review.mr.web_url.split("/-/merge_requests/").next().and_then(|u| u.splitn(4, '/').nth(3)).unwrap_or("");
    format!("{project}!{}", review.mr.iid)
}

fn header_lines<'a>(open: &Open, theme: Theme, today: DateTime<Utc>, width: usize) -> Vec<Line<'a>> {
    let mr = &open.review.mr;
    let muted = Style::default().fg(theme.muted);
    let dot = || Span::styled(" · ", Style::default().fg(theme.faded));
    let (adds, dels) = open.review.files.iter().fold((0, 0), |(a, d), f| (a + f.additions, d + f.deletions));
    let age = short_age((today - mr.updated_at).to_std().unwrap_or_default());
    let pipeline = mr.head_pipeline.as_ref().map(|p| p.status.clone()).unwrap_or_default();
    let (glyph, colour) = pipeline_glyph(&pipeline, theme);
    let branch_room = width.saturating_sub(40);
    let branches = format!("{} → {}", mr.source_branch, mr.target_branch);
    let mut first = vec![Span::raw(" "), Span::styled(mr.author.username.clone(), Style::default().fg(theme.user(&mr.author.username)))];
    if branch_room >= MIN_BRANCH_W {
        first.push(dot());
        first.push(Span::styled(truncate(&branches, branch_room), muted));
    }
    first.extend([
        dot(),
        Span::styled(age, muted),
        dot(),
        Span::styled(format!("+{adds}"), Style::default().fg(theme.success)),
        Span::raw(" "),
        Span::styled(format!("−{dels}"), Style::default().fg(theme.danger)),
        dot(),
        Span::styled(format!("{} files", open.review.files.len()), muted),
        dot(),
        Span::styled(format!("{glyph} {}", pipeline.to_lowercase()), Style::default().fg(colour)),
    ]);
    let first = Line::from(first);
    let approvals = &mr.approvals;
    let wanted = approvals.approved_by.len() as u32 + approvals.approvals_left;
    let threads = open.review.threads.len();
    let unresolved = open.review.unresolved();
    let mut second = vec![Span::raw("  ")];
    if wanted > 0 {
        second.push(Span::styled(
            format!("{} of {wanted} approvals", approvals.approved_by.len()),
            Style::default().fg(if approvals.approved { theme.success } else { theme.muted }),
        ));
    }
    if threads > 0 {
        if second.len() > 1 {
            second.push(dot());
        }
        second.push(Span::styled(format!("{threads} threads"), muted));
        if unresolved > 0 {
            second.push(Span::styled(format!(", {unresolved} unresolved"), Style::default().fg(theme.warn)));
        }
    }
    if mr.has_conflicts {
        second.push(dot());
        second.push(Span::styled("conflicts", Style::default().fg(theme.danger)));
    }
    if second.len() == 1 {
        return vec![first];
    }
    vec![first, Line::from(second)]
}

fn pipeline_glyph(status: &str, theme: Theme) -> (&'static str, ratatui::style::Color) {
    match status.to_ascii_lowercase().as_str() {
        "success" => ("✓", theme.success),
        "failed" => ("✗", theme.danger),
        "running" | "pending" | "created" | "preparing" | "waiting_for_resource" => ("⠋", theme.muted),
        "" => ("", theme.faded),
        _ => ("○", theme.muted),
    }
}

#[allow(clippy::too_many_arguments)]
fn row_line<'a>(
    review: &Review,
    row: &Row,
    selected: bool,
    in_range: bool,
    width: usize,
    theme: Theme,
    today: DateTime<Utc>,
    me: &str,
) -> Line<'a> {
    let bar = Span::styled(if selected || in_range { "▎" } else { " " }, Style::default().fg(theme.accent));
    let mut spans = vec![bar];
    let body = width.saturating_sub(1);
    match row {
        Row::Header | Row::Gap => {}
        Row::File { index, open } => spans.extend(file_spans(review, &review.files[*index], *open, body, theme)),
        Row::Hunk { file, index, open } => spans.extend(hunk_spans(&review.files[*file], *index, *open, body, theme)),
        Row::Line { file, hunk, index } => {
            let line = &review.files[*file].hunks[*hunk].lines[*index];
            spans.extend(line_spans(line, selected || in_range, body, theme));
        }
        Row::Thread { id } => {
            if let Some(thread) = review.thread(id) {
                spans.extend(thread_spans(thread, body, theme, today, me));
            }
        }
        Row::Draft { index } => spans.extend(draft_spans(&review.drafts[*index], body, theme)),
        Row::Outdated { file } => {
            let count = review.outdated(&review.files[*file].new_path).len();
            spans.push(Span::styled(format!("{INDENT}outdated · {count} thread{}", plural(count)), Style::default().fg(theme.faded)));
        }
    }
    Line::from(spans)
}

fn file_spans<'a>(review: &Review, file: &File, open: bool, width: usize, theme: Theme) -> Vec<Span<'a>> {
    let mark = if open { "▾ " } else { "▸ " };
    let (dir, base) = match file.new_path.rsplit_once('/') {
        Some((dir, base)) => (format!("{dir}/"), base.to_owned()),
        None => (String::new(), file.new_path.clone()),
    };
    let counts = format!("+{} −{}", file.additions, file.deletions);
    let threads =
        review.threads.iter().filter(|t| t.anchor.as_ref().is_some_and(|a| a.path == file.new_path || a.path == file.old_path)).count();
    let anchors = if threads > 0 { format!("  ◆{threads}") } else { String::new() };
    let state = file_state(file, review, open);
    let tail_w = counts.width() + anchors.width() + state.as_ref().map(|s| s.width() + 2).unwrap_or(0);
    let name_room = width.saturating_sub(mark.width() + tail_w + 2);
    let name = truncate(&format!("{dir}{base}"), name_room);
    let (dir, base) = match name.rsplit_once('/') {
        Some((d, b)) => (format!("{d}/"), b.to_owned()),
        None => (String::new(), name),
    };
    let pad = name_room.saturating_sub(dir.width() + base.width()) + 2;
    let mut spans = vec![
        Span::styled(mark, Style::default().fg(theme.faded)),
        Span::styled(dir, Style::default().fg(theme.muted)),
        Span::styled(base, Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" ".repeat(pad)),
        Span::styled(format!("+{}", file.additions), Style::default().fg(theme.success)),
        Span::raw(" "),
        Span::styled(format!("−{}", file.deletions), Style::default().fg(theme.danger)),
        Span::styled(anchors, Style::default().fg(theme.warn)),
    ];
    if let Some(state) = state {
        spans.push(Span::styled(format!("  {state}"), Style::default().fg(theme.faded)));
    }
    spans
}

fn file_state(file: &File, review: &Review, open: bool) -> Option<String> {
    if review.viewed.contains(&file.new_path) {
        return Some("viewed".into());
    }
    if file.binary {
        return Some("binary".into());
    }
    if file.too_large {
        return Some("too large".into());
    }
    match file.kind {
        FileKind::Renamed => Some(format!("renamed from {}", file.old_path)),
        FileKind::Deleted => Some("deleted".into()),
        FileKind::Added => Some("added".into()),
        FileKind::Mode => Some("mode changed".into()),
        FileKind::Modified if !open => Some("folded".into()),
        FileKind::Modified => None,
    }
}

fn hunk_spans<'a>(file: &File, index: usize, open: bool, width: usize, theme: Theme) -> Vec<Span<'a>> {
    let hunk = &file.hunks[index];
    let mark = if open { "▾ " } else { "▸ " };
    let (range, context) = match hunk.header.find(" @@") {
        Some(at) => (hunk.header[..at + 3].to_owned(), hunk.header[at + 3..].trim().to_owned()),
        None => (hunk.header.clone(), String::new()),
    };
    let count = if open { String::new() } else { format!("  ({} lines)", hunk.lines.len()) };
    let room = width.saturating_sub(INDENT.len() + mark.len() + range.width() + count.width() + 1);
    vec![
        Span::styled(format!("{INDENT}{mark}"), Style::default().fg(theme.faded)),
        Span::styled(format!("{range} "), Style::default().fg(theme.muted)),
        Span::raw(truncate(&context, room)),
        Span::styled(count, Style::default().fg(theme.faded)),
    ]
}

fn line_spans<'a>(line: &DiffLine, selected: bool, width: usize, theme: Theme) -> Vec<Span<'a>> {
    let gutter_colour = if selected { theme.muted } else { theme.faded };
    let number = |n: Option<u32>| n.map(|n| format!("{n:>GUTTER_W$}")).unwrap_or_else(|| " ".repeat(GUTTER_W));
    let (sign, colour) = match line.kind {
        LineKind::Added => ("+", Some(theme.success)),
        LineKind::Removed => ("-", Some(theme.danger)),
        LineKind::Context => (" ", None),
    };
    let surface = colour.map(|c| theme.tint(c, SURFACE_PCT));
    let base = match colour {
        Some(c) => Style::default().fg(mix_fg(theme, c)).bg(surface.unwrap_or(theme.base)),
        None => Style::default(),
    };
    let mut spans = vec![
        Span::styled(format!("{} {} ", number(line.old), number(line.new)), Style::default().fg(gutter_colour)),
        Span::styled(sign.to_owned(), colour.map(|c| base.fg(c)).unwrap_or(base)),
    ];
    let room = width.saturating_sub(GUTTER_W * 2 + 3);
    spans.extend(text_spans(line, room, base, colour.map(|c| base.fg(c).bg(theme.tint(c, WORD_PCT))), theme));
    spans
}

fn mix_fg(theme: Theme, colour: ratatui::style::Color) -> ratatui::style::Color {
    theme.tint(colour, LINE_PCT)
}

/// The text with its changed words emphasised, tabs made visible, trailing spaces marked, cut to `room`.
fn text_spans<'a>(line: &DiffLine, room: usize, base: Style, word: Option<Style>, theme: Theme) -> Vec<Span<'a>> {
    let changed = line.kind != LineKind::Context;
    let trimmed = line.text.trim_end_matches([' ', '\t']);
    let trailing = line.text.len() - trimmed.len();
    let mut spans = vec![];
    let mut used = 0;
    let mut cursor = 0;
    let mut push = |text: &str, style: Style, spans: &mut Vec<Span<'a>>| {
        let shown = text.replace('\t', TAB);
        if used >= room {
            return;
        }
        let cut = truncate(&shown, room - used);
        used += cut.width();
        spans.push(Span::styled(cut, style));
    };
    for range in &line.words {
        if range.start > cursor {
            push(&trimmed[cursor..range.start.min(trimmed.len())], base, &mut spans);
        }
        let end = range.end.min(trimmed.len());
        if range.start < end {
            push(&trimmed[range.start..end], word.unwrap_or(base), &mut spans);
        }
        cursor = end.max(cursor);
    }
    if cursor < trimmed.len() {
        push(&trimmed[cursor..], base, &mut spans);
    }
    if changed && trailing > 0 {
        push(&"·".repeat(trailing), base.fg(theme.warn), &mut spans);
    }
    spans
}

fn thread_spans<'a>(thread: &Thread, width: usize, theme: Theme, today: DateTime<Utc>, me: &str) -> Vec<Span<'a>> {
    let first = thread.first();
    let author = if first.author.username == me { "you".to_owned() } else { first.author.username.clone() };
    let replies = thread.notes.len() - 1;
    let age = short_age((today - first.created_at).to_std().unwrap_or_default());
    let (glyph, colour) = if thread.resolved { ("✓", theme.faded) } else { ("◆", theme.warn) };
    let tail = match replies {
        0 => format!(" · {age}"),
        n => format!(" · {n} repl{} · {age}", if n == 1 { "y" } else { "ies" }),
    };
    let head = format!("{INDENT}{glyph} {author} · ");
    let room = width.saturating_sub(head.width() + tail.width());
    let body = truncate(first.body.lines().next().unwrap_or(""), room);
    let text = if thread.resolved { Style::default().fg(theme.faded) } else { Style::default() };
    vec![
        Span::styled(format!("{INDENT}{glyph} "), Style::default().fg(colour)),
        Span::styled(format!("{author} · "), Style::default().fg(if thread.resolved { theme.faded } else { theme.user(&author) })),
        Span::styled(body, text),
        Span::styled(tail, Style::default().fg(theme.faded)),
    ]
}

fn draft_spans<'a>(draft: &crate::review::Draft, width: usize, theme: Theme) -> Vec<Span<'a>> {
    let head = format!("{INDENT}◇ you · ");
    let tail = if draft.id.is_none() { " · unsaved" } else { " · draft" };
    let room = width.saturating_sub(head.width() + tail.width());
    let first = truncate(draft.body.lines().next().unwrap_or_default(), room);
    vec![
        Span::styled(head, Style::default().fg(theme.warn)),
        Span::raw(first),
        Span::styled(tail, Style::default().fg(if draft.id.is_none() { theme.danger } else { theme.faded })),
    ]
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff;

    fn spans_text(spans: &[Span]) -> String {
        spans.iter().map(|s| s.content.to_string()).collect()
    }

    #[test]
    fn line_shows_both_gutters_the_sign_and_visible_tabs() {
        let hunk = &diff::parse("@@ -1,2 +1,2 @@\n \tkeep  \n-\told\n+\tnew  \n")[0];
        let context = spans_text(&line_spans(&hunk.lines[0], false, 60, Theme::default()));
        assert_eq!(context, "   1    1  →   keep");
        let added = spans_text(&line_spans(&hunk.lines[2], false, 60, Theme::default()));
        assert_eq!(added, "        2 +→   new··", "trailing spaces are marked on changed lines");
    }

    #[test]
    fn long_lines_are_cut_to_the_pane() {
        let hunk = &diff::parse(&format!("@@ -1 +1 @@\n+{}\n", "x".repeat(100)))[0];
        let text = spans_text(&line_spans(&hunk.lines[0], false, 30, Theme::default()));
        assert_eq!(text.width(), 30);
        assert!(text.ends_with('…'));
    }

    #[test]
    fn hunk_rows_split_range_and_context_and_count_when_folded() {
        let file = File::from_diff(&crate::api::DiffFile {
            diff: "@@ -12,4 +12,5 @@ pub async fn charge\n a\n".into(),
            new_path: "a.rs".into(),
            old_path: "a.rs".into(),
            ..Default::default()
        });
        let open = spans_text(&hunk_spans(&file, 0, true, 80, Theme::default()));
        assert_eq!(open, "   ▾ @@ -12,4 +12,5 @@ pub async fn charge");
        let closed = spans_text(&hunk_spans(&file, 0, false, 80, Theme::default()));
        assert!(closed.ends_with("(1 lines)"), "{closed}");
    }
}
