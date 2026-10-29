//! The right pane: every conversation of one place, or of the MR, notes in order, bodies as light markdown.
mod body;
mod conversation;

use super::app::{App, ConversationList, Entry, Focus, List, ListRow, Open, Pane, ReviewInputs};
use super::drag::{self, TextRow};
use super::images::Thumbs;
use super::theme::Theme;
use super::ui::{rule_pane, side_pane};
use crate::review::image::Image;
use crate::review::{Place, Review};
use chrono::{DateTime, Utc};
use conversation::{conversation_lines, heading};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use std::ops::Range;
use unicode_width::UnicodeWidthStr;
use vtui::field::Field;

use body::Anchor;
pub(super) use body::{body_lines, inline};
pub(super) use conversation::{InlineLayout, shown_ages};

/// The compose box grows with its text up to this many rows, then scrolls.
const COMPOSE_ROWS: usize = 8;

/// One piece of a conversation as the pane lays it out: a line of text, or a picture that takes
/// the rows its thumbnail needs, or a line of its own when it cannot be drawn.
enum Piece {
    Text(Line<'static>),
    /// A line of a body, the text as written that a drag copies, and the links drawn in it.
    Written {
        line: Line<'static>,
        raw: String,
        links: Vec<Anchor>,
    },
    Picture(Image),
}

/// A picture ready to draw, and where: the loop paints it over the rows the pane left blank.
pub(super) struct Placement {
    pub(crate) url: String,
    pub(crate) area: Rect,
}

/// What a laid-out row carries besides its text.
#[derive(Debug)]
enum Mark {
    /// The first of the rows reserved for a ready picture.
    Picture { url: String, size: ratatui::layout::Size },
    /// A link's text `column` cells into its row, clickable to its web link: a markdown link,
    /// or a picture shown as its `[image: …]` line.
    Link { column: u16, text: String, url: String },
    /// A row of a body line, and the bytes of the line as written under each of its cells.
    Written { raw: String, cells: Vec<Range<usize>> },
}

/// The pane's rows laid out at a width: its title, each row with the cursor stop it belongs to,
/// and what some rows carry besides their text.
#[derive(Debug)]
struct Laid {
    title: String,
    rows: Vec<(Option<Entry>, Line<'static>)>,
    marks: Vec<(usize, usize, Mark)>,
}

/// The last frame's layout of the pane, and what it was laid out from.
#[derive(Debug)]
pub(crate) struct PaneLayout {
    inputs: PaneInputs,
    laid: Laid,
}

/// Everything the pane's layout reads, the cursor and the scroll aside: they only move within it.
#[derive(Debug, PartialEq)]
struct PaneInputs {
    review: ReviewInputs,
    pane: Pane,
    here: bool,
    width: usize,
    theme: Theme,
    ages: Vec<String>,
    me: String,
    ascii: bool,
    pictures: u64,
}

impl PaneInputs {
    fn from_app(app: &App, width: usize) -> Option<Self> {
        let open = app.open.as_ref()?;
        let pane = open.pane.as_ref()?;
        Some(Self {
            review: ReviewInputs::from_open(open),
            pane: pane.unmoved(),
            here: at_cursor(open, pane),
            width,
            theme: app.theme,
            ages: shown_ages(&open.review, app.today),
            me: app.me.clone(),
            ascii: app.ascii,
            pictures: app.thumbs.changes(),
        })
    }
}

/// The last frame's layout while nothing it was made from changed.
fn cached(app: &App, width: usize) -> Option<&Laid> {
    let last = app.pane_layout.as_ref()?;
    PaneInputs::from_app(app, width).is_some_and(|inputs| inputs == last.inputs).then_some(&last.laid)
}

fn fresh_layout(app: &App, width: usize) -> Option<PaneLayout> {
    Some(PaneLayout { inputs: PaneInputs::from_app(app, width)?, laid: lay_out_pane(app, width)? })
}

/// The pane, and the pictures it made room for: the caller paints them last, above the fades.
/// Under zen's diff it hangs from a rule carrying its title.
pub(crate) fn draw(f: &mut Frame<'_>, app: &mut App, area: Rect, under_diff: bool) -> Vec<Placement> {
    let theme = app.theme;
    let (focused, zen) = (app.focus == Focus::Side, app.zen);
    let block = |title: &str| if under_diff { rule_pane(theme, title) } else { side_pane(theme, title, focused, zen) };
    let inner = block("").inner(area);
    let width = inner.width.saturating_sub(1) as usize;
    if cached(app, width).is_none() {
        app.pane_layout = fresh_layout(app, width);
    }
    let Some(PaneLayout { laid: Laid { title, rows, marks }, .. }) = &app.pane_layout else { return vec![] };
    f.render_widget(block(title), area);
    let compose = app
        .composes_outside_the_diff()
        .then(|| if under_diff { box_rows(&app.buffer, inner.width, COMPOSE_ROWS) } else { box_height(&app.buffer, inner) });
    let label = compose.map(|_| app.input_label()).unwrap_or_default();
    let thread = compose.and(app.input.as_ref()).and_then(|input| app.open.as_ref()?.thread_written_in(input));
    let view = app.open.as_ref().and_then(|open| app.kept.pane_view(open));
    let (entries, current) = view.map(|view| (view.entries.as_slice(), view.focused)).unwrap_or_default();
    let replied = view.zip(thread).and_then(|(view, thread)| view.conversations.iter().position(|c| c.thread.as_ref() == Some(&thread)));
    let under = replied.and_then(|conversation| last_row_of(conversation, rows));
    let (inner, gap) = match (compose, under) {
        (Some(height), Some(last)) => (inner, Some(Gap { at: last + 1, height: usize::from(height) })),
        (Some(height), None) => {
            let [list, box_area] = Layout::vertical([Constraint::Min(0), Constraint::Length(height)]).areas(inner);
            draw_compose(f, theme, &label, &app.buffer, box_area);
            (list, None)
        }
        (None, _) => (inner, None),
    };
    let height = inner.height as usize;
    let on = |entry: &Option<Entry>| entry.is_some() && *entry == current;
    let first = shown(rows.iter().position(|(entry, _)| on(entry)).unwrap_or(0), gap);
    let last = shown(rows.iter().rposition(|(entry, _)| on(entry)).unwrap_or(0), gap);
    let in_replied = current.is_some_and(|entry| Some(entry.conversation) == replied);
    let (first, last) = with_box_in_view(first, last, gap.filter(|_| in_replied), height);
    let Some(pane) = app.open.as_mut().and_then(|open| open.pane.as_mut()) else { return vec![] };
    let scroll = settle(pane.scroll, first, last, height);
    pane.scroll = scroll;
    let y_of = |row: usize| shown(row, gap).checked_sub(scroll).filter(|y| *y < height).map(|y| inner.y + y as u16);
    let visible: Vec<(u16, &Option<Entry>, &Line<'static>)> =
        rows.iter().enumerate().filter_map(|(row, (entry, line))| Some((y_of(row)?, entry, line))).collect();
    for (y, entry, line) in &visible {
        let bar = Span::styled(if on(entry) { "▎" } else { " " }, Style::default().fg(theme.accent));
        let drawn = Line::from(std::iter::once(bar).chain(line.spans.iter().cloned()).collect::<Vec<_>>());
        f.render_widget(drawn, Rect { y: *y, height: 1, ..inner });
    }
    if let Some(gap) = gap {
        let top = inner.y + gap.at.saturating_sub(scroll) as u16;
        let box_area = Rect { y: top, height: gap.height as u16, ..inner }.intersection(inner);
        draw_compose(f, theme, &label, &app.buffer, box_area);
    }
    let list_rows: Vec<ListRow> = visible
        .iter()
        .filter_map(|(y, entry, _)| {
            let index = entries.iter().position(|e| Some(*e) == **entry)?;
            Some(ListRow { area: Rect { y: *y, height: 1, ..inner }, list: List::Threads, index })
        })
        .collect();
    app.list_rows.extend(list_rows);
    let width = inner.width.saturating_sub(1);
    let mut placements = vec![];
    for (row, piece, mark) in marks {
        let Some(y) = y_of(*row) else { continue };
        let x = inner.x + 1;
        match mark {
            Mark::Picture { url, size } if shown(*row, gap) + usize::from(size.height) <= scroll + height => {
                placements.push(Placement { url: url.clone(), area: Rect::new(x, y, size.width.min(width), size.height) });
            }
            Mark::Picture { .. } => {}
            Mark::Link { column, text, url } => {
                app.links.push(super::ui::Link { x: x + column, y, text: text.clone(), url: url.clone() });
            }
            Mark::Written { raw, cells } => {
                app.text_rows.push(TextRow { at: Position::new(x, y), width, line: *piece, text: raw.clone(), cells: cells.clone() });
            }
        }
    }
    placements
}

/// The rows the pane under zen's diff takes `width` columns wide to show all it holds: its rule,
/// its conversations and the compose box, which the pane grows for rather than holding it to a share.
pub(super) fn rows_needed(app: &App, width: u16) -> u16 {
    let inner = rule_pane(app.theme, "").inner(Rect::new(0, 0, width, 1));
    let width = inner.width.saturating_sub(1) as usize;
    let rows = match cached(app, width) {
        Some(laid) => laid.rows.len(),
        None => lay_out_pane(app, width).map_or(0, |laid| laid.rows.len()),
    };
    let compose = if app.composes_outside_the_diff() { box_rows(&app.buffer, inner.width, COMPOSE_ROWS) } else { 0 };
    u16::try_from(rows).unwrap_or(u16::MAX).saturating_add(compose).saturating_add(1)
}

/// The conversations of the open pane as rows `width` cells wide, pictures and links marked.
fn lay_out_pane(app: &App, width: usize) -> Option<Laid> {
    let theme = app.theme;
    let open = app.open.as_ref()?;
    let ConversationList { conversations, entries, .. } = open.pane_view()?;
    let pane = open.pane.as_ref()?;
    let web = |url: &str| app.web_url(open, url);
    let here = at_cursor(open, pane);
    let mut lines: Vec<(Option<Entry>, Piece)> = vec![];
    for (index, conversation) in conversations.iter().enumerate() {
        if index > 0 {
            lines.push((None, Piece::Text(Line::from(Span::styled("─".repeat(width), Style::default().fg(theme.border))))));
        }
        let look = Look { theme, today: app.today, me: &app.me, ascii: app.ascii, width };
        if open.lists_every_thread() {
            let first = entries.iter().find(|e| e.conversation == index).copied();
            lines.extend(heading(&open.review, conversation, look).into_iter().map(|line| (first, Piece::Text(line))));
        }
        lines.extend(conversation_lines(&open.review, conversation, index, &entries, look));
    }
    if conversations.is_empty() && pane.only_with.is_some() {
        lines.push((
            None,
            Piece::Text(Line::from(Span::styled("you take part in no thread · m shows them all", Style::default().fg(theme.faded)))),
        ));
    }
    let more = open.review.others_in_file(&pane.place);
    if more > 0 {
        lines.push((None, Piece::Text(Line::default())));
        let footer = format!("{more} more thread{} in this file · ]n", if more == 1 { "" } else { "s" });
        lines.push((None, Piece::Text(Line::from(Span::styled(footer, Style::default().fg(theme.faded))))));
    }
    let mut rows: Vec<(Option<Entry>, Line<'static>)> = vec![];
    let mut marks: Vec<(usize, usize, Mark)> = vec![];
    for (index, (entry, piece)) in lines.into_iter().enumerate() {
        for (line, row_marks) in lay_out(piece, &app.thumbs, width, theme, &web) {
            marks.extend(row_marks.into_iter().map(|mark| (rows.len(), index, mark)));
            rows.push((entry, line));
        }
    }
    Some(Laid { title: title(&open.review, pane, conversations.len(), here), rows, marks })
}

/// The diff's cursor sits on the place the pane shows.
fn at_cursor(open: &Open, pane: &Pane) -> bool {
    open.row().and_then(|row| open.review.place_of(row)).is_some_and(|place| place == pane.place)
}

/// A piece as rows: text wrapped to `width`; a ready picture as blank rows its thumbnail covers;
/// any other picture as one line, still loading or `[image: alt]`, which clicks through to it.
fn lay_out(piece: Piece, thumbs: &Thumbs, width: usize, theme: Theme, web: &impl Fn(&str) -> String) -> Vec<(Line<'static>, Vec<Mark>)> {
    let image = match piece {
        Piece::Text(line) => return wrap(line, width).into_iter().map(|l| (l, vec![])).collect(),
        Piece::Written { line, raw, links } => return written_rows(line, &raw, &links, width, web),
        Piece::Picture(image) => image,
    };
    let cols = u16::try_from(width).unwrap_or(u16::MAX);
    if let Some(size) = thumbs.cells(&image.url, cols) {
        let mark = Mark::Picture { url: image.url, size };
        let mut rows = vec![(Line::default(), vec![mark])];
        rows.extend((1..size.height).map(|_| (Line::default(), vec![])));
        return rows;
    }
    if matches!(thumbs.get(&image.url), Some(super::images::Thumb::Loading)) {
        return vec![(Line::from(Span::styled("… loading image", Style::default().fg(theme.faded))), vec![])];
    }
    let text = vtui::text::truncate(&fallback(&image), width);
    let mark = Mark::Link { column: 0, text: text.clone(), url: web(&image.url) };
    vec![(Line::from(Span::styled(text, Style::default().fg(theme.link))), vec![mark])]
}

/// A body line wrapped to `width`, each row with the bytes of `raw` under its cells and the part
/// of each link it shows.
fn written_rows(
    line: Line<'static>,
    raw: &str,
    links: &[Anchor],
    width: usize,
    web: &impl Fn(&str) -> String,
) -> Vec<(Line<'static>, Vec<Mark>)> {
    let drawn: String = line.spans.iter().map(|span| span.content.as_ref()).collect();
    let dropped: Vec<Range<usize>> = links.iter().flat_map(|link| link.markup.clone()).collect();
    let cells = drag::aligned(&drawn, raw, &dropped);
    let rows = wrap(line, width).into_iter().scan(0, |start, row| {
        let end = (*start + row.width()).min(cells.len());
        let written = Mark::Written { raw: raw.to_owned(), cells: cells[*start..end].to_vec() };
        let shown = links.iter().filter_map(|link| link_in_row(&row, *start..end, link, web));
        let marks = std::iter::once(written).chain(shown).collect();
        *start = end;
        Some((row, marks))
    });
    rows.collect()
}

/// The part of `link` a row covering the line's cells `row_cells` shows, if any.
fn link_in_row(row: &Line<'_>, row_cells: Range<usize>, link: &Anchor, web: &impl Fn(&str) -> String) -> Option<Mark> {
    let shown = link.cells.start.max(row_cells.start)..link.cells.end.min(row_cells.end);
    if shown.is_empty() {
        return None;
    }
    let (from, to) = (shown.start - row_cells.start, shown.end - row_cells.start);
    let drawn: String = row.spans.iter().map(|span| span.content.as_ref()).collect();
    let text = drag::text_on_cells(&drawn, from..to).to_owned();
    Some(Mark::Link { column: u16::try_from(from).ok()?, text, url: web(link.url.as_str()) })
}

/// How a picture reads where it cannot be drawn.
fn fallback(image: &Image) -> String {
    if image.alt.trim().is_empty() { "[image]".to_owned() } else { format!("[image: {}]", image.alt.trim()) }
}

/// Text rows the box shows: its own lines, at least one, at most 8 or 40 % of the pane, plus its border.
pub(super) fn box_height(field: &Field, pane: Rect) -> u16 {
    box_rows(field, pane.width, usize::from(pane.height) * 40 / 100)
}

/// The compose box's rows in a pane `width` cells wide, its text held to `most` rows.
fn box_rows(field: &Field, width: u16, most: usize) -> u16 {
    let width = width.saturating_sub(4).max(1) as usize;
    let rows: usize = field.text().split('\n').map(|line| line.width().max(1).div_ceil(width)).sum();
    (rows.clamp(1, most.clamp(1, COMPOSE_ROWS)) + 2) as u16
}

/// The compose box: its target in the top border, the keys in the bottom one, the caret reversed.
pub(super) fn draw_compose(f: &mut Frame<'_>, theme: Theme, label: &str, field: &Field, area: Rect) {
    let block = ratatui::widgets::Block::bordered()
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_focus))
        .title(Span::styled(format!(" {label} "), Style::default().fg(theme.accent)))
        .title_bottom(Span::styled(" enter save · ⌘enter post · ⌥enter newline ", Style::default().fg(theme.faded)));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let width = inner.width.max(1) as usize;
    let (before, under, after) = field.split();
    let caret = if under.is_empty() || under == "\n" { " " } else { under };
    let tail = if under == "\n" { format!("\n{after}") } else { after.to_owned() };
    let mut rows: Vec<Line<'static>> = vec![];
    let mut current: Vec<Span<'static>> = vec![];
    let push_text = |text: &str, style: Style, rows: &mut Vec<Line<'static>>, current: &mut Vec<Span<'static>>| {
        let mut parts = text.split('\n');
        if let Some(first) = parts.next() {
            current.push(Span::styled(first.to_owned(), style));
        }
        for part in parts {
            rows.extend(wrap(Line::from(std::mem::take(current)), width));
            current.push(Span::styled(part.to_owned(), style));
        }
    };
    push_text(before, Style::default(), &mut rows, &mut current);
    let caret_row = rows.len() + wrap(Line::from(current.clone()), width).len().saturating_sub(1);
    current.push(Span::styled(caret.to_owned(), Style::default().add_modifier(Modifier::REVERSED)));
    push_text(&tail, Style::default(), &mut rows, &mut current);
    rows.extend(wrap(Line::from(current), width));
    let height = inner.height as usize;
    let scroll = (caret_row + 1).saturating_sub(height);
    f.render_widget(Paragraph::new(rows.into_iter().skip(scroll).take(height).collect::<Vec<_>>()), inner);
}

/// The last of the pane's `rows` showing the conversation at index `conversation`.
fn last_row_of(conversation: usize, rows: &[(Option<Entry>, Line<'static>)]) -> Option<usize> {
    rows.iter().rposition(|(entry, _)| entry.is_some_and(|e| e.conversation == conversation))
}

/// Where the compose box sits among the pane's rows: under the thread it answers, before row `at`.
#[derive(Clone, Copy)]
struct Gap {
    at: usize,
    height: usize,
}

/// The pane position of row `row`, once the box makes room among the rows.
fn shown(row: usize, gap: Option<Gap>) -> usize {
    match gap {
        Some(gap) if row >= gap.at => row + gap.height,
        _ => row,
    }
}

/// The span to keep in view: the cursor's lines and, while the cursor is in the thread the box answers, the whole box
/// with as much of the thread above it as fits.
fn with_box_in_view(first: usize, last: usize, gap: Option<Gap>, height: usize) -> (usize, usize) {
    let Some(Gap { at, height: rows }) = gap else { return (first, last) };
    let end = at + rows;
    (first.min(at).max(end.saturating_sub(height)), last.max(end - 1))
}

/// Keeps the cursor's lines in view: the top of the entry first, then as much of it as fits.
fn settle(scroll: usize, first: usize, last: usize, height: usize) -> usize {
    if height == 0 {
        return 0;
    }
    if first < scroll {
        return first;
    }
    if last >= scroll + height {
        return (last + 1).saturating_sub(height).min(first);
    }
    scroll
}

/// `charge.rs:57 · 2 threads`, `charge.rs:-13` for the old side, `on the MR`, `charge.rs · outdated`,
/// `the whole MR · mine · 3 of 12 threads`; `↑ line 57` when the cursor moved to a line without conversations.
fn title(review: &Review, pane: &Pane, count: usize, here: bool) -> String {
    let name = |file: usize| {
        let path = &review.files[file].new_path;
        path.rsplit('/').next().unwrap_or(path).to_owned()
    };
    let plural = |n: usize| if n == 1 { "thread" } else { "threads" };
    let threads = match pane.only_with {
        Some(_) => {
            let every = review.conversations(&pane.place).len();
            format!("mine · {count} of {every} {}", plural(every))
        }
        None => format!("{count} {}", plural(count)),
    };
    let (head, line) = match &pane.place {
        Place::Line { file, new: Some(n), .. } => (format!("{}:{n}", name(*file)), Some(n.to_string())),
        Place::Line { file, old: Some(o), .. } => (format!("{}:-{o}", name(*file)), Some(format!("-{o}"))),
        Place::Line { file, .. } => (name(*file), None),
        Place::Mr => ("on the MR".to_owned(), None),
        Place::Outdated { file } => (format!("{} · outdated", name(*file)), None),
        Place::All => ("the whole MR".to_owned(), None),
    };
    let head = if count == 0 && pane.only_with.is_none() { head } else { format!("{head} · {threads}") };
    match (here, line) {
        (false, Some(line)) => format!("{head} · ↑ line {line}"),
        _ => head,
    }
}

/// How notes read: the palette, today for ages, who "you" is, emoji or their plain words, and
/// the columns a table may take.
#[derive(Clone, Copy)]
pub(super) struct Look<'a> {
    pub(super) theme: Theme,
    pub(super) today: DateTime<Utc>,
    pub(super) me: &'a str,
    pub(super) ascii: bool,
    pub(super) width: usize,
}

/// A line cut at word boundaries to `width`, styles kept; a word longer than the width is cut.
pub(super) fn wrap(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    if width == 0 || line.width() <= width {
        return vec![line];
    }
    let mut rows: Vec<Vec<Span<'static>>> = vec![vec![]];
    let mut used = 0;
    for span in line.spans {
        for word in span.content.split_inclusive(' ') {
            let w = word.width();
            if used + w > width && used > 0 {
                rows.push(vec![]);
                used = 0;
            }
            let mut rest = word;
            while rest.width() > width {
                let cut = rest.char_indices().scan(0, |acc, (i, c)| {
                    *acc += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
                    Some((i, *acc))
                });
                let at = cut
                    .take_while(|&(_, acc)| acc <= width)
                    .last()
                    .map_or(rest.len(), |(i, _)| i + rest[i..].chars().next().map_or(0, char::len_utf8));
                if let Some(row) = rows.last_mut() {
                    row.push(Span::styled(rest[..at].to_owned(), span.style));
                }
                rows.push(vec![]);
                rest = &rest[at..];
            }
            if let Some(row) = rows.last_mut() {
                row.push(Span::styled(rest.to_owned(), span.style));
            }
            used += rest.width();
        }
    }
    rows.into_iter().map(Line::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn text(lines: &[Line<'_>]) -> Vec<String> {
        lines.iter().map(|l| l.spans.iter().map(|s| s.content.to_string()).collect()).collect()
    }

    #[test]
    fn a_picture_that_cannot_be_drawn_reads_as_its_alt_and_links_to_the_forge() {
        let thumbs = Thumbs::disabled();
        let web = |url: &str| crate::forge::image::web_url("gitlab.com", &"acme/widgets".into(), url);
        let image = Image { alt: "the chart".into(), url: "/uploads/ab12/chart.png".into() };
        let rows = lay_out(Piece::Picture(image), &thumbs, 40, Theme::default(), &web);
        assert_eq!(text(&[rows[0].0.clone()]), ["[image: the chart]"]);
        let [Mark::Link { url, .. }] = rows[0].1.as_slice() else { panic!("a link") };
        assert_eq!(url, "https://gitlab.com/acme/widgets/uploads/ab12/chart.png");
        assert_eq!(fallback(&Image { alt: " ".into(), url: "u".into() }), "[image]");
        assert_eq!(text(&body_lines("![c](https://x/c.png)", 80, Theme::default())), ["[image: c]"], "other panes show the line");
    }

    #[test]
    fn a_ready_picture_takes_the_rows_of_its_thumbnail() {
        let mut thumbs = super::super::images::tests::test_thumbs();
        thumbs.arrived("https://x/a.png", Some(::image::DynamicImage::new_rgb8(200, 100)));
        let image = Image { alt: String::new(), url: "https://x/a.png".into() };
        let rows = lay_out(Piece::Picture(image.clone()), &thumbs, 40, Theme::default(), &|u: &str| u.to_owned());
        assert_eq!(rows.len(), 5, "200x100 pixels at a 10x20 font is 20x5 cells");
        assert!(matches!(rows[0].1.as_slice(), [Mark::Picture { size, .. }] if size.width == 20 && size.height == 5));
        assert!(rows[1..].iter().all(|(_, marks)| marks.is_empty()));
        thumbs.request_new(["https://x/b.png".to_owned()]);
        let loading =
            lay_out(Piece::Picture(Image { url: "https://x/b.png".into(), ..image }), &thumbs, 40, Theme::default(), &|u: &str| {
                u.to_owned()
            });
        assert_eq!(text(&[loading[0].0.clone()]), ["… loading image"]);
    }

    #[test]
    fn the_title_names_the_line_and_marks_the_old_side() {
        let review = crate::review::tests::review();
        let old = Pane::at(Place::Line { file: 0, new: None, old: Some(13) });
        assert_eq!(title(&review, &old, 1, true), "charge.rs:-13 · 1 thread");
        assert_eq!(title(&review, &old, 2, false), "charge.rs:-13 · 2 threads · ↑ line -13");
        assert_eq!(title(&review, &Pane::at(Place::Mr), 1, true), "on the MR · 1 thread");
    }

    #[test]
    fn the_title_of_my_threads_counts_them_among_every_thread() {
        let review = crate::review::tests::review();
        let mine = Pane { only_with: Some("nina".into()), ..Pane::at(Place::All) };
        assert_eq!(title(&review, &Pane::at(Place::All), 3, true), "the whole MR · 3 threads");
        assert_eq!(title(&review, &mine, 1, true), "the whole MR · mine · 1 of 3 threads");
        assert_eq!(title(&review, &mine, 0, true), "the whole MR · mine · 0 of 3 threads", "an empty filter still says so");
    }

    #[test]
    fn long_lines_wrap_at_words_and_long_words_are_cut() {
        let rows = wrap(Line::from("one two three"), 8);
        assert_eq!(text(&rows), ["one two ", "three"]);
        assert_eq!(text(&wrap(Line::from("abcdefghij"), 4)), ["abcd", "efgh", "ij"]);
    }

    #[test]
    fn the_cursor_entry_stays_in_view() {
        assert_eq!(settle(0, 2, 3, 10), 0);
        assert_eq!(settle(0, 12, 14, 10), 5);
        assert_eq!(settle(8, 3, 4, 10), 3);
        assert_eq!(settle(0, 2, 30, 10), 2, "a long entry shows from its top");
    }

    #[test]
    fn the_box_under_a_thread_stays_in_view_with_the_thread_above_it() {
        let gap = Some(Gap { at: 12, height: 4 });
        assert_eq!((shown(11, gap), shown(12, gap)), (11, 16), "rows after the box move down by its height");
        assert_eq!(with_box_in_view(8, 11, gap, 10), (8, 15), "thread and box fit");
        assert_eq!(with_box_in_view(2, 11, gap, 10), (6, 15), "the box shows whole, the thread as much as fits");
        assert_eq!(with_box_in_view(2, 3, None, 10), (2, 3));
    }
}
