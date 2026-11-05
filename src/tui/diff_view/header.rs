//! The review header: the MR's title link, its author, size, pipeline, approvals, progress and review app.
use crate::forge::{Deployment, Kind, Mr, PipelineStatus};
use crate::review::{Review, Side};
use crate::tui::app::{Open, progress_bar};
use crate::tui::theme::Theme;
use crate::tui::ui::{DEPLOYED, Link, short_age};
use chrono::{DateTime, Utc};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;
use vtui::text::truncate;

/// Cells of the viewed-files bar in the review header.
const PROGRESS_W: usize = 10;
/// Room for a review app's environment name in the header before it is cut.
const ENVIRONMENT_W: usize = 32;
const MIN_BRANCH_W: usize = 12;

/// The pipeline word ends the header's first row; it links to the run on the forge.
pub(super) fn pipeline_link(review: &Review, header: &[Line<'_>], area: Rect) -> Option<Link> {
    let url = review.mr.pipeline.as_ref()?.web_url.clone()?;
    let spans = &header.first()?.spans;
    let (last, before) = spans.split_last()?;
    let x = before.iter().map(Span::width).sum::<usize>();
    let x = area.x + u16::try_from(x).ok()?;
    (x + u16::try_from(last.width()).ok()? <= area.right()).then(|| Link { x, y: area.y, text: last.content.to_string(), url })
}

/// The `group/project!42` at the start of the pane title, one cell past the border and its space.
pub(super) fn title_link(review: &Review, kind: Kind, area: Rect) -> Link {
    Link { x: area.x + 2, y: area.y, text: mr_ref(review, kind), url: review.mr.web_url.clone() }
}

/// `group/project!42` on GitLab, `owner/repo#42` on GitHub.
pub(super) fn mr_ref(review: &Review, kind: Kind) -> String {
    format!("{}{}{}", review.mr.project, kind.sigil(), review.mr.number)
}

pub(super) fn header_lines<'a>(open: &Open, theme: Theme, today: DateTime<Utc>, width: usize) -> Vec<Line<'a>> {
    let mr = &open.review.mr;
    let muted = Style::default().fg(theme.muted);
    let dot = || Span::styled(" · ", Style::default().fg(theme.faded));
    let (adds, dels) = line_totals(open);
    let age = short_age((today - mr.updated_at).to_std().unwrap_or_default());
    let status = mr.pipeline.as_ref().map(|p| p.status);
    let (glyph, colour) = status.map_or(("", theme.faded), |s| pipeline_glyph(s, theme));
    let badge = peek_badge(open.review.peek, theme);
    let branch_room = width.saturating_sub(40 + badge.iter().map(Span::width).sum::<usize>());
    let branches = format!("{} → {}", mr.source_branch, mr.target_branch);
    let mut first = vec![Span::raw(" ")];
    first.extend(badge);
    first.push(Span::styled(mr.author.username.clone(), Style::default().fg(theme.user(&mr.author.username))));
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
        Span::styled(format!("{glyph} {}", status.map_or("", PipelineStatus::word)), Style::default().fg(colour)),
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
    if mr.conflicts {
        second.push(dot());
        second.push(Span::styled("conflicts", Style::default().fg(theme.danger)));
    }
    let progress = viewed_spans(open, theme);
    if !progress.is_empty() {
        if second.len() > 1 {
            second.push(dot());
        }
        second.extend(progress);
    }
    if let Some(deployment) = open.deployments.as_deref().and_then(Deployment::to_try) {
        if second.len() > 1 {
            second.push(dot());
        }
        second.extend(deployment_spans(open, deployment, theme));
    }
    if second.len() == 1 {
        return vec![first];
    }
    vec![first, Line::from(second)]
}

/// Lines added and removed, in every file.
fn line_totals(open: &Open) -> (usize, usize) {
    open.review.files.iter().fold((0, 0), |(a, d), f| (a + f.additions, d + f.deletions))
}

/// How far the reading went, once a file was marked viewed.
fn viewed_spans<'a>(open: &Open, theme: Theme) -> Vec<Span<'a>> {
    let progress = open.review.progress();
    if progress.viewed == 0 {
        return vec![];
    }
    let (done, left) = progress_bar(progress.viewed, progress.files, PROGRESS_W);
    let folded = if progress.folded > 0 { format!(" · {} folded", progress.folded) } else { String::new() };
    vec![
        Span::styled(format!("viewed {}/{}{folded} ", progress.viewed, progress.files), Style::default().fg(theme.muted)),
        Span::styled(done, Style::default().fg(if progress.done() { theme.success } else { theme.accent })),
        Span::styled(left, Style::default().fg(theme.faded)),
    ]
}

/// A worktree's two rows: its branches and size, then the work it leaves out and how far the reading went.
pub(super) fn local_header<'a>(open: &Open, theme: Theme, today: DateTime<Utc>, width: usize) -> Vec<Line<'a>> {
    let mr = &open.review.mr;
    let muted = Style::default().fg(theme.muted);
    let dot = || Span::styled(" · ", Style::default().fg(theme.faded));
    let (adds, dels) = line_totals(open);
    let age = short_age((today - mr.updated_at).to_std().unwrap_or_default());
    let first = Line::from(vec![
        Span::raw(" "),
        Span::styled(truncate(&format!("{} → {}", mr.source_branch, mr.target_branch), width.saturating_sub(30)), muted),
        dot(),
        Span::styled(age, muted),
        dot(),
        Span::styled(format!("+{adds}"), Style::default().fg(theme.success)),
        Span::raw(" "),
        Span::styled(format!("−{dels}"), Style::default().fg(theme.danger)),
        dot(),
        Span::styled(format!("{} files", open.review.files.len()), muted),
    ]);
    let hidden = hidden_work(mr.uncommitted).map(|text| Span::styled(text, Style::default().fg(theme.warn)));
    let progress = viewed_spans(open, theme);
    let between = (hidden.is_some() && !progress.is_empty()).then(dot);
    let second: Vec<Span<'a>> = hidden.into_iter().chain(between).chain(progress).collect();
    if second.is_empty() {
        return vec![first];
    }
    vec![first, Line::from([vec![Span::raw("  ")], second].concat())]
}

/// Zen's one line on a worktree: which, against what, its size and the work it leaves out.
pub(super) fn local_zen_header<'a>(open: &Open, theme: Theme, width: usize) -> Line<'a> {
    let mr = &open.review.mr;
    let (adds, dels) = line_totals(open);
    let hidden = hidden_work(mr.uncommitted).map(|text| format!(" · {text}"));
    let hidden_w = hidden.as_ref().map_or(0, |text| text.width());
    let text = format!("{} · {} → {} · +{adds} −{dels}", mr.title, mr.source_branch, mr.target_branch);
    let shown = Span::styled(truncate(&text, width.saturating_sub(hidden_w)), Style::default().fg(theme.faded));
    Line::from([vec![shown], hidden.map(|text| Span::styled(text, Style::default().fg(theme.warn))).into_iter().collect()].concat())
}

fn hidden_work(uncommitted: u32) -> Option<String> {
    (uncommitted > 0).then(|| format!("● {uncommitted} uncommitted, not shown · I shows"))
}

/// What a worktree's empty diff says: why there can be none, or where the work is; commits past the base that change
/// no file are still commits.
pub(super) fn nothing_ahead(mr: &Mr) -> String {
    let ahead = if mr.refs.base == mr.refs.head { "no commits ahead" } else { "no changes ahead" };
    match (&mr.no_diff, mr.uncommitted) {
        (Some(reason), _) => format!("no diff: {reason}"),
        (None, 0) => ahead.to_owned(),
        (None, hidden) => format!("{ahead} · {hidden} uncommitted · I shows them"),
    }
}

/// The review app to try, `+n` for the others the pipeline pane lists, and a word when it runs an older push.
fn deployment_spans<'a>(open: &Open, deployment: &Deployment, theme: Theme) -> Vec<Span<'a>> {
    let behind = !deployment.current;
    let colour = if behind { theme.muted } else { theme.link };
    let mut spans =
        vec![Span::styled(format!("{DEPLOYED}{}", truncate(&deployment.environment, ENVIRONMENT_W)), Style::default().fg(colour))];
    let others = open.deployments.as_ref().map_or(0, |all| all.len().saturating_sub(1));
    if others > 0 {
        spans.push(Span::styled(format!(" +{others}"), Style::default().fg(theme.muted)));
    }
    if behind {
        spans.push(Span::styled(" · older push", Style::default().fg(theme.faded)));
    }
    spans
}

/// The review app on the header's second row links to where it runs.
pub(super) fn deployment_link(open: &Open, header: &[Line<'_>], area: Rect) -> Option<Link> {
    let url = Deployment::to_try(open.deployments.as_deref()?)?.url.clone();
    let spans = &header.get(1)?.spans;
    let at = spans.iter().position(|s| s.content.starts_with(DEPLOYED))?;
    let x = area.x + u16::try_from(spans[..at].iter().map(Span::width).sum::<usize>()).ok()?;
    let text = spans[at].content.to_string();
    (x + u16::try_from(text.width()).ok()? <= area.right()).then(|| Link { x, y: area.y + 1, text, url })
}

/// Zen's one line on top: which MR, whose, its size, its pipeline and its unresolved threads, all quiet.
pub(super) fn zen_header<'a>(open: &Open, sigil: char, theme: Theme, width: usize) -> Line<'a> {
    let mr = &open.review.mr;
    let (adds, dels) = line_totals(open);
    let pipeline = mr.pipeline.as_ref().map(|p| format!(" {}", pipeline_glyph(p.status, theme).0)).unwrap_or_default();
    let unresolved = open.review.unresolved();
    let threads = if unresolved > 0 { format!(" · ◆{unresolved}") } else { String::new() };
    let tail = format!(" · {} · +{adds} −{dels}{pipeline}{threads}", mr.author.username);
    let head = format!("{sigil}{} ", mr.number);
    let badge = peek_badge(open.review.peek, theme);
    let room = width.saturating_sub(badge.iter().map(Span::width).sum::<usize>() + head.width() + tail.width());
    let title = truncate(&mr.title, room);
    Line::from([badge, vec![Span::styled(format!("{head}{title}{tail}"), Style::default().fg(theme.faded))]].concat())
}

/// The side a peek reads, loud at the head of the header, then a gap; nothing while the diff shows.
fn peek_badge<'a>(peek: Option<Side>, theme: Theme) -> Vec<Span<'a>> {
    let Some(side) = peek else { return vec![] };
    let (word, ground) = match side {
        Side::Old => (" BEFORE ", theme.danger),
        Side::New => (" AFTER ", theme.success),
    };
    vec![Span::styled(word, Style::default().fg(theme.base).bg(ground).add_modifier(Modifier::BOLD)), Span::raw(" ")]
}

pub(in crate::tui) fn pipeline_glyph(status: PipelineStatus, theme: Theme) -> (&'static str, Color) {
    match status {
        PipelineStatus::Success => ("✓", theme.success),
        PipelineStatus::Failed => ("✗", theme.danger),
        running if running.is_running() => ("⠋", theme.muted),
        _ => ("○", theme.muted),
    }
}
