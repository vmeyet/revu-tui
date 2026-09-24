//! The cover page, centred over the panes: calm sections under faded headers, the description in
//! the same light markdown as notes, a cursor bar on the thread and file rows `enter` jumps to.
use super::app::{App, Brief};
use super::diff_view::pipeline_glyph;
use super::theme::Theme;
use super::thread_view::body_lines;
use super::ui::{pane, short_age, truncate};
use crate::ai::triage::Risk;
use crate::forge::ReviewState;
use chrono::{DateTime, Utc};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Padding, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

/// Share of the screen the cover takes each way, and the widest it gets: past that, lines are too long to read.
const SIZE_PCT: u16 = 90;
const MAX_WIDTH: u16 = 110;
const PAD_X: u16 = 2;
const PAD_Y: u16 = 1;

pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    let theme = app.theme;
    let today = app.today;
    let from_review = app.open.as_ref().is_some_and(|o| app.brief.as_ref().is_some_and(|b| b.key == o.key));
    let Some(brief) = app.brief.as_mut() else { return };
    let width = (area.width * SIZE_PCT / 100).clamp(area.width.min(40), MAX_WIDTH).min(area.width);
    let text_w = width.saturating_sub(2 + 2 * PAD_X);
    let (lines, target_lines) = lines(brief, theme, today, usize::from(text_w));
    let rows = wrapped_rows(&lines, text_w);
    let frame_y = 2 + 2 * PAD_Y;
    let height = (rows as u16).saturating_add(frame_y).min(area.height * SIZE_PCT / 100).max(area.height.min(10));
    let popup = Rect { x: area.x + (area.width - width) / 2, y: area.y + (area.height - height) / 2, width, height };
    let hint = if from_review {
        " j k move · enter go there · p pipeline · o browser · esc close "
    } else {
        " enter open the MR · j k scroll · o browser · esc close "
    };
    let block = pane(theme, &format!("{}{} {}", brief.sigil, brief.number, brief.title), true)
        .padding(Padding::new(PAD_X, PAD_X, PAD_Y, PAD_Y))
        .title_bottom(Line::from(Span::styled(hint, Style::default().fg(theme.faded))));
    let inner = block.inner(popup);
    let visible = usize::from(inner.height);
    if brief.follow
        && let Some(&line) = target_lines.get(brief.selected)
    {
        let at = wrapped_rows(&lines[..line], inner.width);
        brief.scroll = brief.scroll.clamp(at.saturating_sub(visible.saturating_sub(1)), at);
        brief.follow = false;
    }
    brief.scroll = brief.scroll.min(rows.saturating_sub(visible));
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false }).scroll((brief.scroll as u16, 0));
    f.render_widget(Clear, popup);
    f.render_widget(paragraph.block(block).style(Style::default().bg(theme.surface)), popup);
}

/// The cover's lines, and the line each cursor row sits on, threads then files.
fn lines<'a>(brief: &Brief, theme: Theme, today: DateTime<Utc>, width: usize) -> (Vec<Line<'a>>, Vec<usize>) {
    let mut lines = head(brief, theme, today);
    lines.push(Line::default());
    match brief.description.trim() {
        "" => lines.push(Line::from(Span::styled("no description", Style::default().fg(theme.faded)))),
        text => lines.extend(body_lines(text, theme)),
    }
    section(&mut lines, "checks", "", theme);
    lines.push(checks_line(brief, theme, width));
    section(&mut lines, "review", "", theme);
    lines.extend(review_lines(brief, theme, width));
    let mut targets = vec![];
    if let (Some(threads), Some(files)) = (&brief.threads, &brief.files) {
        let open = if threads.is_empty() { "none open".to_owned() } else { format!("{} open", threads.len()) };
        section(&mut lines, "threads", &open, theme);
        for (i, thread) in threads.iter().enumerate() {
            targets.push(lines.len());
            lines.push(thread_line(thread, brief.selected == i, theme, width));
        }
        let by = if files.iter().any(|f| f.risk.is_some()) { "by risk" } else { "by size" };
        section(&mut lines, "files", &format!("{} · {by}", files.len()), theme);
        for (i, file) in files.iter().enumerate() {
            targets.push(lines.len());
            lines.push(file_line(file, brief.selected == threads.len() + i, theme, width));
        }
    } else {
        section(&mut lines, "threads", &format!("{} open", brief.unresolved), theme);
        lines.push(Line::from(Span::styled("open the MR to see its threads and files", Style::default().fg(theme.faded))));
    }
    (lines, targets)
}

fn head<'a>(brief: &Brief, theme: Theme, today: DateTime<Utc>) -> Vec<Line<'a>> {
    let muted = Style::default().fg(theme.muted);
    let age = short_age((today - brief.updated_at).to_std().unwrap_or_default());
    let mut spans = vec![
        Span::styled(brief.author.clone(), Style::default().fg(theme.user(&brief.author)).add_modifier(Modifier::BOLD)),
        Span::styled(format!("  {} → {}", brief.source_branch, brief.target_branch), muted),
        Span::styled(format!("  {age}"), Style::default().fg(theme.faded)),
    ];
    if !brief.labels.is_empty() {
        spans.push(Span::styled(format!("  {}", brief.labels.iter().map(|l| format!("~{l}")).collect::<Vec<_>>().join(" ")), muted));
    }
    vec![Line::from(spans)]
}

/// A blank row, then the section's name in faded capitals and its count, quieter, beside it.
fn section(lines: &mut Vec<Line<'_>>, title: &str, detail: &str, theme: Theme) {
    lines.push(Line::default());
    let mut spans = vec![Span::styled(title.to_uppercase(), Style::default().fg(theme.faded).add_modifier(Modifier::BOLD))];
    if !detail.is_empty() {
        spans.push(Span::styled(format!("  {detail}"), Style::default().fg(theme.faded)));
    }
    lines.push(Line::from(spans));
}

fn checks_line<'a>(brief: &Brief, theme: Theme, width: usize) -> Line<'a> {
    let Some(status) = brief.checks.status.as_deref() else {
        return Line::from(Span::styled("no pipeline on this commit", Style::default().fg(theme.faded)));
    };
    let (glyph, colour) = pipeline_glyph(status, theme);
    let mut spans = vec![Span::styled(format!("{glyph} {}", status.to_lowercase()), Style::default().fg(colour))];
    if !brief.checks.failed.is_empty() {
        let names = truncate(&brief.checks.failed.join(", "), width.saturating_sub(status.width() + 6));
        spans.push(Span::styled(format!(" · {names}"), Style::default().fg(theme.danger)));
    }
    Line::from(spans)
}

fn review_lines<'a>(brief: &Brief, theme: Theme, width: usize) -> Vec<Line<'a>> {
    let review = &brief.review;
    let count = review.approved_by.len();
    let approvals = match review.approvals_left {
        Some(left) => format!("{count} of {} approvals", count + left as usize),
        None => format!("{count} approval{}", if count == 1 { "" } else { "s" }),
    };
    let me = match (review.i_approved, review.i_review) {
        (true, _) => ("you approved", theme.success),
        (false, true) => ("waits on your review", theme.warn),
        (false, false) => ("you are not a reviewer", theme.faded),
    };
    let first = Line::from(vec![
        Span::styled(
            approvals,
            Style::default().fg(if review.approvals_left == Some(0) && count > 0 { theme.success } else { theme.muted }),
        ),
        Span::styled("  ·  ", Style::default().fg(theme.faded)),
        Span::styled(me.0, Style::default().fg(me.1)),
    ]);
    let mut lines = vec![first];
    let reviewers: Vec<String> = review
        .reviewers
        .iter()
        .map(|(name, state)| {
            let approved = review.approved_by.contains(name) || *state == Some(ReviewState::Approved);
            match (approved, state) {
                (true, _) => format!("{name} ✓"),
                (false, Some(ReviewState::RequestedChanges)) => format!("{name} ✗ changes asked"),
                (false, Some(ReviewState::Reviewed | ReviewState::ReviewStarted)) => format!("{name} reviewed"),
                _ => name.clone(),
            }
        })
        .collect();
    if !reviewers.is_empty() {
        lines.push(Line::from(Span::styled(truncate(&reviewers.join("  ·  "), width), Style::default().fg(theme.muted))));
    }
    lines
}

fn thread_line<'a>(thread: &super::app::ThreadRow, selected: bool, theme: Theme, width: usize) -> Line<'a> {
    let head = format!("{} · {} · ", thread.place, thread.author);
    let room = width.saturating_sub(2 + head.width());
    Line::from(vec![
        bar(selected, theme),
        Span::styled(thread.place.clone(), Style::default().fg(if selected { theme.accent } else { theme.muted })),
        Span::styled(format!(" · {} · ", thread.author), Style::default().fg(theme.faded)),
        Span::raw(truncate(&thread.first_words, room)),
    ])
}

fn file_line<'a>(file: &super::app::FileRow, selected: bool, theme: Theme, width: usize) -> Line<'a> {
    let tick = if file.viewed { "✓ " } else { "  " };
    let counts = format!("+{} −{}", file.additions, file.deletions);
    let risk = file.risk.map_or(String::new(), |r| format!("  {}", risk_word(r)));
    let room = width.saturating_sub(2 + tick.width() + counts.width() + risk.width() + 2);
    let path = truncate(&file.path, room);
    let pad = room.saturating_sub(path.width()) + 2;
    let path_style = if selected { Style::default().fg(theme.accent) } else { Style::default() };
    Line::from(vec![
        bar(selected, theme),
        Span::styled(tick, Style::default().fg(theme.success)),
        Span::styled(path, path_style),
        Span::raw(" ".repeat(pad)),
        Span::styled(format!("+{}", file.additions), Style::default().fg(theme.success)),
        Span::styled(format!(" −{}", file.deletions), Style::default().fg(theme.danger)),
        Span::styled(risk, Style::default().fg(file.risk.map_or(theme.faded, |r| risk_colour(r, theme)))),
    ])
}

fn bar<'a>(selected: bool, theme: Theme) -> Span<'a> {
    Span::styled(if selected { "▎ " } else { "  " }, Style::default().fg(theme.accent))
}

fn risk_word(risk: Risk) -> &'static str {
    match risk {
        Risk::Cosmetic => "cosmetic",
        Risk::Logic => "logic",
        Risk::Data => "data",
        Risk::Security => "security",
    }
}

fn risk_colour(risk: Risk, theme: Theme) -> ratatui::style::Color {
    match risk {
        Risk::Cosmetic => theme.faded,
        Risk::Logic => theme.muted,
        Risk::Data => theme.warn,
        Risk::Security => theme.danger,
    }
}

/// Rows the wrapped text takes, close enough to stop the scroll at the last page.
fn wrapped_rows(lines: &[Line], width: u16) -> usize {
    let width = usize::from(width.max(1));
    lines.iter().map(|l| l.width().max(1).div_ceil(width)).sum()
}
