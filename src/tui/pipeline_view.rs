//! The pipeline pane: the run's state and counts, then each stage and its jobs, failures first.
use super::app::{App, Focus, Run};
use super::theme::Theme;
use super::ui::{draw_empty, settle_scroll, side_pane, spinner, truncate};
use crate::forge::checks::{Checks, Job, JobState};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
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
    let lines = lines(checks, pipeline.selected, inner.width as usize, theme, tick);
    let height = inner.height as usize;
    let cursor = lines.iter().position(|(job, _)| *job == Some(pipeline.selected)).unwrap_or(0);
    let scroll = settle_scroll(0, cursor, height);
    let shown: Vec<Line> = lines.into_iter().skip(scroll).take(height).map(|(_, line)| line).collect();
    f.render_widget(Paragraph::new(shown), inner);
}

/// Every row of the pane, each with the index of the job it shows, if any.
fn lines<'a>(checks: &Checks, selected: usize, width: usize, theme: Theme, tick: &str) -> Vec<(Option<usize>, Line<'a>)> {
    let mut lines = vec![(None, summary(checks, theme)), (None, Line::default())];
    let mut index = 0;
    for stage in &checks.stages {
        lines.push((None, Line::from(Span::styled(format!(" {}", stage.name.to_uppercase()), Style::default().fg(theme.faded)))));
        for job in &stage.jobs {
            lines.push((Some(index), job_line(job, index == selected, width, theme, tick)));
            index += 1;
        }
        lines.push((None, Line::default()));
    }
    lines
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
    let mut spans = vec![Span::raw(" ")];
    for (state, label, colour) in parts {
        let count = checks.count(state);
        if count == 0 {
            continue;
        }
        if spans.len() > 1 {
            spans.push(Span::styled(" · ", Style::default().fg(theme.faded)));
        }
        spans.push(Span::styled(format!("{count} {label}"), Style::default().fg(colour)));
    }
    Line::from(spans)
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
