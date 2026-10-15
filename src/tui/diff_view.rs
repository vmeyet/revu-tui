//! The review pane: rows from `Review::rows()` turned into styled lines, only for the visible window.
mod anchors;
mod header;
mod line;
mod rows;
mod select;
mod side_by_side;
#[cfg(test)]
mod test_support;
mod wrap;

use super::app::{App, Focus, Open, PIN_MIN_HEIGHT, Pins, pins, settle_with_pins};
use super::theme::Theme;
use super::thread_view;
use super::ui::{draw_empty, pane, settle_scroll, truncate, truncate_middle};
use crate::review::{Review, Row};
use anchors::{Anchors, focused_range};
pub(super) use header::pipeline_glyph;
use header::{deployment_link, header_lines, mr_ref, pipeline_link, title_link, zen_header};
use line::number;
pub(super) use line::{coloured, paint};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Padding, Paragraph};
use rows::row_line;
use select::text_rows;
use side_by_side::{SIDE_BY_SIDE_MIN_W, side_by_side_lines};
use std::collections::VecDeque;
use unicode_width::UnicodeWidthStr;
use wrap::wrap_row;

const GUTTER_W: usize = 4;
/// The glyph and count column before the line numbers.
const ANCHOR_W: usize = 2;
const TAB: &str = "→   ";
/// What sets a thread's rows off from the code, ending in the sign's column.
const THREAD_RULE: &str = "│ ";

/// The line numbers before the sign: both sides, the one side a line is read on when the window is narrow, or none
/// in the compact layout, which leaves the sign alone.
#[derive(Clone, Copy)]
pub(super) enum Gutter {
    Both,
    One,
    Sign,
}

impl Gutter {
    /// The new number alone when there is one column, the old one on a line the new side does not have.
    fn numbers(self, old: Option<u32>, new: Option<u32>) -> String {
        match self {
            Self::Both => format!("{} {} ", number(old), number(new)),
            Self::One => format!("{} ", number(new.or(old))),
            Self::Sign => String::new(),
        }
    }

    fn width(self) -> usize {
        match self {
            Self::Both => 2 * (GUTTER_W + 1),
            Self::One => GUTTER_W + 1,
            Self::Sign => 0,
        }
    }

    /// A file's path cut to `room`: from its end, or from its middle in the compact layout, where a cut end would
    /// take the file's name.
    fn path(self, path: &str, room: usize) -> String {
        match self {
            Self::Both | Self::One => truncate(path, room),
            Self::Sign => truncate_middle(path, room),
        }
    }

    /// The cursor's line at the right of the pane's title, where no row shows its number.
    fn cursor_title(self, line: Option<u32>, theme: Theme) -> Option<Line<'static>> {
        let number = line.filter(|_| matches!(self, Self::Sign))?;
        Some(Line::from(Span::styled(format!(" L{number} "), Style::default().fg(theme.muted))).right_aligned())
    }

    /// Continuation rows start under the text: past the cursor bar, the anchor column, the numbers and the sign.
    fn wrap_indent(self) -> usize {
        1 + ANCHOR_W + self.width() + 1
    }

    /// The columns a thread's rows have in a diff `width` wide: from where the code starts to one short of the edge.
    fn thread_width(self, width: usize) -> usize {
        width.saturating_sub(self.wrap_indent() + 1)
    }

    /// The columns of `body` a compose box takes under a thread: its frame around the columns of the thread's rows.
    fn thread_box(self, body: Rect) -> Rect {
        let frame = 1;
        let width = self.thread_width(usize::from(body.width)) + 2 * frame;
        Rect { x: body.x + (self.wrap_indent() - frame) as u16, width: width as u16, ..body }
    }
}

pub(crate) fn draw(f: &mut Frame<'_>, app: &mut App, area: Rect, gutter: Gutter) {
    let theme = app.theme;
    let title = match &app.open {
        Some(open) => {
            format!(
                "{} · {}",
                mr_ref(&open.review, app.hosts.kind_of(&open.key)),
                truncate(&open.review.mr.title, area.width.saturating_sub(30) as usize)
            )
        }
        None => "Review".to_owned(),
    };
    let zen = app.zen;
    let block = if zen { Block::default().padding(Padding::horizontal(1)) } else { pane(theme, &title, app.focus == Focus::Review) };
    let block = match gutter.cursor_title(app.cursor_line(), theme) {
        Some(line) => block.title_top(line),
        None => block,
    };
    if let Some(open) = app.open.as_ref().filter(|_| !zen) {
        app.links.push(title_link(&open.review, app.hosts.kind_of(&open.key), area));
    }
    let inner = block.inner(area);
    f.render_widget(block, area);
    if app.open.is_none() {
        let lines: &[&str] = match app.opening {
            Some(_) => &[app.spinner(), "", "fetching the merge request"],
            None => &["open a merge request", "to read its diff here"],
        };
        return draw_empty(f, theme, inner, lines);
    }
    let today = app.today;
    let sigil = app.open.as_ref().map_or('!', |o| app.hosts.kind_of(&o.key).sigil());
    let wrap = app.wraps();
    let short = app.short;
    let sought = app.search.as_ref().map(|s| s.query.clone()).filter(|q| !q.is_empty());
    app.fit_diff(inner.width as usize >= SIDE_BY_SIDE_MIN_W);
    let compose_row = app.compose_row();
    let compose_label = app.input_label();
    let composes_at_foot = app.composes_outside_the_diff() && app.areas.side.is_empty();
    let Some(open) = app.open.as_mut() else { return };
    let header = match () {
        () if zen => vec![zen_header(open, sigil, theme, inner.width as usize), Line::default()],
        () if short => vec![],
        () => header_lines(open, theme, today, inner.width as usize),
    };
    let body = Rect { y: inner.y + header.len() as u16, height: inner.height.saturating_sub(header.len() as u16), ..inner };
    let pipeline = if zen { None } else { pipeline_link(&open.review, &header, inner) };
    let deployment = if zen { None } else { deployment_link(open, &header, inner) };
    f.render_widget(Paragraph::new(header), inner);
    app.links.extend(pipeline.into_iter().chain(deployment));
    if open.prose.is_some() {
        return super::prose_view::draw(f, app, body);
    }
    let body = if composes_at_foot { above_compose(f, theme, &compose_label, &app.buffer, body) } else { body };
    let compose_box = gutter.thread_box(body);
    let compose = compose_row.map(|row| (row, usize::from(thread_view::box_height(&app.buffer, compose_box))));
    let look = thread_view::Look { theme, today, me: &app.me, ascii: app.ascii, width: gutter.thread_width(usize::from(body.width)) };
    if open.review.reads_threads_in_diff() {
        app.thread_rows.fit(open, look);
    }
    let thread_rows = &mut app.thread_rows;
    let on_row = open.row_view();
    let stretch = focused_range(&open.review, on_row.as_ref().or_else(|| app.kept.pane_view(open)));
    let anchors = Anchors { markers: app.kept.markers(open), stretch };
    let height = body.height as usize;
    let width = body.width as usize;
    let mut row_lines = |open: &Open, i: usize| -> Vec<Line<'static>> {
        let row = &open.rows[i];
        let marked = i == open.selected || open.is_selected(i);
        if let (Row::Thread { stop, .. }, Some(place)) = (row, open.review.place_of(row)) {
            let rows = thread_rows.stops(&open.review, &place, look).get(*stop);
            return thread_lines(rows.map_or(&[], Vec::as_slice), marked, gutter, theme);
        }
        let overflow = if wrap { Overflow::Wrap(width) } else { Overflow::Cut(width) };
        if open.review.shows_side_by_side()
            && let Some(lines) = side_by_side_lines(&open.review, &anchors, row, marked, overflow, theme)
        {
            return lines;
        }
        let text_row = matches!(row, Row::Line { .. } | Row::Pair { .. } | Row::Context { .. });
        let found = |line: Line<'static>| match &sought {
            Some(query) if text_row => {
                super::app::mark_search(&line, query, gutter.wrap_indent(), Style::default().add_modifier(Modifier::REVERSED))
            }
            _ => line,
        };
        if wrap && text_row {
            wrap_row(found(row_line(&open.review, &anchors, row, marked, gutter, overflow, theme)), width, gutter.wrap_indent())
        } else {
            vec![found(row_line(&open.review, &anchors, row, marked, gutter, Overflow::Cut(width), theme))]
        }
    };
    let mut render = |open: &Open, i: usize| -> Vec<Line<'static>> {
        let room = compose.filter(|(row, _)| *row == i).map_or(0, |(_, rows)| rows);
        row_lines(open, i).into_iter().chain(std::iter::repeat_n(Line::default(), room)).collect()
    };
    let pinning = height >= PIN_MIN_HEIGHT;
    let pinned_at = |open: &Open| if pinning { pins(&open.rows, open.scroll, open.selected) } else { Pins::default() };
    open.scroll = if pinning {
        settle_with_pins(&open.rows, open.scroll, open.selected, height).0
    } else {
        settle_scroll(open.scroll, open.selected, height)
    };
    let mut drawn: VecDeque<Vec<Line<'_>>> = (open.scroll..=open.selected).map(|i| render(open, i)).collect();
    let mut drawn_height: usize = drawn.iter().map(Vec::len).sum();
    while open.scroll < open.selected && drawn_height > height.saturating_sub(pinned_at(open).count()) {
        drawn_height -= drawn.pop_front().map_or(0, |row| row.len());
        open.scroll += 1;
    }
    let pinned = pinned_at(open);
    open.pinned_file = pinned.file;
    let pinned_lines: Vec<Line<'_>> = [pinned.file, pinned.hunk]
        .into_iter()
        .flatten()
        .map(|i| pinned_line(&open.review, &anchors, &open.rows[i], gutter, width, theme))
        .collect();
    let mut lines: Vec<Line<'_>> = pinned_lines;
    let mut selectable = vec![];
    let mut compose_area = None;
    for i in open.scroll..open.rows.len() {
        if lines.len() >= height {
            break;
        }
        let top = body.y + lines.len() as u16;
        selectable.extend(text_rows(&open.review, &open.rows[i], i, Rect { y: top, ..body }, gutter, wrap));
        lines.extend(drawn.pop_front().unwrap_or_else(|| render(open, i)));
        if let Some((_, rows)) = compose.filter(|(row, _)| *row == i) {
            let top = body.y + lines.len().saturating_sub(rows) as u16;
            compose_area = Some(Rect { y: top, height: rows as u16, ..compose_box }.intersection(body));
        }
    }
    lines.truncate(height);
    selectable.retain(|row| row.at.y < body.bottom());
    f.render_widget(Paragraph::new(lines), body);
    if let Some(area) = compose_area.filter(|area| !area.is_empty()) {
        thread_view::draw_compose(f, theme, &compose_label, &app.buffer, area);
    }
    app.text_rows.extend(selectable);
}

/// The diff's `body` over the compose box, drawn at its foot while what is written has no row on screen to hang
/// under and no pane shows the box.
fn above_compose(f: &mut Frame<'_>, theme: Theme, label: &str, text: &super::field::Field, body: Rect) -> Rect {
    let rows = thread_view::box_height(text, body);
    let [body, compose] = Layout::vertical([Constraint::Min(0), Constraint::Length(rows)]).areas(body);
    thread_view::draw_compose(f, theme, label, text, compose);
    body
}

/// One cursor stop of a conversation under its line: its rows behind a rule, starting where the code does.
fn thread_lines(rows: &[Line<'static>], marked: bool, gutter: Gutter, theme: Theme) -> Vec<Line<'static>> {
    let bar = Span::styled(if marked { "▎" } else { " " }, Style::default().fg(theme.accent));
    let before_rule = gutter.wrap_indent() - 1 - THREAD_RULE.width();
    let rule = Span::styled(format!("{}{THREAD_RULE}", " ".repeat(before_rule)), Style::default().fg(theme.border));
    rows.iter().map(|row| Line::from([vec![bar.clone(), rule.clone()], row.spans.clone()].concat())).collect()
}

/// A header row pinned above the diff: drawn like the row itself, on the surface colour so it
/// reads as a bar the diff scrolls under.
fn pinned_line<'a>(review: &Review, anchors: &Anchors<'_>, row: &Row, gutter: Gutter, width: usize, theme: Theme) -> Line<'a> {
    let line = row_line(review, anchors, row, false, gutter, Overflow::Cut(width), theme);
    let used: usize = line.spans.iter().map(Span::width).sum();
    let mut spans = line.spans;
    spans.push(Span::raw(" ".repeat(width.saturating_sub(used))));
    Line::from(spans.into_iter().map(|s| s.patch_style(Style::default().bg(theme.surface))).collect::<Vec<_>>())
}

/// The width a line is drawn at before `w` cuts it into screen rows; wide enough for any real line.
const UNCUT: usize = 4_096;

/// What a line wider than the pane does: cut at `width` with `…`, or drawn whole for `wrap_row` to wrap at `width`.
#[derive(Clone, Copy)]
enum Overflow {
    Cut(usize),
    Wrap(usize),
}
