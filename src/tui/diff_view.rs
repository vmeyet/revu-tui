//! The review pane: rows from `Review::rows()` turned into styled lines, only for the visible window.
use super::app::{App, Focus, Open};
use super::theme::Theme;
use super::ui::{draw_empty, pane, settle_scroll, short_age, spinner, truncate};
use crate::diff::words::{Segment, same_but_whitespace, segments};
use crate::diff::{Line as DiffLine, LineKind};
use crate::forge::Kind;
use crate::review::{File, FileKind, Mark, Marker, Markers, Place, Review, Row, Side};
use crate::syntax::Token;
use chrono::{DateTime, Utc};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use std::ops::Range;
use unicode_width::UnicodeWidthStr;

const GUTTER_W: usize = 4;
/// The glyph and count column before the line numbers.
const ANCHOR_W: usize = 2;
const TAB: &str = "→   ";
const INDENT: &str = "   ";
const MIN_BRANCH_W: usize = 12;

pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
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
    let block = pane(theme, &title, app.focus == Focus::Review);
    if let Some(open) = &app.open {
        app.links.push(title_link(&open.review, app.hosts.kind_of(&open.key), area));
    }
    let inner = block.inner(area);
    f.render_widget(block, area);
    if app.open.is_none() {
        let lines: &[&str] = match app.opening {
            Some(_) => &[spinner(app.now.duration_since(app.started)), "", "fetching the merge request"],
            None => &["open a merge request", "to read its diff here"],
        };
        return draw_empty(f, theme, inner, lines);
    }
    let today = app.today;
    let folded = app.header_folded;
    let wrap = app.wrap;
    let Some(open) = app.open.as_mut() else { return };
    let anchors = Anchors { markers: open.review.markers(), stretch: focused_range(open) };
    let header = if folded { vec![folded_header(open, theme)] } else { header_lines(open, theme, today, inner.width as usize) };
    let body = Rect { y: inner.y + header.len() as u16, height: inner.height.saturating_sub(header.len() as u16), ..inner };
    let height = body.height as usize;
    let width = body.width as usize;
    let render = |open: &Open, i: usize| -> Vec<Line<'static>> {
        let row = &open.rows[i];
        let (selected, in_range) = (i == open.selected, open.is_selected(i));
        if wrap && matches!(row, Row::Line { .. } | Row::Pair { .. } | Row::Context { .. }) {
            wrap_row(row_line(&open.review, &anchors, row, selected, in_range, UNCUT, theme), width, WRAP_INDENT)
        } else {
            vec![row_line(&open.review, &anchors, row, selected, in_range, width, theme)]
        }
    };
    open.scroll = settle_scroll(open.scroll, open.selected, height);
    while wrap && open.scroll < open.selected && (open.scroll..=open.selected).map(|i| render(open, i).len()).sum::<usize>() > height {
        open.scroll += 1;
    }
    let lines: Vec<Line> = (open.scroll..open.rows.len()).flat_map(|i| render(open, i)).take(height).collect();
    let pipeline = if folded { None } else { pipeline_link(&open.review, &header, inner) };
    f.render_widget(Paragraph::new(header), inner);
    f.render_widget(Paragraph::new(lines), body);
    app.links.extend(pipeline);
}

/// The pipeline word ends the header's first row; it links to the run on the forge.
fn pipeline_link(review: &Review, header: &[Line], area: Rect) -> Option<super::ui::Link> {
    let url = review.mr.pipeline.as_ref()?.web_url.clone()?;
    let spans = &header.first()?.spans;
    let (last, before) = spans.split_last()?;
    let x = before.iter().map(Span::width).sum::<usize>();
    let x = area.x + u16::try_from(x).ok()?;
    (x + u16::try_from(last.width()).ok()? <= area.right()).then(|| super::ui::Link { x, y: area.y, text: last.content.to_string(), url })
}

/// The width a line is drawn at before `w` cuts it into screen rows; wide enough for any real line.
const UNCUT: usize = 4_096;
/// Continuation rows start under the text: past the cursor bar, both gutters and the sign.
const WRAP_INDENT: usize = 1 + ANCHOR_W + GUTTER_W * 2 + 3;

/// One long line as several screen rows of `width`: continuation rows indented under the text,
/// every row padded with the line's fill so a changed line stays one coloured block.
fn wrap_row(line: Line<'static>, width: usize, indent: usize) -> Vec<Line<'static>> {
    let mut spans = line.spans;
    let fill = trailing_fill(&mut spans);
    let mut rows: Vec<Vec<Span<'static>>> = vec![];
    let mut row: Vec<Span<'static>> = vec![];
    let mut used = 0;
    for span in spans {
        let mut piece = String::new();
        for c in span.content.chars() {
            let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
            if used + w > width && used > indent {
                row.push(Span::styled(std::mem::take(&mut piece), span.style));
                rows.push(std::mem::replace(&mut row, vec![Span::styled(" ".repeat(indent), fill.unwrap_or_default())]));
                used = indent;
            }
            piece.push(c);
            used += w;
        }
        row.push(Span::styled(piece, span.style));
    }
    rows.push(row);
    rows.into_iter().map(|row| padded(row, width, fill)).collect()
}

/// Drops the blank padding a changed line ends with, and gives back its style to pad each row again.
fn trailing_fill(spans: &mut Vec<Span<'static>>) -> Option<Style> {
    let last = spans.last().filter(|s| !s.content.is_empty() && s.content.chars().all(|c| c == ' '))?;
    let style = last.style;
    spans.pop();
    Some(style)
}

fn padded(mut row: Vec<Span<'static>>, width: usize, fill: Option<Style>) -> Line<'static> {
    let used: usize = row.iter().map(Span::width).sum();
    if let (Some(style), true) = (fill, used < width) {
        row.push(Span::styled(" ".repeat(width - used), style));
    }
    Line::from(row)
}

/// The `group/project!42` at the start of the pane title, one cell past the border and its space.
pub fn title_link(review: &Review, kind: Kind, area: Rect) -> super::ui::Link {
    super::ui::Link { x: area.x + 2, y: area.y, text: mr_ref(review, kind), url: review.mr.web_url.clone() }
}

/// `group/project!42` on GitLab, `owner/repo#42` on GitHub.
pub fn mr_ref(review: &Review, kind: Kind) -> String {
    format!("{}{}{}", review.mr.project, kind.sigil(), review.mr.number)
}

fn header_lines<'a>(open: &Open, theme: Theme, today: DateTime<Utc>, width: usize) -> Vec<Line<'a>> {
    let mr = &open.review.mr;
    let muted = Style::default().fg(theme.muted);
    let dot = || Span::styled(" · ", Style::default().fg(theme.faded));
    let (adds, dels) = open.review.files.iter().fold((0, 0), |(a, d), f| (a + f.additions, d + f.deletions));
    let age = short_age((today - mr.updated_at).to_std().unwrap_or_default());
    let pipeline = mr.pipeline.as_ref().map(|p| p.status.clone()).unwrap_or_default();
    let (glyph, colour) = pipeline_glyph(&pipeline, theme);
    let branch_room = width.saturating_sub(40);
    let branches = format!("{} → {}", mr.source_branch, mr.target_branch);
    let mut first = vec![Span::raw(" "), Span::styled(mr.author.username.clone(), Style::default().fg(theme.user(&mr.author.username)))];
    if branch_room >= MIN_BRANCH_W {
        first.push(dot());
        first.push(Span::styled(truncate(&branches, branch_room), muted));
    }
    first.extend([
        dot(),
        Span::styled(age, muted),
        dot(),
        Span::styled(format!("+{adds}"), Style::default().fg(theme.success)),
        Span::raw(" "),
        Span::styled(format!("−{dels}"), Style::default().fg(theme.danger)),
        dot(),
        Span::styled(format!("{} files", open.review.files.len()), muted),
        dot(),
        Span::styled(format!("{glyph} {}", pipeline.to_lowercase()), Style::default().fg(colour)),
    ]);
    let first = Line::from(first);
    let approvals = &mr.approvals;
    let wanted = approvals.approved_by.len() as u32 + approvals.approvals_left;
    let threads = open.review.threads.len();
    let unresolved = open.review.unresolved();
    let mut second = vec![Span::raw("  ")];
    if wanted > 0 {
        second.push(Span::styled(
            format!("{} of {wanted} approvals", approvals.approved_by.len()),
            Style::default().fg(if approvals.approved { theme.success } else { theme.muted }),
        ));
    }
    if threads > 0 {
        if second.len() > 1 {
            second.push(dot());
        }
        second.push(Span::styled(format!("{threads} threads"), muted));
        if unresolved > 0 {
            second.push(Span::styled(format!(", {unresolved} unresolved"), Style::default().fg(theme.warn)));
        }
    }
    if mr.conflicts {
        second.push(dot());
        second.push(Span::styled("conflicts", Style::default().fg(theme.danger)));
    }
    if second.len() == 1 {
        return vec![first];
    }
    vec![first, Line::from(second)]
}

/// `zh`: the header on one row, the author, the size, the pipeline and what is still open.
fn folded_header<'a>(open: &Open, theme: Theme) -> Line<'a> {
    let mr = &open.review.mr;
    let dot = || Span::styled(" · ", Style::default().fg(theme.faded));
    let (adds, dels) = open.review.files.iter().fold((0, 0), |(a, d), f| (a + f.additions, d + f.deletions));
    let pipeline = mr.pipeline.as_ref().map(|p| p.status.clone()).unwrap_or_default();
    let (glyph, colour) = pipeline_glyph(&pipeline, theme);
    let mut spans = vec![
        Span::styled("▸ ", Style::default().fg(theme.faded)),
        Span::styled(mr.author.username.clone(), Style::default().fg(theme.user(&mr.author.username))),
        dot(),
        Span::styled(format!("+{adds}"), Style::default().fg(theme.success)),
        Span::raw(" "),
        Span::styled(format!("−{dels}"), Style::default().fg(theme.danger)),
    ];
    if !glyph.is_empty() {
        spans.extend([dot(), Span::styled(glyph, Style::default().fg(colour))]);
    }
    let unresolved = open.review.unresolved();
    if unresolved > 0 {
        spans.extend([dot(), Span::styled(format!("{unresolved} unresolved"), Style::default().fg(theme.warn))]);
    }
    Line::from(spans)
}

fn pipeline_glyph(status: &str, theme: Theme) -> (&'static str, ratatui::style::Color) {
    match status.to_ascii_lowercase().as_str() {
        "success" => ("✓", theme.success),
        "failed" => ("✗", theme.danger),
        "running" | "pending" | "created" | "preparing" | "waiting_for_resource" => ("⠋", theme.muted),
        "" => ("", theme.faded),
        _ => ("○", theme.muted),
    }
}

fn row_line<'a>(review: &Review, anchors: &Anchors, row: &Row, selected: bool, in_range: bool, width: usize, theme: Theme) -> Line<'a> {
    let bar = Span::styled(if selected || in_range { "▎" } else { " " }, Style::default().fg(theme.accent));
    let mut spans = vec![bar];
    let body = width.saturating_sub(1);
    if matches!(row, Row::Line { .. } | Row::Pair { .. } | Row::Context { .. }) {
        let marker = review.marker_of(&anchors.markers, row);
        let spans_range = marker.is_none() && anchors.stretch.is_some_and(|stretch| stretch.covers(review, row));
        spans.extend(if spans_range { range_spans(theme) } else { anchor_spans(marker, theme) });
    }
    let body = if spans.len() > 1 { body.saturating_sub(ANCHOR_W) } else { body };
    match row {
        Row::Gap => {}
        Row::Header => spans.extend(on_the_mr_spans(review, theme)),
        Row::File { index, open } => spans.extend(file_spans(review, &review.files[*index], *open, body, theme)),
        Row::Hunk { file, index, open } => spans.extend(hunk_spans(&review.files[*file], *index, *open, body, theme)),
        Row::Line { file, hunk, index } => {
            let line = &review.files[*file].hunks[*hunk].lines[*index];
            spans.extend(line_spans(line, review.files[*file].spans(*hunk, *index), selected || in_range, body, theme));
        }
        Row::Pair { file, hunk, removed, added } => {
            let lines = &review.files[*file].hunks[*hunk].lines;
            let code = review.files[*file].spans(*hunk, *added);
            let (old, new) = (&lines[*removed], &lines[*added]);
            if review.quiet_whitespace && same_but_whitespace(&old.text, &new.text) {
                spans.extend(quiet_spans(old, new, code, selected || in_range, body, theme));
            } else {
                spans.extend(pair_spans(old, new, code, selected || in_range, body, theme));
            }
        }
        Row::Context { file, old, new, .. } => {
            let path = &review.files[*file].new_path;
            let text = review.context.texts.get(path).and_then(|t| t.get(*new as usize - 1)).cloned().unwrap_or_default();
            let line = DiffLine { kind: LineKind::Context, old: Some(*old), new: Some(*new), text, words: vec![], no_newline: false };
            spans.extend(line_spans(&line, &[], selected || in_range, body, theme));
        }
    }
    Line::from(spans)
}

/// What the anchor column draws from: the marks of every line, and the range comment the pane is on.
struct Anchors {
    markers: Markers,
    stretch: Option<Stretch>,
}

/// The lines of a range comment before its last, which carries the mark.
#[derive(Clone, Copy)]
struct Stretch {
    file: usize,
    side: Side,
    first: u32,
    last: u32,
}

impl Stretch {
    fn covers(self, review: &Review, row: &Row) -> bool {
        let Some(Place::Line { file, new, old }) = review.place_of(row) else { return false };
        let number = if self.side == Side::New { new } else { old };
        file == self.file && number.is_some_and(|n| (self.first..self.last).contains(&n))
    }
}

/// The range of the thread or draft under the pane's cursor, when it spans several lines.
fn focused_range(open: &Open) -> Option<Stretch> {
    let position = match open.focused_thread() {
        Some(id) => open.review.thread(&id)?.first().position.clone()?,
        None => open.review.drafts.get(open.focused_draft()?)?.position.clone()?,
    };
    let side = position.line.side();
    let number = |line: crate::forge::LineRef| if side == Side::New { line.new } else { line.old };
    let (first, last) = (number(position.start?)?, number(position.line)?);
    let file = open.review.files.iter().position(|f| f.new_path == position.new_path || f.old_path == position.old_path)?;
    Some(Stretch { file, side, first, last })
}

fn range_spans<'a>(theme: Theme) -> Vec<Span<'a>> {
    vec![Span::styled("│", Style::default().fg(theme.accent)), Span::raw(" ")]
}

/// The anchor column: the most pressing mark of the line, then how many conversations it holds when more than one.
fn anchor_spans<'a>(marker: Option<Marker>, theme: Theme) -> Vec<Span<'a>> {
    let Some(marker) = marker else { return vec![Span::raw(" ".repeat(ANCHOR_W))] };
    let (glyph, colour) = match marker.mark {
        Mark::Unresolved => ("◆", theme.warn),
        Mark::Draft if marker.unsaved => ("◇", theme.danger),
        Mark::Draft => ("◇", theme.accent),
        Mark::Resolved => ("✓", theme.faded),
    };
    let count = match marker.count {
        0 | 1 => " ".to_owned(),
        n @ 2..=9 => n.to_string(),
        _ => "+".to_owned(),
    };
    vec![Span::styled(glyph, Style::default().fg(colour)), Span::styled(count, Style::default().fg(colour))]
}

/// The row under the header: the conversations on the MR itself, `enter` opens them.
fn on_the_mr_spans<'a>(review: &Review, theme: Theme) -> Vec<Span<'a>> {
    let listed = review.conversations(&Place::Mr);
    if listed.is_empty() {
        return vec![];
    }
    let unresolved = listed.iter().filter_map(|c| c.thread.as_deref().and_then(|id| review.thread(id))).any(|t| !t.resolved);
    let (glyph, colour) = if unresolved { ("◆", theme.warn) } else { ("◇", theme.accent) };
    vec![
        Span::styled(format!("{glyph} {} on the MR", listed.len()), Style::default().fg(colour)),
        Span::styled("  enter opens", Style::default().fg(theme.faded)),
    ]
}

fn file_spans<'a>(review: &Review, file: &File, open: bool, width: usize, theme: Theme) -> Vec<Span<'a>> {
    let mark = if open { "▾ " } else { "▸ " };
    let (dir, base) = match file.new_path.rsplit_once('/') {
        Some((dir, base)) => (format!("{dir}/"), base.to_owned()),
        None => (String::new(), file.new_path.clone()),
    };
    let counts = format!("+{} −{}", file.additions, file.deletions);
    let in_file = |a: &crate::review::Anchor| a.path == file.new_path || a.path == file.old_path;
    let threads = review.threads.iter().filter(|t| !t.outdated && t.anchor.as_ref().is_some_and(in_file)).count();
    let drafts = review.drafts.iter().filter(|d| d.reply_to.is_none() && d.anchor.as_ref().is_some_and(in_file)).count();
    let outdated = review.outdated(&file.new_path).len();
    let anchors = [
        (threads > 0).then(|| format!("  ◆{threads}")),
        (drafts > 0).then(|| format!(" ◇{drafts}")),
        (outdated > 0).then(|| format!(" · {outdated} outdated")),
    ]
    .into_iter()
    .flatten()
    .collect::<String>();
    let state = file_state(file, review, open);
    let tail_w = counts.width() + anchors.width() + state.as_ref().map_or(0, |s| s.width() + 2);
    let name_room = width.saturating_sub(mark.width() + tail_w + 2);
    let name = truncate(&format!("{dir}{base}"), name_room);
    let (dir, base) = match name.rsplit_once('/') {
        Some((d, b)) => (format!("{d}/"), b.to_owned()),
        None => (String::new(), name),
    };
    let pad = name_room.saturating_sub(dir.width() + base.width()) + 2;
    let mut spans = vec![
        Span::styled(mark, Style::default().fg(theme.faded)),
        Span::styled(dir, Style::default().fg(theme.muted)),
        Span::styled(base, Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" ".repeat(pad)),
        Span::styled(format!("+{}", file.additions), Style::default().fg(theme.success)),
        Span::raw(" "),
        Span::styled(format!("−{}", file.deletions), Style::default().fg(theme.danger)),
        Span::styled(anchors, Style::default().fg(theme.warn)),
    ];
    if let Some(state) = state {
        spans.push(Span::styled(format!("  {state}"), Style::default().fg(theme.faded)));
    }
    spans
}

fn file_state(file: &File, review: &Review, open: bool) -> Option<String> {
    if review.viewed.contains(&file.new_path) {
        return Some("viewed".into());
    }
    if file.binary {
        return Some("binary".into());
    }
    if file.too_large {
        return Some("too large".into());
    }
    match file.kind {
        FileKind::Renamed => Some(format!("renamed from {}", file.old_path)),
        FileKind::Deleted => Some("deleted".into()),
        FileKind::Added => Some("added".into()),
        FileKind::Mode => Some("mode changed".into()),
        FileKind::Modified if !open => Some("folded".into()),
        FileKind::Modified => None,
    }
}

fn hunk_spans<'a>(file: &File, index: usize, open: bool, width: usize, theme: Theme) -> Vec<Span<'a>> {
    let hunk = &file.hunks[index];
    let mark = if open { "▾ " } else { "▸ " };
    let (range, context) = match hunk.header.find(" @@") {
        Some(at) => (hunk.header[..at + 3].to_owned(), hunk.header[at + 3..].trim().to_owned()),
        None => (hunk.header.clone(), String::new()),
    };
    let count = if open { String::new() } else { format!("  ({} lines)", hunk.lines.len()) };
    let room = width.saturating_sub(INDENT.len() + mark.len() + range.width() + count.width() + 1);
    vec![
        Span::styled(format!("{INDENT}{mark}"), Style::default().fg(theme.faded)),
        Span::styled(format!("{range} "), Style::default().fg(theme.muted)),
        Span::raw(truncate(&context, room)),
        Span::styled(count, Style::default().fg(theme.faded)),
    ]
}

/// How one kind of changed line paints: sign, text colour, fills, and the colour of its meaning.
struct Paint {
    sign: &'static str,
    text: Color,
    fill: Option<Color>,
    word: Option<Color>,
    accent: Color,
}

fn paint(kind: LineKind, theme: Theme) -> Option<Paint> {
    match kind {
        LineKind::Added => {
            Some(Paint { sign: "+", text: theme.added, fill: theme.added_fill, word: theme.added_word, accent: theme.success })
        }
        LineKind::Removed => {
            Some(Paint { sign: "-", text: theme.removed, fill: theme.removed_fill, word: theme.removed_word, accent: theme.danger })
        }
        LineKind::Context => None,
    }
}

/// A changed line is filled edge to edge; without a fill, the text itself carries the colour
/// and changed words go bold, so every theme reads on every terminal.
fn line_spans<'a>(line: &DiffLine, code: &[(Range<usize>, Token)], selected: bool, width: usize, theme: Theme) -> Vec<Span<'a>> {
    let gutter_colour = if selected { theme.muted } else { theme.faded };
    let number = |n: Option<u32>| n.map_or_else(|| " ".repeat(GUTTER_W), |n| format!("{n:>GUTTER_W$}"));
    let paint = paint(line.kind, theme);
    let base = match &paint {
        Some(p) => p.fill.map_or(Style::default().fg(p.text), |fill| Style::default().fg(p.text).bg(fill)),
        None => Style::default(),
    };
    let sign = paint.as_ref().map_or_else(|| Span::styled(" ", base), |p| Span::styled(p.sign, base.fg(p.accent)));
    let word = paint.as_ref().map(|p| match p.word {
        Some(fill) => base.fg(p.accent).bg(fill),
        None => base.add_modifier(Modifier::BOLD),
    });
    let mut spans = vec![Span::styled(format!("{} {} ", number(line.old), number(line.new)), base.fg(gutter_colour)), sign];
    let room = width.saturating_sub(GUTTER_W * 2 + 3);
    let text = text_spans(line, code, room, base, word, theme);
    let used: usize = text.iter().map(Span::width).sum();
    spans.extend(text);
    if paint.is_some() && used < room {
        spans.push(Span::styled(" ".repeat(room - used), base));
    }
    spans
}

/// The text cut into pieces wherever a changed word or a syntax colour starts or stops: syntax
/// sets the text colour, the diff keeps the fills; tabs made visible, trailing spaces marked, cut to `room`.
fn text_spans<'a>(
    line: &DiffLine,
    code: &[(Range<usize>, Token)],
    room: usize,
    base: Style,
    word: Option<Style>,
    theme: Theme,
) -> Vec<Span<'a>> {
    let changed = line.kind != LineKind::Context;
    let trimmed = line.text.trim_end_matches([' ', '\t']);
    let trailing = line.text.len() - trimmed.len();
    let mut edges: Vec<usize> = line.words.iter().chain(code.iter().map(|(range, _)| range)).flat_map(|r| [r.start, r.end]).collect();
    edges.extend([0, trimmed.len()]);
    edges.retain(|&edge| edge <= trimmed.len() && trimmed.is_char_boundary(edge));
    edges.sort_unstable();
    edges.dedup();
    let mut parts: Vec<(&str, Style)> = vec![];
    for piece in edges.windows(2) {
        let (from, to) = (piece[0], piece[1]);
        let in_word = line.words.iter().any(|w| w.start <= from && from < w.end);
        let style = if in_word { word.unwrap_or(base) } else { base };
        parts.push((&trimmed[from..to], with_syntax(style, code, from, theme)));
    }
    let dots = "·".repeat(trailing);
    if changed && trailing > 0 {
        parts.push((&dots, base.fg(theme.warn)));
    }
    fit(parts, room)
}

/// `style` with the syntax colour of the token covering byte `at`, if any.
fn with_syntax(style: Style, code: &[(Range<usize>, Token)], at: usize, theme: Theme) -> Style {
    code.iter().find(|(range, _)| range.start <= at && at < range.end).map_or(style, |(_, token)| style.fg(theme.syntax.colour(*token)))
}

/// A removed line and its added twin on one row: the kept text in its syntax colours, each old
/// word struck through in the removed colours, then its replacement in the added colours.
/// `code` is the added line's syntax, so offsets follow the new text: kept and new pieces advance it.
fn pair_spans<'a>(
    old: &DiffLine,
    new: &DiffLine,
    code: &[(Range<usize>, Token)],
    selected: bool,
    width: usize,
    theme: Theme,
) -> Vec<Span<'a>> {
    let gutter = Style::default().fg(if selected { theme.muted } else { theme.faded });
    let number = |n: Option<u32>| n.map_or_else(|| " ".repeat(GUTTER_W), |n| format!("{n:>GUTTER_W$}"));
    let with_fill = |style: Style, fill: Option<Color>| fill.map_or(style, |f| style.bg(f));
    let dropped = with_fill(Style::default().fg(theme.danger).add_modifier(Modifier::CROSSED_OUT), theme.removed_word);
    let added = with_fill(Style::default().fg(theme.success), theme.added_word);
    let parts = segments(&old.text, &new.text);
    let mut styled: Vec<(&str, Style)> = vec![];
    let mut at = 0;
    for part in &parts {
        match part {
            Segment::Same(text) => {
                styled.extend(coloured(text, at, code, theme));
                at += text.len();
            }
            Segment::Old(text) => styled.push((text.as_str(), dropped)),
            Segment::New(text) => {
                styled.push((text.as_str(), added));
                at += text.len();
            }
        }
    }
    let mut spans =
        vec![Span::styled(format!("{} {} ", number(old.old), number(new.new)), gutter), Span::styled("~", Style::default().fg(theme.warn))];
    spans.extend(fit(styled, width.saturating_sub(GUTTER_W * 2 + 3)));
    spans
}

/// `text`, found at byte `at` of its line, cut wherever a syntax token starts or stops.
fn coloured<'t>(text: &'t str, at: usize, code: &[(Range<usize>, Token)], theme: Theme) -> Vec<(&'t str, Style)> {
    let end = at + text.len();
    let mut edges: Vec<usize> = code.iter().flat_map(|(r, _)| [r.start, r.end]).filter(|&e| at < e && e < end).map(|e| e - at).collect();
    edges.extend([0, text.len()]);
    edges.retain(|&edge| text.is_char_boundary(edge));
    edges.sort_unstable();
    edges.dedup();
    edges.windows(2).map(|w| (&text[w[0]..w[1]], with_syntax(Style::default(), code, at + w[0], theme))).collect()
}

/// A line that changed only in whitespace, under `W`: read as context, both numbers, a `≈` for a sign.
fn quiet_spans<'a>(
    old: &DiffLine,
    new: &DiffLine,
    code: &[(Range<usize>, Token)],
    selected: bool,
    width: usize,
    theme: Theme,
) -> Vec<Span<'a>> {
    let context = DiffLine { kind: LineKind::Context, old: old.old, words: vec![], ..new.clone() };
    let mut spans = line_spans(&context, code, selected, width, theme);
    if let Some(sign) = spans.get_mut(1) {
        *sign = Span::styled("≈", Style::default().fg(theme.faded));
    }
    spans
}

/// Styled pieces laid end to end, tabs made visible, cut to `room` columns.
fn fit<'a>(parts: Vec<(&str, Style)>, room: usize) -> Vec<Span<'a>> {
    let mut spans = vec![];
    let mut used = 0;
    for (text, style) in parts {
        if used >= room {
            break;
        }
        let cut = truncate(&text.replace('\t', TAB), room - used);
        used += cut.width();
        spans.push(Span::styled(cut, style));
    }
    spans
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::diff;

    fn spans_text(spans: &[Span]) -> String {
        spans.iter().map(|s| s.content.to_string()).collect()
    }

    fn cart() -> crate::review::File {
        crate::review::File::from_diff(&crate::forge::DiffFile {
            diff: include_str!("../review/fixtures/cart.ts.diff").to_owned(),
            old_path: "src/cart.ts".into(),
            new_path: "src/cart.ts".into(),
            ..crate::forge::DiffFile::default()
        })
    }

    /// Each piece of a painted line as `fg/bg text`, so a snapshot shows the colours.
    #[test]
    fn an_inline_row_keeps_syntax_on_the_kept_text_and_diff_colours_on_the_words() {
        let hunk = &diff::parse("@@ -1 +1 @@\n-const total = 1;\n+const total = 2;\n")[0];
        let (old, new) = (&hunk.lines[0], &hunk.lines[1]);
        let code = [(0..5, Token::Keyword), (14..15, Token::Number)];
        let theme = Theme::named("tokyonight").unwrap();
        let spans = pair_spans(old, new, &code, false, 80, theme);
        let style_of = |text: &str| spans.iter().find(|s| s.content == text).map(|s| s.style).unwrap();
        assert_eq!(spans_text(&spans).trim_end(), "   1    1 ~const total = 1;2;");
        assert_eq!(style_of("const").fg, Some(theme.syntax.colour(Token::Keyword)), "kept text takes its syntax colour");
        assert_eq!(style_of(" total = ").fg, None);
        assert_eq!(style_of("1;").fg, Some(theme.danger), "the old word keeps the diff colour");
        assert!(style_of("1;").add_modifier.contains(Modifier::CROSSED_OUT));
        assert_eq!(style_of("2;").fg, Some(theme.success), "the new word keeps the diff colour");
    }

    fn painted(spans: &[Span]) -> String {
        spans
            .iter()
            .filter(|s| !s.content.trim().is_empty())
            .map(|s| format!("{:?}/{:?} {:?}", s.style.fg, s.style.bg, s.content))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn syntax_sets_the_text_and_the_diff_keeps_the_fills() {
        let file = cart();
        let theme = Theme::named("tokyonight").unwrap();
        let spans = line_spans(&file.hunks[0].lines[1], file.spans(0, 1), false, 100, theme);
        let keyword = spans.iter().find(|s| s.content == "const").unwrap();
        assert_eq!((keyword.style.fg, keyword.style.bg), (Some(theme.syntax.keyword), theme.removed_fill));
        let comment = spans.iter().find(|s| s.content.contains("// cents")).unwrap();
        assert_eq!((comment.style.fg, comment.style.bg), (Some(theme.syntax.comment), theme.removed_fill));
        let changed = line_spans(&file.hunks[0].lines[2], file.spans(0, 2), false, 100, theme);
        let word = changed.iter().find(|s| s.content.contains("quantity")).unwrap();
        assert_eq!(word.style.bg, theme.added_word, "a changed word keeps its stronger fill under the syntax colour");
    }

    #[test]
    fn on_an_unknown_ground_syntax_colours_the_text_and_only_the_sign_says_removed() {
        let file = cart();
        let theme = Theme::default();
        let spans = line_spans(&file.hunks[0].lines[1], file.spans(0, 1), false, 100, theme);
        assert_eq!(spans[1].content, "-");
        assert_eq!(spans[1].style.fg, Some(theme.danger));
        let keyword = spans.iter().find(|s| s.content == "const").unwrap();
        assert_eq!((keyword.style.fg, keyword.style.bg), (Some(theme.syntax.keyword), None));
    }

    #[test]
    fn snapshot_highlighted_typescript_hunk() {
        let file = cart();
        let theme = Theme::named("tokyonight").unwrap();
        let lines: Vec<String> = file.hunks[0]
            .lines
            .iter()
            .enumerate()
            .map(|(i, line)| painted(&line_spans(line, file.spans(0, i), false, 100, theme)))
            .collect();
        insta::assert_snapshot!("highlighted_typescript_hunk", lines.join("\n---\n"));
    }

    #[test]
    fn line_shows_both_gutters_the_sign_and_visible_tabs() {
        let hunk = &diff::parse("@@ -1,2 +1,2 @@\n \tkeep  \n-\told\n+\tnew  \n")[0];
        let context = spans_text(&line_spans(&hunk.lines[0], &[], false, 60, Theme::default()));
        assert_eq!(context, "   1    1  →   keep");
        let added = spans_text(&line_spans(&hunk.lines[2], &[], false, 60, Theme::default()));
        assert_eq!(added.trim_end(), "        2 +→   new··", "trailing spaces are marked on changed lines");
        assert_eq!(added.width(), 60, "a changed line is filled edge to edge");
    }

    #[test]
    fn long_lines_are_cut_to_the_pane() {
        let hunk = &diff::parse(&format!("@@ -1 +1 @@\n+{}\n", "x".repeat(100)))[0];
        let text = spans_text(&line_spans(&hunk.lines[0], &[], false, 30, Theme::default()));
        assert_eq!(text.width(), 30);
        assert!(text.ends_with('…'));
    }

    #[test]
    fn hunk_rows_split_range_and_context_and_count_when_folded() {
        let file = File::from_diff(&crate::forge::DiffFile {
            diff: "@@ -12,4 +12,5 @@ pub async fn charge\n a\n".into(),
            new_path: "a.rs".into(),
            old_path: "a.rs".into(),
            ..Default::default()
        });
        let open = spans_text(&hunk_spans(&file, 0, true, 80, Theme::default()));
        assert_eq!(open, "   ▾ @@ -12,4 +12,5 @@ pub async fn charge");
        let closed = spans_text(&hunk_spans(&file, 0, false, 80, Theme::default()));
        assert!(closed.ends_with("(1 lines)"), "{closed}");
    }

    #[test]
    fn wrapping_indents_continuations_and_keeps_the_fill_on_every_row() {
        let fill = Style::default().bg(Theme::named("dracula").unwrap().danger);
        let line = Line::from(vec![Span::raw("  1    1 "), Span::styled("+abcdefghij", fill), Span::styled("     ", fill)]);
        let rows = wrap_row(line, 14, 4);
        let text: Vec<String> = rows.iter().map(|r| r.spans.iter().map(|s| s.content.to_string()).collect()).collect();
        assert_eq!(text, ["  1    1 +abcd", "    efghij    "]);
        assert!(rows.iter().all(|r| r.width() == 14), "every row reaches the edge");
        assert_eq!(rows[1].spans.last().unwrap().style, fill, "the fill pads the last row");
    }
}
