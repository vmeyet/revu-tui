//! The cover page, centred over the panes: calm sections under faded headers, the description in
//! the same light markdown as notes, a cursor bar on the thread rows `enter` jumps to.
use super::app::{App, Brief};
use super::diff_view::pipeline_glyph;
use super::theme::Theme;
use super::thread_view::body_lines;
use super::ui::{DEPLOYED, pane, short_age, truncate};
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
/// Lines each thread takes: what it says, then where it is.
const THREAD_ROWS: usize = 2;

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
    let hint = match (from_review, brief.selected_thread()) {
        (true, Some(thread)) => {
            let keys = " · enter go there · esc close ";
            let room = usize::from(width).saturating_sub(4 + keys.width());
            format!(" {}{keys}", keep_end(&thread.place, room))
        }
        (true, None) => " j k move · p pipeline · o browser · esc close ".to_owned(),
        (false, _) => " enter open the MR · j k scroll · o browser · esc close ".to_owned(),
    };
    let block = pane(theme, &format!("{}{} {}", brief.sigil, brief.number, brief.title), true)
        .padding(Padding::new(PAD_X, PAD_X, PAD_Y, PAD_Y))
        .title_bottom(Line::from(Span::styled(hint, Style::default().fg(theme.faded))));
    let inner = block.inner(popup);
    let visible = usize::from(inner.height);
    if brief.follow
        && let Some(&line) = brief.selected.and_then(|i| target_lines.get(i))
    {
        let at = wrapped_rows(&lines[..line], inner.width);
        let end = wrapped_rows(&lines[..(line + THREAD_ROWS).min(lines.len())], inner.width);
        brief.scroll = brief.scroll.clamp(end.saturating_sub(visible), at);
        brief.follow = false;
    }
    brief.scroll = brief.scroll.min(rows.saturating_sub(visible));
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false }).scroll((brief.scroll as u16, 0));
    f.render_widget(Clear, popup);
    f.render_widget(paragraph.block(block).style(Style::default().bg(theme.surface)), popup);
}

/// The cover's lines, and the line each thread row starts on.
fn lines<'a>(brief: &Brief, theme: Theme, today: DateTime<Utc>, width: usize) -> (Vec<Line<'a>>, Vec<usize>) {
    let mut lines = head(brief, theme, today);
    lines.push(Line::default());
    match brief.description.trim() {
        "" => lines.push(Line::from(Span::styled("no description", Style::default().fg(theme.faded)))),
        text => lines.extend(body_lines(text, width, theme)),
    }
    section(&mut lines, "checks", "", theme);
    lines.push(checks_line(brief, theme, width));
    if !brief.deployments.is_empty() {
        section(&mut lines, "review apps", "", theme);
        lines.extend(brief.deployments.iter().map(|d| deployment_line(d, theme, width)));
    }
    section(&mut lines, "review", "", theme);
    lines.extend(review_lines(brief, theme, width));
    let mut targets = vec![];
    if let Some(threads) = &brief.threads {
        let open = if threads.is_empty() { "none open".to_owned() } else { format!("{} open", threads.len()) };
        section(&mut lines, "threads", &open, theme);
        for (i, thread) in threads.iter().enumerate() {
            targets.push(lines.len());
            lines.extend(thread_lines(thread, brief.selected == Some(i), theme, width));
        }
    } else {
        section(&mut lines, "threads", &format!("{} open", brief.unresolved), theme);
        lines.push(Line::from(Span::styled("open the MR to see its threads", Style::default().fg(theme.faded))));
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

/// A review app: its environment, then its address for the terminal to open.
fn deployment_line<'a>(deployment: &crate::forge::Deployment, theme: Theme, width: usize) -> Line<'a> {
    let name = format!("{DEPLOYED}{}  ", deployment.environment);
    let url = truncate(&deployment.url, width.saturating_sub(name.width()));
    Line::from(vec![Span::styled(name, Style::default().fg(theme.muted)), Span::styled(url, Style::default().fg(theme.link))])
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

/// The comment first, on the whole width; where it sits comes second, short and quiet.
fn thread_lines<'a>(thread: &super::app::ThreadRow, selected: bool, theme: Theme, width: usize) -> [Line<'a>; 2] {
    let author = format!("{} · ", thread.author);
    let room = width.saturating_sub(2 + author.width());
    let what = Line::from(vec![
        bar(selected, theme),
        Span::styled(author, Style::default().fg(if selected { theme.accent } else { theme.user(&thread.author) })),
        Span::raw(truncate(&thread.first_words, room)),
    ]);
    let replies = match thread.replies {
        0 => String::new(),
        1 => " · 1 reply".to_owned(),
        n => format!(" · {n} replies"),
    };
    let place = truncate(&format!("{}{replies}", thread.short_place), width.saturating_sub(2));
    let bar_style = Style::default().fg(theme.accent);
    let r#where = Line::from(vec![
        Span::styled(if selected { "▎ " } else { "  " }, bar_style),
        Span::styled(place, Style::default().fg(theme.faded)),
    ]);
    [what, r#where]
}

fn bar<'a>(selected: bool, theme: Theme) -> Span<'a> {
    Span::styled(if selected { "▎ " } else { "  " }, Style::default().fg(theme.accent))
}

/// `text` cut from the left to `room` columns, so a path keeps its file name and line.
fn keep_end(text: &str, room: usize) -> String {
    if text.width() <= room {
        return text.to_owned();
    }
    let mut kept = String::new();
    let mut used = 1;
    for c in text.chars().rev() {
        let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if used + w > room {
            break;
        }
        kept.insert(0, c);
        used += w;
    }
    format!("…{kept}")
}

/// Rows the wrapped text takes, close enough to stop the scroll at the last page.
fn wrapped_rows(lines: &[Line], width: u16) -> usize {
    let width = usize::from(width.max(1));
    lines.iter().map(|l| l.width().max(1).div_ceil(width)).sum()
}

#[cfg(test)]
mod tests {
    use super::keep_end;

    #[test]
    fn a_long_place_keeps_its_end() {
        assert_eq!(keep_end("src/a.rs:3", 20), "src/a.rs:3");
        assert_eq!(keep_end("apps/backend/src/sync/users.ts:10", 17), "…sync/users.ts:10");
    }
}
