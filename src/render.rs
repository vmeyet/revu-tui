//! Plain-terminal output for the scriptable commands: colours only on a tty, columns aligned by width.
use chrono::{DateTime, Utc};
use owo_colors::OwoColorize;
use std::io::IsTerminal;
use unicode_width::UnicodeWidthStr;

const GAP: usize = 2;

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub color: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Style {
    #[default]
    Plain,
    Dim,
    Bold,
    Accent,
    Ok,
    Warn,
    Bad,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub text: String,
    pub style: Style,
    pub right: bool,
}

pub fn cell(text: impl Into<String>, style: Style) -> Cell {
    Cell { text: text.into(), style, right: false }
}

pub fn right(text: impl Into<String>, style: Style) -> Cell {
    Cell { text: text.into(), style, right: true }
}

impl Theme {
    pub fn detect() -> Self {
        Self { color: std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none() }
    }

    #[cfg(test)]
    pub fn plain() -> Self {
        Self { color: false }
    }

    pub fn paint(self, text: &str, style: Style) -> String {
        if !self.color {
            return text.to_owned();
        }
        match style {
            Style::Plain => text.to_owned(),
            Style::Dim => text.dimmed().to_string(),
            Style::Bold => text.bold().to_string(),
            Style::Accent => text.cyan().to_string(),
            Style::Ok => text.green().to_string(),
            Style::Warn => text.yellow().to_string(),
            Style::Bad => text.red().to_string(),
        }
    }

    /// Rows as aligned lines; every column is padded to its widest cell, the last one is not.
    pub fn table(self, rows: &[Vec<Cell>]) -> String {
        let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
        let widths: Vec<usize> =
            (0..columns).map(|c| rows.iter().filter_map(|r| r.get(c)).map(|cell| cell.text.width()).max().unwrap_or(0)).collect();
        rows.iter().map(|row| self.line(row, &widths)).collect()
    }

    fn line(self, row: &[Cell], widths: &[usize]) -> String {
        let last = row.len().saturating_sub(1);
        let mut out = String::new();
        for (i, cell) in row.iter().enumerate() {
            let pad = widths[i].saturating_sub(cell.text.width());
            let text = self.paint(&cell.text, cell.style);
            match (cell.right, i == last) {
                (true, _) => out.push_str(&format!("{}{text}", " ".repeat(pad))),
                (false, true) => out.push_str(&text),
                (false, false) => out.push_str(&format!("{text}{}", " ".repeat(pad))),
            }
            if i != last {
                out.push_str(&" ".repeat(GAP));
            }
        }
        out.trim_end().to_owned() + "\n"
    }
}

/// `just now`, `12m`, `2h`, `3d`, `5w`, then the date.
pub fn age(then: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = (now - then).num_seconds().max(0);
    match secs {
        s if s < 60 => "just now".into(),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s if s < 86_400 * 14 => format!("{}d", s / 86_400),
        s if s < 86_400 * 90 => format!("{}w", s / (86_400 * 7)),
        _ => then.format("%Y-%m-%d").to_string(),
    }
}

/// One glyph for a pipeline status, GraphQL (`SUCCESS`) or REST (`success`) spelling.
pub fn pipeline(status: Option<&str>) -> Cell {
    match status.map(str::to_ascii_lowercase).as_deref() {
        Some("success") => cell("✓", Style::Ok),
        Some("failed") => cell("✗", Style::Bad),
        Some("running" | "pending" | "created" | "waiting_for_resource" | "preparing") => cell("●", Style::Warn),
        Some("canceled" | "skipped" | "manual" | "scheduled") => cell("–", Style::Dim),
        _ => cell("", Style::Plain),
    }
}

pub fn truncate(text: &str, max: usize) -> String {
    let first_line = text.lines().next().unwrap_or("").trim();
    if first_line.width() <= max {
        return first_line.to_owned();
    }
    let mut out = String::new();
    for c in first_line.chars() {
        if out.width() + 1 >= max {
            break;
        }
        out.push(c);
    }
    out + "…"
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn columns_align_by_display_width_and_numbers_go_right() {
        let rows = vec![
            vec![cell("!42", Style::Plain), cell("ünïcode title", Style::Plain), right("+412", Style::Ok)],
            vec![cell("!7", Style::Plain), cell("short", Style::Plain), right("+3", Style::Ok)],
        ];
        assert_eq!(Theme::plain().table(&rows), "!42  ünïcode title  +412\n!7   short            +3\n");
    }

    #[test]
    fn colour_never_shifts_the_columns() {
        let rows = vec![vec![cell("a", Style::Ok), cell("b", Style::Plain)], vec![cell("ccc", Style::Plain), cell("d", Style::Plain)]];
        let coloured = Theme { color: true }.table(&rows);
        assert!(coloured.contains("\x1b[32ma\x1b[39m    b"), "{coloured:?}");
    }

    #[test]
    fn ages_read_like_a_human() {
        let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();
        let cases =
            [(30, "just now"), (60 * 12, "12m"), (3600 * 2, "2h"), (86_400 * 3, "3d"), (86_400 * 20, "2w"), (86_400 * 200, "2026-03-06")];
        for (secs, expected) in cases {
            assert_eq!(age(now - chrono::Duration::seconds(secs), now), expected);
        }
    }

    #[test]
    fn pipeline_glyphs_ignore_case() {
        assert_eq!(pipeline(Some("SUCCESS")).text, "✓");
        assert_eq!(pipeline(Some("failed")).text, "✗");
        assert_eq!(pipeline(Some("RUNNING")).text, "●");
        assert_eq!(pipeline(None).text, "");
    }

    #[test]
    fn truncate_keeps_the_first_line_and_marks_the_cut() {
        assert_eq!(truncate("short\nsecond", 10), "short");
        assert_eq!(truncate("a very long title indeed", 10), "a very lo…");
    }
}
