//! The pipeline pane: the run's state and counts, then each stage and its jobs, failures first.
use super::app::{App, Focus, Open, Run};
use super::theme::Theme;
use super::ui::{DEPLOYED, Link, counts, draw_empty, settle_scroll, side_pane, spinner, truncate};
use crate::forge::checks::{Checks, Job, JobState};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    let theme = app.theme;
    let Some(open) = &app.open else { return };
    let Some(pipeline) = &open.pipeline else { return };
    let tick = spinner(app.now.duration_since(app.started));
    let title = match &pipeline.run {
        Run::Ready(checks) => format!("Pipeline · {}", word(checks.state())),
        _ => "Pipeline".to_owned(),
    };
    let block = side_pane(theme, &title, app.focus == Focus::Side, app.zen);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let checks = match &pipeline.run {
        Run::Waiting => return draw_empty(f, theme, inner, &[tick, "", "reading the pipeline"]),
        Run::Nothing => return draw_empty(f, theme, inner, &["no pipeline ran", "on this commit"]),
        Run::Failed(message) => return draw_empty(f, theme, inner, &["pipeline unreadable", message, "", "r to retry"]),
        Run::Ready(checks) => checks,
    };
    let width = inner.width as usize;
    let deployed = deployment_lines(open, width, theme);
    let rows = rows(&deployed, checks);
    let height = inner.height as usize;
    let cursor = rows.iter().position(|row| matches!(row, PaneRow::Job(index, _) if *index == pipeline.selected)).unwrap_or(0);
    let scroll = settle_scroll(0, cursor, height);
    let links = app_links(&deployed, scroll, inner);
    let shown: Vec<Line> =
        rows.iter().skip(scroll).take(height).map(|row| row_line(row, checks, pipeline.selected, width, theme, tick)).collect();
    f.render_widget(Paragraph::new(shown), inner);
    app.links.extend(links);
}

/// Each review app's address where it shows in `inner`, `scroll` rows down, so a click opens it.
fn app_links(deployed: &[(Line, Option<String>)], scroll: usize, inner: Rect) -> Vec<Link> {
    let height = usize::from(inner.height);
    deployed
        .iter()
        .enumerate()
        .filter_map(|(row, (line, url))| {
            let y = u16::try_from(row.checked_sub(scroll).filter(|y| *y < height)?).ok()?;
            let text = line.spans.first()?.content.trim_start().to_owned();
            let x = inner.x + u16::try_from(line.width() - text.width()).ok()?;
            Some(Link { x, y: inner.y + y, text, url: url.clone()? })
        })
        .collect()
}

/// The review apps above the jobs, each with its address, which a click opens; a blank row closes
/// the block. Nothing when the branch went nowhere.
fn deployment_lines<'a>(open: &Open, width: usize, theme: Theme) -> Vec<(Line<'a>, Option<String>)> {
    let Some(deployments) = open.deployments.as_deref().filter(|d| !d.is_empty()) else { return vec![] };
    let mut lines = vec![(Line::from(Span::styled(" REVIEW APPS", Style::default().fg(theme.faded))), None)];
    for deployment in deployments {
        let behind = !deployment.current;
        let name = format!(" {DEPLOYED}{}", deployment.environment);
        let note = if behind { " · older push" } else { "" };
        lines.push((
            Line::from(vec![
                Span::styled(name, if behind { Style::default().fg(theme.muted) } else { Style::default() }),
                Span::styled(note, Style::default().fg(theme.faded)),
            ]),
            None,
        ));
        let url = Span::styled(format!("   {}", truncate(&deployment.url, width.saturating_sub(4))), Style::default().fg(theme.link));
        lines.push((Line::from(url), Some(deployment.url.clone())));
    }
    lines.push((Line::default(), None));
    lines
}

/// One row of the pane, drawn only once it is in view.
enum PaneRow<'p> {
    Deployed(&'p Line<'static>),
    Summary,
    Blank,
    Stage(&'p str),
    /// A job, and its index among all the run's jobs.
    Job(usize, &'p Job),
}

/// Every row of the pane: the review apps, the counts, then each stage and its jobs.
fn rows<'p>(deployed: &'p [(Line<'static>, Option<String>)], checks: &'p Checks) -> Vec<PaneRow<'p>> {
    let mut rows: Vec<PaneRow> = deployed.iter().map(|(line, _)| PaneRow::Deployed(line)).collect();
    rows.extend([PaneRow::Summary, PaneRow::Blank]);
    let mut index = 0;
    for stage in &checks.stages {
        rows.push(PaneRow::Stage(&stage.name));
        for job in &stage.jobs {
            rows.push(PaneRow::Job(index, job));
            index += 1;
        }
        rows.push(PaneRow::Blank);
    }
    rows
}

fn row_line(row: &PaneRow, checks: &Checks, selected: usize, width: usize, theme: Theme, tick: &str) -> Line<'static> {
    match row {
        PaneRow::Deployed(line) => (*line).clone(),
        PaneRow::Summary => summary(checks, theme),
        PaneRow::Blank => Line::default(),
        PaneRow::Stage(name) => Line::from(Span::styled(format!(" {}", name.to_uppercase()), Style::default().fg(theme.faded))),
        PaneRow::Job(index, job) => job_line(job, *index == selected, width, theme, tick),
    }
}

/// `2 passed · 1 failed · 1 running`, zeros left out.
fn summary<'a>(checks: &Checks, theme: Theme) -> Line<'a> {
    let parts = [
        (JobState::Failed, "failed", theme.danger),
        (JobState::Running, "running", theme.accent),
        (JobState::Pending, "pending", theme.muted),
        (JobState::Passed, "passed", theme.success),
        (JobState::Skipped, "skipped", theme.faded),
        (JobState::Canceled, "canceled", theme.faded),
        (JobState::Manual, "manual", theme.muted),
    ];
    counts(&parts.map(|(state, label, colour)| (checks.count(state), label, colour)), theme)
}

fn job_line<'a>(job: &Job, selected: bool, width: usize, theme: Theme, tick: &str) -> Line<'a> {
    let bar = Span::styled(if selected { "▎" } else { " " }, Style::default().fg(theme.accent));
    let (glyph, colour) = glyph(job, theme, tick);
    let took = job.seconds.map(duration).unwrap_or_default();
    let room = width.saturating_sub(4 + took.width() + 1);
    let name = truncate(&job.name, room);
    let pad = room.saturating_sub(name.width()) + 1;
    let text = match job.state {
        JobState::Failed if !job.allowed_to_fail => Style::default().add_modifier(Modifier::BOLD),
        JobState::Skipped | JobState::Canceled => Style::default().fg(theme.faded),
        _ => Style::default(),
    };
    Line::from(vec![
        bar,
        Span::styled(format!(" {glyph} "), Style::default().fg(colour)),
        Span::styled(name, text),
        Span::raw(" ".repeat(pad)),
        Span::styled(took, Style::default().fg(theme.faded)),
    ])
}

/// A failure the forge lets pass shows `!` in the warning colour, never the red cross.
fn glyph<'t>(job: &Job, theme: Theme, tick: &'t str) -> (&'t str, Color) {
    match job.state {
        JobState::Passed => ("✓", theme.success),
        JobState::Failed if job.allowed_to_fail => ("!", theme.warn),
        JobState::Failed => ("✗", theme.danger),
        JobState::Running => (tick, theme.accent),
        JobState::Pending => ("○", theme.muted),
        JobState::Manual => ("▶", theme.muted),
        JobState::Canceled => ("⊘", theme.faded),
        JobState::Skipped => ("–", theme.faded),
    }
}

fn word(state: JobState) -> &'static str {
    match state {
        JobState::Passed => "passed",
        JobState::Failed => "failed",
        JobState::Running | JobState::Pending => "running",
        JobState::Canceled => "canceled",
        JobState::Manual => "manual",
        JobState::Skipped => "skipped",
    }
}

/// `7s`, `1m03s`, `1h02m`.
fn duration(seconds: u64) -> String {
    match seconds {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m{:02}s", s / 60, s % 60),
        s => format!("{}h{:02}m", s / 3600, (s % 3600) / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_at_a_glance() {
        assert_eq!(duration(7), "7s");
        assert_eq!(duration(63), "1m03s");
        assert_eq!(duration(3720), "1h02m");
    }
}
