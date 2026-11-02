//! The packages of a lockfile in the diff area: the file and its counts, the changes in view, and the way to the raw diff.
use super::app::App;
use super::theme::Theme;
use super::ui::{self, draw_empty};
use crate::lockfile::{Alert, Bump, Change, ChangeKind, Summary};
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
            (count(ChangeKind::warns), "to check", theme.warn),
            (count(|k| matches!(k, ChangeKind::Updated { .. }) && !k.warns()), "updated", theme.muted),
            (count(|k| matches!(k, ChangeKind::Added { .. }) && !k.warns()), "added", theme.success),
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
        ChangeKind::SameVersion { version, .. } | ChangeKind::Added { version, .. } | ChangeKind::Removed { version } => {
            version.to_string()
        }
        ChangeKind::Updated { from, to, .. } => format!("{from} → {to}"),
    }
}

fn alert_text(alert: &Alert) -> String {
    match alert {
        Alert::ChecksumChanged => "checksum changed".to_owned(),
        Alert::SourceChanged => "source changed".to_owned(),
        Alert::InstallScript => "install script added".to_owned(),
        Alert::SourceMoved { to } => format!("source moved to {to}"),
        Alert::ChecksumLost => "checksum gone".to_owned(),
        Alert::OffRegistry { from } => format!("from {from}"),
    }
}

fn bump_word(bump: Bump) -> &'static str {
    match bump {
        Bump::Breaking => "breaking",
        Bump::Minor => "minor",
        Bump::Patch => "patch",
        Bump::Other => "",
    }
}

/// What the row says after its versions, the warning first in view.
fn note(kind: &ChangeKind) -> String {
    let lead = match kind {
        ChangeKind::SameVersion { .. } => "same version",
        ChangeKind::Updated { bump, .. } => bump_word(*bump),
        ChangeKind::Added { .. } | ChangeKind::Removed { .. } => "",
    };
    let alerts = kind.alerts().iter().map(alert_text);
    std::iter::once(lead.to_owned()).filter(|lead| !lead.is_empty()).chain(alerts).collect::<Vec<_>>().join(", ")
}

fn row<'a>(change: &Change, name_w: usize, versions_w: usize, theme: Theme) -> Line<'a> {
    let (glyph, colour) = match &change.kind {
        kind if kind.warns() => ("⚠", theme.warn),
        ChangeKind::Updated { from, to, bump, .. } => (
            if to.is_older_than(from) { "↓" } else { "↑" },
            match bump {
                Bump::Breaking => theme.danger,
                Bump::Minor => theme.accent,
                Bump::Patch | Bump::Other => theme.muted,
            },
        ),
        ChangeKind::SameVersion { .. } | ChangeKind::Added { .. } => ("+", theme.success),
        ChangeKind::Removed { .. } => ("-", theme.danger),
    };
    let style = Style::default().fg(colour);
    let name = format!("{:<name_w$}", change.name.to_string());
    let versions = format!("{:<versions_w$}", versions(&change.kind));
    Line::from(vec![
        Span::styled(format!("{glyph} "), style),
        Span::styled(format!("{name}  "), style.add_modifier(Modifier::BOLD)),
        Span::raw(format!("{versions}  ")),
        Span::styled(note(&change.kind), style),
    ])
}
