//! The packages of a lockfile in the diff area: the file and its counts, the changes in view, and the way to the raw diff.
use super::app::App;
use super::theme::Theme;
use super::ui::{self, draw_empty};
use crate::lockfile::{Bump, Change, ChangeKind, Drift, Summary};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

/// The title row and the empty row under it.
const TITLE_ROWS: u16 = 2;
/// The empty row above the way to the raw diff, and that row.
const FOOT_ROWS: u16 = 2;

pub(crate) fn draw(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let theme = app.theme;
    let spinner = app.spinner();
    let Some(open) = app.open.as_mut() else { return };
    let Some(packages) = open.packages.as_mut() else { return };
    let body = Rect { y: area.y + TITLE_ROWS, height: area.height.saturating_sub(TITLE_ROWS + FOOT_ROWS), ..area };
    let path = packages.sides.path();
    let Some(summary) = &packages.summary else {
        f.render_widget(Paragraph::new(Line::styled(path.to_owned(), Style::default().fg(theme.muted))), area);
        return draw_empty(f, theme, body, &[spinner, "", "reading the file at both commits"]);
    };
    let title =
        [vec![Span::styled(format!("{path}  "), Style::default().add_modifier(Modifier::BOLD))], counts(summary, theme).spans].concat();
    f.render_widget(Paragraph::new(Line::from(title)), area);
    let foot = Rect { y: body.bottom() + 1, height: 1, ..area }.intersection(area);
    let threads = open.review.threads_on_file(path);
    f.render_widget(Paragraph::new(Line::styled(way_to_raw_diff(threads), Style::default().fg(theme.faded))), foot);
    packages.scroll = packages.scroll.min(summary.changes.len().saturating_sub(body.height as usize));
    let name_w = summary.changes.iter().map(|c| c.name.to_string().width()).max().unwrap_or(0);
    let versions_w = summary.changes.iter().map(|c| versions(&c.kind).width()).max().unwrap_or(0);
    let shown = summary.changes.iter().skip(packages.scroll).take(body.height as usize);
    let rows: Vec<Line<'_>> = shown.map(|change| row(change, name_w, versions_w, theme)).collect();
    f.render_widget(Paragraph::new(rows), body);
}

fn counts<'a>(summary: &Summary, theme: Theme) -> Line<'a> {
    if summary.changes.is_empty() {
        return Line::styled("no package changed", Style::default().fg(theme.muted));
    }
    let count = |of: fn(&ChangeKind) -> bool| summary.changes.iter().filter(|c| of(&c.kind)).count();
    ui::counts(
        &[
            (count(|k| matches!(k, ChangeKind::Warning { .. })), "changed in place", theme.warn),
            (count(|k| matches!(k, ChangeKind::Updated { .. })), "updated", theme.muted),
            (count(|k| matches!(k, ChangeKind::Added { .. })), "added", theme.success),
            (count(|k| matches!(k, ChangeKind::Removed { .. })), "removed", theme.danger),
        ],
        theme,
    )
}

fn way_to_raw_diff(threads: usize) -> String {
    match threads {
        0 => "v: raw diff".to_owned(),
        1 => "1 thread on this file · v: raw diff to read it".to_owned(),
        n => format!("{n} threads on this file · v: raw diff to read them"),
    }
}

fn versions(kind: &ChangeKind) -> String {
    match kind {
        ChangeKind::Warning { version, .. } | ChangeKind::Added { version } | ChangeKind::Removed { version } => version.to_string(),
        ChangeKind::Updated { from, to, .. } => format!("{from} → {to}"),
    }
}

fn row<'a>(change: &Change, name_w: usize, versions_w: usize, theme: Theme) -> Line<'a> {
    let (glyph, colour, note) = match &change.kind {
        ChangeKind::Warning { drift: Drift::Checksum, .. } => ("⚠", theme.warn, "same version, checksum changed"),
        ChangeKind::Warning { drift: Drift::Source, .. } => ("⚠", theme.warn, "same version, source changed"),
        ChangeKind::Warning { drift: Drift::ChecksumAndSource, .. } => ("⚠", theme.warn, "same version, checksum and source changed"),
        ChangeKind::Updated { from, to, bump } => {
            let glyph = if to.is_older_than(from) { "↓" } else { "↑" };
            match bump {
                Bump::Breaking => (glyph, theme.danger, "breaking"),
                Bump::Minor => (glyph, theme.accent, "minor"),
                Bump::Patch => (glyph, theme.muted, "patch"),
                Bump::Other => (glyph, theme.muted, ""),
            }
        }
        ChangeKind::Added { .. } => ("+", theme.success, ""),
        ChangeKind::Removed { .. } => ("-", theme.danger, ""),
    };
    let style = Style::default().fg(colour);
    let name = format!("{:<name_w$}", change.name.to_string());
    let versions = format!("{:<versions_w$}", versions(&change.kind));
    Line::from(vec![
        Span::styled(format!("{glyph} "), style),
        Span::styled(format!("{name}  "), style.add_modifier(Modifier::BOLD)),
        Span::raw(format!("{versions}  ")),
        Span::styled(note, style),
    ])
}
