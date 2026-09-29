//! The right pane: every conversation of one place, or of the MR, notes in order, bodies as light markdown.
use super::app::{App, Entry, EntryKind, Focus, Open, Pane};
use super::drag::{self, TextRow};
use super::field::Field;
use super::images::Thumbs;
use super::table::table_lines;
use super::theme::Theme;
use super::ui::{rule_pane, short_age, side_pane};
use crate::forge::Note;
use crate::review::image::{self, Image};
use crate::review::{Anchor, Conversation, Draft, File, Place, Review, Side, Spot, Thread};
use chrono::{DateTime, Utc};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use std::ops::Range;
use std::sync::Arc;
use unicode_width::UnicodeWidthStr;

/// The compose box grows with its text up to this many rows, then scrolls.
const COMPOSE_ROWS: usize = 8;

/// One piece of a conversation as the pane lays it out: a line of text, or a picture that takes
/// the rows its thumbnail needs, or a line of its own when it cannot be drawn.
enum Piece {
    Text(Line<'static>),
    /// A line of a body, and the text as written that a drag copies.
    Written {
        line: Line<'static>,
        raw: String,
    },
    Picture(Image),
}

/// A picture ready to draw, and where: the loop paints it over the rows the pane left blank.
pub struct Placement {
    pub url: String,
    pub area: Rect,
}

/// What a laid-out row carries besides its text.
#[derive(Debug)]
enum Mark {
    /// The first of the rows reserved for a ready picture.
    Picture { url: String, size: ratatui::layout::Size },
    /// A picture shown as its `[image: …]` line, clickable to its web link.
    Link { text: String, url: String },
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
pub struct PaneLayout {
    inputs: PaneInputs,
    laid: Laid,
}

/// Everything the pane's layout reads, the cursor and the scroll aside: they only move within it.
#[derive(Debug)]
struct PaneInputs {
    threads: Arc<[Thread]>,
    files: Arc<[File]>,
    drafts: Vec<Draft>,
    pane: Pane,
    here: bool,
    width: usize,
    theme: Theme,
    /// Ages next to notes read in whole seconds.
    second: i64,
    me: String,
    ascii: bool,
    pictures: u64,
}

impl PaneInputs {
    fn of(app: &App, width: usize) -> Option<Self> {
        let open = app.open.as_ref()?;
        let pane = open.pane.as_ref()?;
        Some(Self {
            threads: Arc::clone(&open.review.threads),
            files: Arc::clone(&open.review.files),
            drafts: open.review.drafts.clone(),
            pane: Pane { note: 0, scroll: 0, ..pane.clone() },
            here: at_cursor(open, pane),
            width,
            theme: app.theme,
            second: app.today.timestamp(),
            me: app.me.clone(),
            ascii: app.ascii,
            pictures: app.thumbs.changes(),
        })
    }
}

/// Threads and files by identity: a review shares them until a fetch replaces them, and a held copy
/// keeps their memory from being reused by the next ones.
impl PartialEq for PaneInputs {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.threads, &other.threads)
            && Arc::ptr_eq(&self.files, &other.files)
            && (&self.drafts, &self.pane, self.here, self.width, self.theme, self.second, &self.me, self.ascii, self.pictures)
                == (&other.drafts, &other.pane, other.here, other.width, other.theme, other.second, &other.me, other.ascii, other.pictures)
    }
}

/// The last frame's layout while nothing it was made from changed.
fn cached(app: &App, width: usize) -> Option<&Laid> {
    let last = app.pane_layout.as_ref()?;
    PaneInputs::of(app, width).is_some_and(|inputs| inputs == last.inputs).then_some(&last.laid)
}

fn fresh_layout(app: &App, width: usize) -> Option<PaneLayout> {
    Some(PaneLayout { inputs: PaneInputs::of(app, width)?, laid: lay_out_pane(app, width)? })
}

/// The pane, and the pictures it made room for: the caller paints them last, above the fades.
/// Under zen's diff it hangs from a rule carrying its title.
pub fn draw(f: &mut Frame, app: &mut App, area: Rect, under_diff: bool) -> Vec<Placement> {
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
    let inner = if app.input.is_some() {
        let height = if under_diff { box_rows(&app.buffer, inner.width, COMPOSE_ROWS) } else { box_height(&app.buffer, inner) };
        let [list, box_area] = Layout::vertical([Constraint::Min(0), Constraint::Length(height)]).areas(inner);
        draw_compose(f, theme, &app.input_label(), &app.buffer, box_area);
        list
    } else {
        inner
    };
    let height = inner.height as usize;
    let current = app.open.as_ref().and_then(Open::pane_view).and_then(|(_, _, current)| current);
    let on = |entry: &Option<Entry>| entry.is_some() && *entry == current;
    let first = rows.iter().position(|(entry, _)| on(entry)).unwrap_or(0);
    let last = rows.iter().rposition(|(entry, _)| on(entry)).unwrap_or(0);
    let Some(pane) = app.open.as_mut().and_then(|open| open.pane.as_mut()) else { return vec![] };
    let scroll = settle(pane.scroll, first, last, height);
    pane.scroll = scroll;
    let drawn: Vec<Line> = rows
        .iter()
        .skip(scroll)
        .take(height)
        .map(|(entry, line)| {
            let bar = Span::styled(if on(entry) { "▎" } else { " " }, Style::default().fg(theme.accent));
            Line::from(std::iter::once(bar).chain(line.spans.iter().cloned()).collect::<Vec<_>>())
        })
        .collect();
    f.render_widget(Paragraph::new(drawn), inner);
    let width = inner.width.saturating_sub(1);
    let mut placements = vec![];
    for (row, piece, mark) in marks {
        let Some(y) = row.checked_sub(scroll).filter(|y| *y < height) else { continue };
        let (x, y) = (inner.x + 1, inner.y + y as u16);
        match mark {
            Mark::Picture { url, size } if row + usize::from(size.height) <= scroll + height => {
                placements.push(Placement { url: url.clone(), area: Rect::new(x, y, size.width.min(width), size.height) });
            }
            Mark::Picture { .. } => {}
            Mark::Link { text, url } => app.links.push(super::ui::Link { x, y, text: text.clone(), url: url.clone() }),
            Mark::Written { raw, cells } => {
                app.text_rows.push(TextRow { at: Position::new(x, y), width, line: *piece, text: raw.clone(), cells: cells.clone() });
            }
        }
    }
    placements
}

/// The rows the pane under zen's diff takes `width` columns wide to show all it holds: its rule,
/// its conversations and the compose box, which the pane grows for rather than holding it to a share.
pub fn rows_needed(app: &App, width: u16) -> u16 {
    let inner = rule_pane(app.theme, "").inner(Rect::new(0, 0, width, 1));
    let width = inner.width.saturating_sub(1) as usize;
    let rows = match cached(app, width) {
        Some(laid) => laid.rows.len(),
        None => lay_out_pane(app, width).map_or(0, |laid| laid.rows.len()),
    };
    let compose = if app.input.is_some() { box_rows(&app.buffer, inner.width, COMPOSE_ROWS) } else { 0 };
    u16::try_from(rows).unwrap_or(u16::MAX).saturating_add(compose).saturating_add(1)
}

/// The conversations of the open pane as rows `width` cells wide, pictures and links marked.
fn lay_out_pane(app: &App, width: usize) -> Option<Laid> {
    let theme = app.theme;
    let open = app.open.as_ref()?;
    let (conversations, entries, _) = open.pane_view()?;
    let pane = open.pane.as_ref()?;
    let web = |url: &str| crate::forge::image::web_url(open.key.host.as_deref().unwrap_or(&app.host), &open.key.project, url);
    let here = at_cursor(open, pane);
    let mut lines: Vec<(Option<Entry>, Piece)> = vec![];
    for (index, conversation) in conversations.iter().enumerate() {
        if index > 0 {
            lines.push((None, Piece::Text(Line::from(Span::styled("─".repeat(width), Style::default().fg(theme.border))))));
        }
        let look = Look { theme, today: app.today, me: &app.me, ascii: app.ascii, width };
        lines.extend(conversation_lines(open, conversation, index, &entries, look));
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
        for (line, mark) in lay_out(piece, &app.thumbs, width, theme, &web) {
            if let Some(mark) = mark {
                marks.push((rows.len(), index, mark));
            }
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
fn lay_out(piece: Piece, thumbs: &Thumbs, width: usize, theme: Theme, web: &impl Fn(&str) -> String) -> Vec<(Line<'static>, Option<Mark>)> {
    let image = match piece {
        Piece::Text(line) => return wrap(line, width).into_iter().map(|l| (l, None)).collect(),
        Piece::Written { line, raw } => return written_rows(line, &raw, width),
        Piece::Picture(image) => image,
    };
    let cols = u16::try_from(width).unwrap_or(u16::MAX);
    if let Some(size) = thumbs.cells(&image.url, cols) {
        let mark = Mark::Picture { url: image.url, size };
        let mut rows = vec![(Line::default(), Some(mark))];
        rows.extend((1..size.height).map(|_| (Line::default(), None)));
        return rows;
    }
    if matches!(thumbs.get(&image.url), Some(super::images::Thumb::Loading)) {
        return vec![(Line::from(Span::styled("… loading image", Style::default().fg(theme.faded))), None)];
    }
    let text = super::ui::truncate(&fallback(&image), width);
    let mark = Mark::Link { text: text.clone(), url: web(&image.url) };
    vec![(Line::from(Span::styled(text, Style::default().fg(theme.link))), Some(mark))]
}

/// A body line wrapped to `width`, each row with the bytes of `raw` under its cells.
fn written_rows(line: Line<'static>, raw: &str, width: usize) -> Vec<(Line<'static>, Option<Mark>)> {
    let drawn: String = line.spans.iter().map(|span| span.content.as_ref()).collect();
    let cells = drag::aligned(&drawn, raw);
    let rows = wrap(line, width).into_iter().scan(0, |start, row| {
        let end = (*start + row.width()).min(cells.len());
        let mark = Mark::Written { raw: raw.to_owned(), cells: cells[*start..end].to_vec() };
        *start = end;
        Some((row, Some(mark)))
    });
    rows.collect()
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
pub(super) fn draw_compose(f: &mut Frame, theme: Theme, label: &str, field: &Field, area: Rect) {
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
struct Look<'a> {
    theme: Theme,
    today: DateTime<Utc>,
    me: &'a str,
    ascii: bool,
    width: usize,
}

/// A thread, or my new draft, as the pane shows it: status, notes, my replies; each line tagged
/// with the cursor stop it belongs to.
fn conversation_lines(
    open: &Open,
    conversation: &Conversation,
    index: usize,
    entries: &[Entry],
    look: Look,
) -> Vec<(Option<Entry>, Piece)> {
    let theme = look.theme;
    let stop = |kind: EntryKind| entries.iter().find(|e| e.conversation == index && e.kind == kind).copied();
    let mut lines = vec![];
    if open.lists_every_thread() {
        let first = entries.iter().find(|e| e.conversation == index).copied();
        lines.extend(heading(&open.review, conversation, look).into_iter().map(|line| (first, Piece::Text(line))));
    }
    if let Some(thread) = conversation.thread.as_deref().and_then(|id| open.review.thread(id)) {
        let shown = entries.iter().filter(|e| e.conversation == index && matches!(e.kind, EntryKind::Note(_))).count();
        lines.push((stop(EntryKind::Note(0)), Piece::Text(status(thread, shown, theme))));
        let replaced = thread.first().position.as_ref().map(|p| open.review.text_at(p)).unwrap_or_default();
        for (n, note) in thread.notes.iter().take(shown).enumerate() {
            let entry = stop(EntryKind::Note(n));
            lines.extend(note_lines(note, &replaced, look).into_iter().map(|line| (entry, line)));
        }
    }
    for &draft in &conversation.drafts {
        let entry = stop(EntryKind::Draft(draft));
        let draft = &open.review.drafts[draft];
        if conversation.thread.is_none() {
            lines.push((entry, Piece::Text(Line::from(Span::styled("◇ draft", Style::default().fg(theme.accent))))));
        }
        let position = draft.position.as_ref().or_else(|| {
            let thread = conversation.thread.as_deref().and_then(|id| open.review.thread(id))?;
            thread.first().position.as_ref()
        });
        let replaced = position.map(|p| open.review.text_at(p)).unwrap_or_default();
        lines.extend(draft_lines(draft, &replaced, look).into_iter().map(|line| (entry, line)));
    }
    lines
}

/// Where a conversation hangs, over it in the list of every conversation: `path:line`, then the
/// code of that line, faded, so the reader keeps the context.
fn heading(review: &Review, conversation: &Conversation, look: Look) -> Vec<Line<'static>> {
    let (place, code) = match review.spot(conversation) {
        Spot::Mr => ("on the MR".to_owned(), None),
        Spot::Outdated(anchor) => (format!("{} · outdated", line_name(anchor)), None),
        Spot::Line(anchor) => (line_name(anchor), review.line_text(anchor)),
    };
    let place = Line::from(Span::styled(place, Style::default().fg(look.theme.muted)));
    let code =
        code.map(|text| Line::from(Span::styled(super::ui::truncate(text.trim(), look.width), Style::default().fg(look.theme.faded))));
    std::iter::once(place).chain(code).collect()
}

/// `src/pay/charge.rs:57`, `src/pay/charge.rs:-13` for the old side.
fn line_name(anchor: &Anchor) -> String {
    match anchor.side {
        Side::New => format!("{}:{}", anchor.path, anchor.line),
        Side::Old => format!("{}:-{}", anchor.path, anchor.line),
    }
}

fn status(thread: &Thread, shown: usize, theme: Theme) -> Line<'static> {
    let (text, colour) = if thread.outdated {
        ("◆ outdated".to_owned(), theme.faded)
    } else if thread.resolved && shown < thread.notes.len() {
        let hidden = thread.notes.len() - shown;
        (format!("✓ resolved · {hidden} more note{} · enter unfolds", if hidden == 1 { "" } else { "s" }), theme.faded)
    } else if thread.resolved {
        ("✓ resolved".to_owned(), theme.faded)
    } else if thread.resolvable {
        ("◆ unresolved".to_owned(), theme.warn)
    } else {
        ("· comment".to_owned(), theme.muted)
    };
    Line::from(Span::styled(text, Style::default().fg(colour)))
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

/// My draft: `you · draft ◇`, `unsaved` in danger until the forge holds it.
fn draft_lines(draft: &crate::review::Draft, replaced: &[String], look: Look) -> Vec<Piece> {
    let theme = look.theme;
    let (state, colour) = if draft.id.is_none() { ("unsaved", theme.danger) } else { ("draft", theme.muted) };
    let mut lines = vec![Piece::Text(Line::from(vec![
        Span::styled("you", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" · {state} ◇"), Style::default().fg(colour)),
    ]))];
    lines.extend(body_pieces(&draft.body, replaced, look.width, theme));
    lines
}

/// `replaced` is the text of the lines the thread hangs on, which a suggestion in the note replaces.
fn note_lines(note: &Note, replaced: &[String], look: Look) -> Vec<Piece> {
    let Look { theme, today, me, ascii, width } = look;
    let author = if note.author.username == me { "you".to_owned() } else { note.author.username.clone() };
    let age = short_age((today - note.created_at).to_std().unwrap_or_default());
    let mut lines = vec![Piece::Text(Line::from(vec![
        Span::styled(author.clone(), Style::default().fg(theme.user(&author)).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" · {age}"), Style::default().fg(theme.muted)),
    ]))];
    lines.extend(body_pieces(&note.body, replaced, width, theme));
    if !note.reactions.is_empty() {
        lines.push(Piece::Text(reactions_line(&note.reactions, theme, ascii)));
    }
    lines
}

/// `👍 2  🎉 1` under a note, mine in the accent colour.
pub fn reactions_line(reactions: &[crate::forge::Reaction], theme: Theme, ascii: bool) -> Line<'static> {
    let spans = reactions.iter().enumerate().flat_map(|(i, r)| {
        let face = if ascii { r.emoji.text() } else { r.emoji.glyph() };
        let style = if r.mine { Style::default().fg(theme.accent).add_modifier(Modifier::BOLD) } else { Style::default().fg(theme.muted) };
        let gap = if i == 0 { "" } else { "  " };
        [Span::raw(gap), Span::styled(format!("{face} {}", r.count), style)]
    });
    Line::from(spans.collect::<Vec<_>>())
}

/// Code spans, bullets, quotes and tables `width` columns at most; the rest is the text as written,
/// wrapped by the widget. Pictures read as their `[image: …]` line, for the panes that never draw them.
pub fn body_lines<'a>(body: &str, width: usize, theme: Theme) -> Vec<Line<'a>> {
    body_pieces(body, &[], width, theme)
        .into_iter()
        .map(|piece| match piece {
            Piece::Text(line) | Piece::Written { line, .. } => line,
            Piece::Picture(image) => Line::from(Span::styled(fallback(&image), Style::default().fg(theme.link))),
        })
        .collect()
}

/// A body as pieces, with a suggestion block drawn as a small diff (the `replaced` lines struck
/// as `-`, the suggested ones as `+`, in the diff colours) and each picture under its line.
fn body_pieces(body: &str, replaced: &[String], width: usize, theme: Theme) -> Vec<Piece> {
    #[derive(PartialEq)]
    enum Fence {
        Out,
        Code,
        Suggestion,
    }
    let mut fence = Fence::Out;
    let mut lines = vec![];
    let mut raws = body.lines().peekable();
    while let Some(raw) = raws.next() {
        let opener = raw.trim_start();
        if opener.starts_with("```") {
            fence = match fence {
                Fence::Out if opener.starts_with("```suggestion") => {
                    let old = Style::default().fg(theme.danger);
                    lines.extend(replaced.iter().map(|text| Piece::Text(Line::from(Span::styled(format!("- {text}"), old)))));
                    Fence::Suggestion
                }
                Fence::Out => Fence::Code,
                Fence::Code | Fence::Suggestion => Fence::Out,
            };
            continue;
        }
        match fence {
            Fence::Code => {
                lines.push(written(Line::from(Span::styled(format!("  {raw}"), Style::default().fg(theme.code))), raw));
                continue;
            }
            Fence::Suggestion => {
                lines.push(written(Line::from(Span::styled(format!("+ {raw}"), Style::default().fg(theme.success))), raw));
                continue;
            }
            Fence::Out => {}
        }
        if !opener.starts_with('|') {
            lines.extend(prose(raw, theme));
            continue;
        }
        let run: Vec<&str> = std::iter::once(raw).chain(std::iter::from_fn(|| raws.next_if(|l| l.trim_start().starts_with('|')))).collect();
        match table_lines(&run, width, theme) {
            Some(table) => lines.extend(table.into_iter().zip(&run).map(|(line, raw)| written(line, raw))),
            None => lines.extend(run.iter().flat_map(|raw| prose(raw, theme))),
        }
    }
    lines
}

/// A line outside fences and tables, each picture under it.
fn prose(raw: &str, theme: Theme) -> Vec<Piece> {
    let (text, pictures) = image::split(raw);
    let line = (pictures.is_empty() || !text.trim().is_empty()).then(|| written(text_line(&text, theme), raw));
    line.into_iter().chain(pictures.into_iter().map(Piece::Picture)).collect()
}

fn written(line: Line<'static>, raw: &str) -> Piece {
    Piece::Written { line, raw: raw.to_owned() }
}

/// One line of prose: bullets, quotes and code spans.
fn text_line(raw: &str, theme: Theme) -> Line<'static> {
    match raw.trim_start() {
        rest if rest.starts_with("- ") || rest.starts_with("* ") => Line::from(inline(&format!("• {}", &rest[2..]), theme)),
        rest if rest.starts_with("> ") => Line::from(Span::styled(format!("▏{}", &rest[2..]), Style::default().fg(theme.muted))),
        _ => Line::from(inline(raw, theme)),
    }
}

pub(super) fn inline<'a>(text: &str, theme: Theme) -> Vec<Span<'a>> {
    text.split('`')
        .enumerate()
        .filter(|(_, part)| !part.is_empty())
        .map(|(i, part)| {
            if i % 2 == 1 {
                Span::styled(part.to_owned(), Style::default().fg(theme.code).bg(theme.surface))
            } else {
                Span::raw(part.to_owned())
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn text(lines: &[Line]) -> Vec<String> {
        lines.iter().map(|l| l.spans.iter().map(|s| s.content.to_string()).collect()).collect()
    }

    /// Pieces as the lines a pane without pictures shows.
    fn texts(pieces: Vec<Piece>) -> Vec<Line<'static>> {
        pieces
            .into_iter()
            .map(|piece| match piece {
                Piece::Text(line) | Piece::Written { line, .. } => line,
                Piece::Picture(image) => Line::from(fallback(&image)),
            })
            .collect()
    }

    #[test]
    fn a_picture_leaves_its_line_and_sits_under_it() {
        let body = "Before:\n![the chart](/uploads/ab12/chart.png) broke\n<img alt=\"after\" src=\"https://x/a.png\">\n```\n![not](https://x/code.png)\n```";
        let pieces = body_pieces(body, &[], 80, Theme::default());
        let shape: Vec<String> = pieces
            .iter()
            .map(|p| match p {
                Piece::Text(line) | Piece::Written { line, .. } => {
                    format!("text {}", line.spans.iter().map(|s| s.content.to_string()).collect::<String>())
                }
                Piece::Picture(image) => format!("picture {}", image.url),
            })
            .collect();
        assert_eq!(
            shape,
            [
                "text Before:",
                "text  broke",
                "picture /uploads/ab12/chart.png",
                "picture https://x/a.png",
                "text   ![not](https://x/code.png)"
            ]
        );
    }

    #[test]
    fn a_picture_that_cannot_be_drawn_reads_as_its_alt_and_links_to_the_forge() {
        let thumbs = Thumbs::off();
        let web = |url: &str| crate::forge::image::web_url("gitlab.com", "acme/widgets", url);
        let image = Image { alt: "the chart".into(), url: "/uploads/ab12/chart.png".into() };
        let rows = lay_out(Piece::Picture(image), &thumbs, 40, Theme::default(), &web);
        assert_eq!(text(&[rows[0].0.clone()]), ["[image: the chart]"]);
        let Some(Mark::Link { url, .. }) = &rows[0].1 else { panic!("a link") };
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
        assert!(matches!(&rows[0].1, Some(Mark::Picture { size, .. }) if size.width == 20 && size.height == 5));
        assert!(rows[1..].iter().all(|(_, mark)| mark.is_none()));
        thumbs.wanted(["https://x/b.png".to_owned()]);
        let loading =
            lay_out(Piece::Picture(Image { url: "https://x/b.png".into(), ..image }), &thumbs, 40, Theme::default(), &|u: &str| {
                u.to_owned()
            });
        assert_eq!(text(&[loading[0].0.clone()]), ["… loading image"]);
    }

    #[test]
    fn bodies_keep_code_bullets_and_quotes() {
        let body = "Use `Key::from` here.\n- one\n> said\n```\nlet x = 1;\n```";
        let lines = text(&body_lines(body, 80, Theme::default()));
        assert_eq!(lines, ["Use Key::from here.", "• one", "▏said", "  let x = 1;"]);
    }

    #[test]
    fn a_table_is_drawn_and_pipes_without_a_delimiter_row_stay_text() {
        let body = "Counts:\n| Name | Count |\n| --- | ---: |\n| alpha | 3 |\nthen\n| a | b |";
        let lines = text(&body_lines(body, 80, Theme::default()));
        assert_eq!(lines, ["Counts:", "Name  │ Count", "──────┼──────", "alpha │     3", "then", "| a | b |"]);
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
    fn a_suggestion_reads_as_a_small_diff() {
        let body = "Try this:\n```suggestion:-0+0\nlet client = Client::default();\n```\nthanks";
        let lines = text(&texts(body_pieces(body, &["let client = Client::new();".to_owned()], 80, Theme::default())));
        assert_eq!(lines, ["Try this:", "- let client = Client::new();", "+ let client = Client::default();", "thanks"]);
        assert_eq!(text(&body_lines("```rust\nlet x = 1;\n```", 80, Theme::default())), ["  let x = 1;"], "other fences stay code");
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
}
