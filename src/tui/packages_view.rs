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
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use vtui::text::{truncate, truncate_middle};

/// The title row and the empty row under it.
const TITLE_ROWS: u16 = 2;
/// The empty row above the way to the raw diff, and that row.
const FOOT_ROWS: u16 = 2;
/// The widest a name or versions column gets: a lockfile's author picks these texts, and they must not push the reason away.
const COLUMN_MAX: usize = 40;
/// The glyph and the gaps of a row.
const ROW_FIXED: usize = 6;

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
    let counts = counts(summary, theme);
    let path_room = (area.width as usize).saturating_sub(counts.width() + 2);
    let path_span = Span::styled(format!("{}  ", truncate_middle(path, path_room)), Style::default().add_modifier(Modifier::BOLD));
    f.render_widget(Paragraph::new(Line::from([vec![path_span], counts.spans].concat())), area);
    let foot = Rect { y: body.bottom() + 1, height: 1, ..area }.intersection(area);
    let threads = open.review.threads_on_file(path);
    f.render_widget(Paragraph::new(Line::styled(way_to_raw_diff(threads), Style::default().fg(theme.faded))), foot);
    packages.scroll = packages.scroll.min(summary.changes.len().saturating_sub(body.height as usize));
    let name_w = summary.changes.iter().map(|c| capped_width(&c.name.to_string())).max().unwrap_or(0);
    let versions_w = summary.changes.iter().map(|c| capped_width(&versions(&c.kind))).max().unwrap_or(0);
    let shown = summary.changes.iter().skip(packages.scroll).take(body.height as usize);
    let rows: Vec<Line<'_>> = shown.map(|change| row(change, name_w, versions_w, area.width as usize, theme)).collect();
    f.render_widget(Paragraph::new(rows), body);
}

/// The columns `text` takes, counted no further than [`COLUMN_MAX`].
fn capped_width(text: &str) -> usize {
    text.chars().take(COLUMN_MAX + 1).map(|c| c.width().unwrap_or(0)).sum::<usize>().min(COLUMN_MAX)
}

/// `text` cut to `width` columns and filled up to them.
fn in_column(text: &str, width: usize) -> String {
    let cut = truncate(text, width);
    let fill = width.saturating_sub(cut.width());
    format!("{cut}{}", " ".repeat(fill))
}

fn unshown_label(lines: usize) -> &'static str {
    if lines == 1 { "raw line not in rows" } else { "raw lines not in rows" }
}

fn counts<'a>(summary: &Summary, theme: Theme) -> Line<'a> {
    let unshown = summary.unshown_lines;
    if summary.changes.is_empty() {
        let said = if unshown == 0 {
            "no package rows · v reads the raw diff".to_owned()
        } else {
            format!("no package rows · {unshown} {} · v reads them", unshown_label(unshown))
        };
        return Line::styled(said, Style::default().fg(theme.muted));
    }
    let count = |of: fn(&ChangeKind) -> bool| summary.changes.iter().filter(|c| of(&c.kind)).count();
    ui::counts(
        &[
            (count(ChangeKind::warns), "to check", theme.warn),
            (count(|k| matches!(k, ChangeKind::Updated { .. }) && !k.warns()), "updated", theme.muted),
            (count(|k| matches!(k, ChangeKind::Added { .. }) && !k.warns()), "added", theme.success),
            (count(|k| matches!(k, ChangeKind::Removed { .. })), "removed", theme.danger),
            (unshown, unshown_label(unshown), theme.accent),
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
        Alert::ChecksumMissing => "no checksum".to_owned(),
        Alert::ChecksumMalformed => "checksum is not a hash".to_owned(),
        Alert::UnknownSource { from } => format!("from {from}"),
        Alert::NotSemver => "version is not semver".to_owned(),
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

/// The reason stays whole as far as it can; the name and the versions share what it leaves.
fn row<'a>(change: &Change, name_w: usize, versions_w: usize, width: usize, theme: Theme) -> Line<'a> {
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
    let note = note(&change.kind);
    let room = width.saturating_sub(ROW_FIXED);
    let note_w = note.width().min(room * 2 / 3);
    let left = room - note_w;
    let versions_w = versions_w.min(left / 2);
    let name_w = name_w.min(left - versions_w);
    Line::from(vec![
        Span::styled(format!("{glyph} "), style),
        Span::styled(format!("{}  ", in_column(&change.name.to_string(), name_w)), style.add_modifier(Modifier::BOLD)),
        Span::raw(format!("{}  ", in_column(&versions(&change.kind), versions_w))),
        Span::styled(truncate(&note, note_w), style),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_column_width_stops_at_the_cap() {
        assert_eq!(capped_width(&"a".repeat(100)), COLUMN_MAX);
    }
}
