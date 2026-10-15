//! The `:news` page, centred over the panes: revu's own changes version by version, the ones new to the reader marked.
use super::app::{App, WhatsNew};
use super::theme::Theme;
use super::thread_view::{inline, wrap};
use super::ui::pane;
use crate::changelog::{Entry, Section};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Padding, Paragraph};

/// Share of the screen the page takes each way, and the widest it gets: past that, lines are too long to read.
const SIZE_PCT: u16 = 90;
const MAX_WIDTH: u16 = 100;
const PAD_X: u16 = 2;
const PAD_Y: u16 = 1;
const BULLET: &str = "• ";
const UNDER_BULLET: &str = "  ";
const NEW_TO_YOU: &str = "  ● new for you";

pub(crate) fn draw(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let theme = app.theme;
    let Some(scroll) = app.news_page else { return };
    let width = (area.width * SIZE_PCT / 100).clamp(area.width.min(40), MAX_WIDTH).min(area.width);
    let lines = lines(&app.whats_new, theme, usize::from(width.saturating_sub(2 + 2 * PAD_X)));
    let popup = centred(area, width, lines.len());
    let block = pane(theme, &format!("what's new · revu {}", app.whats_new.changelog.running), true)
        .padding(Padding::new(PAD_X, PAD_X, PAD_Y, PAD_Y))
        .title_bottom(Line::from(Span::styled(" j k scroll · esc close ", Style::default().fg(theme.faded))));
    let visible = usize::from(block.inner(popup).height);
    let scroll = scroll.min(lines.len().saturating_sub(visible));
    let shown: Vec<Line<'_>> = lines.into_iter().skip(scroll).take(visible).collect();
    f.render_widget(Clear, popup);
    f.render_widget(Paragraph::new(shown).block(block).style(Style::default().bg(theme.surface)), popup);
    app.news_page = Some(scroll);
}

/// The page in the middle of `area`, as tall as its `rows` need and the screen allows.
fn centred(area: Rect, width: u16, rows: usize) -> Rect {
    let rows = u16::try_from(rows).unwrap_or(u16::MAX);
    let height = rows.saturating_add(2 + 2 * PAD_Y).min(area.height * SIZE_PCT / 100).max(area.height.min(10));
    Rect { x: area.x + (area.width - width) / 2, y: area.y + (area.height - height) / 2, width, height }
}

/// Every section, a blank row between two.
fn lines(whats_new: &WhatsNew, theme: Theme, width: usize) -> Vec<Line<'static>> {
    let sections: Vec<Vec<Line<'_>>> =
        whats_new.sections().into_iter().map(|(section, new)| section_lines(section, new, theme, width)).collect();
    sections.join(&Line::default())
}

/// A version's header, then its entries under NEW and FIXED.
fn section_lines(section: &Section, new: bool, theme: Theme, width: usize) -> Vec<Line<'static>> {
    let entries = section.groups().flat_map(|(name, entries)| {
        let title = Line::from(Span::styled(name.to_uppercase(), Style::default().fg(theme.faded).add_modifier(Modifier::BOLD)));
        std::iter::once(title).chain(entries.iter().flat_map(move |entry| entry_lines(entry, theme, width)))
    });
    std::iter::once(header(section, new, theme)).chain(entries).collect()
}

fn header(section: &Section, new: bool, theme: Theme) -> Line<'static> {
    let date = section.date.as_ref().map(|date| format!("  {date}")).unwrap_or_default();
    let mark = if new { NEW_TO_YOU } else { "" };
    Line::from(vec![
        Span::styled(section.title(), Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
        Span::styled(date, Style::default().fg(theme.faded)),
        Span::styled(mark, Style::default().fg(theme.warn)),
    ])
}

/// What in bold after a bullet, then where and how under it, each wrapped with the same indent.
fn entry_lines(entry: &Entry, theme: Theme, width: usize) -> Vec<Line<'static>> {
    let bold = |span: Span<'static>| span.patch_style(Style::default().add_modifier(Modifier::BOLD));
    let what = Line::from(inline(&entry.what, theme).into_iter().map(bold).collect::<Vec<_>>());
    let muted = |span: Span<'static>| if span.style.fg.is_some() { span } else { span.patch_style(Style::default().fg(theme.muted)) };
    let place = Line::from(inline(&entry.place, theme).into_iter().map(muted).collect::<Vec<_>>());
    let how = entry.how.as_ref().map(|how| Line::from(inline(how, theme)));
    let rows = [(what, BULLET), (place, UNDER_BULLET)].into_iter().chain(how.map(|how| (how, UNDER_BULLET)));
    rows.flat_map(|(line, first)| indented(line, first, width)).collect()
}

/// `line` wrapped to `width`, its first row after `first` and the next ones under the text.
fn indented(line: Line<'static>, first: &'static str, width: usize) -> Vec<Line<'static>> {
    let rows = wrap(line, width.saturating_sub(UNDER_BULLET.len()));
    let prefixes = std::iter::once(first).chain(std::iter::repeat(UNDER_BULLET));
    rows.into_iter()
        .zip(prefixes)
        .map(|(row, prefix)| Line::from(std::iter::once(Span::raw(prefix)).chain(row.spans).collect::<Vec<_>>()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_sentence_wraps_under_its_own_text() {
        let rows = indented(Line::from("one two three four five six"), BULLET, 14);
        let text: Vec<String> = rows.iter().map(|row| row.spans.iter().map(|span| span.content.as_ref()).collect()).collect();
        assert_eq!(text, ["• one two ", "  three four ", "  five six"]);
    }
}
