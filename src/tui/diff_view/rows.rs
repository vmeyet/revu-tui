//! Each kind of diff row drawn inline: the MR's own row, files, hunks and lines, tables boxed in Markdown files.
use super::anchors::{Anchors, anchor_spans, range_spans};
use super::line::{line_spans, pair_spans, peeked, quiet_spans};
use super::wrap::is_blank;
use super::{ANCHOR_W, Gutter, Overflow, UNCUT};
use crate::diff::words::same_but_whitespace;
use crate::diff::{Line as DiffLine, LineKind};
use crate::review::{File, FileKind, Place, Review, Row};
use crate::syntax;
use crate::tui::table::{self, TableLine};
use crate::tui::theme::Theme;
use crate::tui::ui::truncate;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

const INDENT: &str = "   ";

pub(super) fn row_line<'a>(
    review: &Review,
    anchors: &Anchors<'_>,
    row: &Row,
    marked: bool,
    gutter: Gutter,
    overflow: Overflow,
    theme: Theme,
) -> Line<'a> {
    let width = match overflow {
        Overflow::Cut(width) => width,
        Overflow::Wrap(_) => UNCUT,
    };
    let bar = Span::styled(if marked { "▎" } else { " " }, Style::default().fg(theme.accent));
    let mut spans = vec![bar];
    let body = width.saturating_sub(1);
    if matches!(row, Row::Line { .. } | Row::Pair { .. } | Row::Context { .. }) {
        let marker = review.marker_of(anchors.markers, row);
        let spans_range = marker.is_none() && anchors.stretch.is_some_and(|stretch| stretch.covers(review, row));
        spans.extend(if spans_range { range_spans(theme) } else { anchor_spans(marker, theme) });
    }
    let body = if spans.len() > 1 { body.saturating_sub(ANCHOR_W) } else { body };
    match row {
        Row::Gap | Row::Thread { .. } => {}
        Row::Header => spans.extend(on_the_mr_spans(review, theme)),
        Row::File { index, open } => spans.extend(file_spans(review, &review.files[*index], *open, gutter, body, theme)),
        Row::Hunk { file, index, open } => spans.extend(hunk_spans(&review.files[*file], *index, *open, body, theme)),
        Row::Line { file, hunk, index } => {
            let file = &review.files[*file];
            let line = &file.hunks[*hunk].lines[*index];
            let shown = review.peek.map(|side| peeked(line, side));
            let drawn = line_spans(shown.as_ref().unwrap_or(line), file.spans(*hunk, *index), marked, gutter, body, theme);
            spans.extend(with_table_lines(drawn, file, &line.text, gutter.wrap_indent(), overflow, theme));
        }
        Row::Pair { file, hunk, removed, added } => {
            let file = &review.files[*file];
            let code = file.spans(*hunk, *added);
            let (old, new) = (&file.hunks[*hunk].lines[*removed], &file.hunks[*hunk].lines[*added]);
            let drawn = if review.quiet_whitespace && same_but_whitespace(&old.text, &new.text) {
                quiet_spans(old, new, code, marked, gutter, body, theme)
            } else {
                pair_spans(old, new, code, marked, gutter, body, theme)
            };
            spans.extend(with_table_lines(drawn, file, &new.text, gutter.wrap_indent(), overflow, theme));
        }
        Row::Context { file, old, new, .. } => {
            let line = context_line(review, *file, *old, *new);
            let line = match review.peek {
                Some(side) => peeked(&line, side),
                None => line,
            };
            let file = &review.files[*file];
            let drawn = line_spans(&line, &[], marked, gutter, body, theme);
            spans.extend(with_table_lines(drawn, file, &line.text, gutter.wrap_indent(), overflow, theme));
        }
    }
    Line::from(spans)
}

/// Line `new` of a file read whole, `old` on the old side.
pub(super) fn context_line(review: &Review, file: usize, old: u32, new: u32) -> DiffLine {
    let path = &review.files[file].new_path;
    let text = review.context.texts.get(path).and_then(|t| t.get(new as usize - 1)).cloned().unwrap_or_default();
    DiffLine { kind: LineKind::Context, old: Some(old), new: Some(new), text, words: vec![], no_newline: false }
}

/// A drawn diff line, its table pipes as box lines when `text` is a table line of a markdown file,
/// its cells wrapped in their columns when it wraps, the text starting `indent` columns into the row;
/// the gutter and the sign before the text stay as they are.
pub(super) fn with_table_lines<'a>(
    mut spans: Vec<Span<'a>>,
    file: &File,
    text: &str,
    indent: usize,
    overflow: Overflow,
    theme: Theme,
) -> Vec<Span<'a>> {
    let Some(line) = TableLine::parse(text).filter(|_| syntax::is_markdown(&file.new_path)) else { return spans };
    let text_spans = spans.split_off(2.min(spans.len()));
    let pad = text_spans.last().filter(|span| is_blank(span)).cloned();
    let rows = match overflow {
        Overflow::Wrap(width) => {
            table::wrapped(&text_spans, line, width.saturating_sub(indent), pad.as_ref().map(|s| s.style).unwrap_or_default(), theme)
        }
        Overflow::Cut(_) => None,
    };
    match rows {
        // Each row is exactly as wide as the text column, so `wrap_row` cuts the line at their joins.
        Some(rows) => spans.extend(rows.into_iter().flatten().chain(pad)),
        None => spans.extend(table::boxed(text_spans, line, theme)),
    }
    spans
}

/// The row under the header: the conversations on the MR itself, `enter` opens them.
fn on_the_mr_spans<'a>(review: &Review, theme: Theme) -> Vec<Span<'a>> {
    let listed = review.conversations(&Place::Mr);
    if listed.is_empty() {
        return vec![];
    }
    let unresolved = listed.iter().filter_map(|c| c.thread.as_ref().and_then(|id| review.thread(id))).any(|t| !t.resolved);
    let (glyph, colour) = if unresolved { ("◆", theme.warn) } else { ("◇", theme.accent) };
    vec![
        Span::styled(format!("{glyph} {} on the MR", listed.len()), Style::default().fg(colour)),
        Span::styled("  enter opens", Style::default().fg(theme.faded)),
    ]
}

fn file_spans<'a>(review: &Review, file: &File, open: bool, gutter: Gutter, width: usize, theme: Theme) -> Vec<Span<'a>> {
    let mark = if open { "▾ " } else { "▸ " };
    let (dir, base) = match file.new_path.rsplit_once('/') {
        Some((dir, base)) => (format!("{dir}/"), base.to_owned()),
        None => (String::new(), file.new_path.clone()),
    };
    let counts = format!("+{} −{}", file.additions, file.deletions);
    let in_file = |a: &crate::review::Anchor| a.path == file.new_path || a.path == file.old_path;
    let on_lines: Vec<&crate::review::Thread> =
        review.threads.iter().filter(|t| !t.outdated && t.anchor.as_ref().is_some_and(in_file)).collect();
    let resolved = on_lines.iter().filter(|t| t.resolved).count();
    let unresolved = on_lines.len() - resolved;
    let drafts = review.drafts.iter().filter(|d| d.reply_to.is_none() && d.anchor.as_ref().is_some_and(in_file)).count();
    let outdated = review.outdated(&file.new_path).len();
    let (warn, faded) = (Style::default().fg(theme.warn), Style::default().fg(theme.faded));
    let anchors: Vec<(String, Style)> = [
        (unresolved > 0).then(|| (format!("◆{unresolved}"), warn)),
        (resolved > 0).then(|| (format!("✓{resolved}"), faded)),
        (drafts > 0).then(|| (format!("◇{drafts}"), warn)),
    ]
    .into_iter()
    .flatten()
    .enumerate()
    .map(|(i, (text, style))| (format!("{}{text}", if i == 0 { "  " } else { " " }), style))
    .chain((outdated > 0).then(|| (format!(" · {outdated} outdated"), warn)))
    .collect();
    let anchors_w: usize = anchors.iter().map(|(text, _)| text.width()).sum();
    let state = file_state(file, review, open);
    let tail_w = counts.width() + anchors_w + state.as_ref().map_or(0, |s| s.width() + 2);
    let name_room = width.saturating_sub(mark.width() + tail_w + 2);
    let name = gutter.path(&format!("{dir}{base}"), name_room);
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
    ];
    spans.extend(anchors.into_iter().map(|(text, style)| Span::styled(text, style)));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::Side;
    use crate::tui::diff_view::test_support::{long_added_line, painted, side_by_side_of, spans_text};
    use crate::tui::diff_view::wrap::wrap_row;

    /// The fixture review with its first file's line threads replaced by `resolved` flags, one thread each.
    fn with_line_threads(resolved: &[bool]) -> Review {
        let base = crate::review::tests::review();
        let sample = base.threads.iter().find(|t| t.anchor.is_some() && !t.outdated).cloned().expect("a line thread");
        let others = base.threads.iter().filter(|t| t.id != sample.id).cloned();
        let made =
            resolved.iter().enumerate().map(|(i, &r)| crate::review::Thread { id: format!("t{i}").into(), resolved: r, ..sample.clone() });
        Review { threads: others.chain(made).collect::<Vec<_>>().into(), ..base }
    }

    fn file_row(review: &Review) -> Vec<Span<'static>> {
        file_spans(review, &review.files[0], true, Gutter::Both, 120, Theme::default())
    }

    #[test]
    fn a_file_counts_open_and_resolved_threads_apart() {
        let all_resolved = file_row(&with_line_threads(&[true, true, true]));
        let text = spans_text(&all_resolved);
        assert!(text.contains("✓3") && !text.contains('◆'), "only resolved threads need nothing: {text}");
        let resolved = all_resolved.iter().find(|s| s.content.contains("✓3")).unwrap();
        assert_eq!(resolved.style.fg, Some(Theme::default().faded));
        let mixed = spans_text(&file_row(&with_line_threads(&[false, true, true])));
        assert!(mixed.contains("◆1 ✓2"), "{mixed}");
        let none = spans_text(&file_row(&with_line_threads(&[])));
        assert!(!none.contains('◆') && !none.contains('✓'), "{none}");
    }

    fn pricing(path: &str) -> File {
        File::from_diff(&crate::forge::DiffFile {
            diff: "@@ -1,5 +1,5 @@\n # Plans\n \n-| Plan | Seats |\n+| Plan | Seats \\| max |\n |:-----|------:|\n | Team | *10* |\n"
                .into(),
            old_path: path.into(),
            new_path: path.into(),
            ..Default::default()
        })
    }

    fn drawn_lines(file: &File, theme: Theme) -> Vec<Vec<Span<'static>>> {
        let lines = &file.hunks[0].lines;
        let each =
            lines.iter().enumerate().map(|(i, line)| (line_spans(line, file.spans(0, i), false, Gutter::Both, 60, theme), &line.text));
        let pair = (pair_spans(&lines[2], &lines[3], file.spans(0, 3), false, Gutter::Both, 60, theme), &lines[3].text);
        each.chain([pair])
            .map(|(spans, text)| with_table_lines(spans, file, text, Gutter::Both.wrap_indent(), Overflow::Cut(60), theme))
            .collect()
    }

    #[test]
    fn snapshot_markdown_table_hunk() {
        let theme = Theme::named("tokyonight").unwrap();
        let lines: Vec<String> = drawn_lines(&pricing("docs/pricing.md"), theme).iter().map(|spans| painted(spans)).collect();
        insta::assert_snapshot!("markdown_table_hunk", lines.join("\n---\n"));
    }

    #[test]
    fn snapshot_wrapped_markdown_table_in_a_narrow_pane() {
        let diff = crate::forge::DiffFile {
            diff: "@@ -1,4 +1,4 @@\n \
                   | Plan | Seats | Notes                             |\n \
                   |:-----|------:|:----------------------------------|\n\
                   -| Team |    10 | shared billing for each workspace |\n\
                   +| Team |    12 | shared billing for all workspaces |\n \
                   | Solo |     1 | one seat                          |\n"
                .into(),
            old_path: "docs/plans.md".into(),
            new_path: "docs/plans.md".into(),
            ..Default::default()
        };
        let review = Review::new((*crate::review::tests::review().mr).clone(), &[diff], vec![], &[]);
        let anchors = Anchors { markers: &review.markers(), stretch: None };
        let rows = review.rows().into_iter().filter(|row| matches!(row, Row::Line { .. } | Row::Context { .. }));
        let drawn = rows.flat_map(|row| {
            let line = row_line(&review, &anchors, &row, false, Gutter::Both, Overflow::Wrap(44), Theme::default());
            wrap_row(line, 44, Gutter::Both.wrap_indent())
        });
        insta::assert_snapshot!("wrapped_markdown_table", drawn.map(|line| spans_text(&line.spans)).collect::<Vec<_>>().join("\n"));
    }

    #[test]
    fn a_peek_draws_its_side_without_signs_or_fills_numbered_on_that_side_alone() {
        let peek = |side| side_by_side_of("@@ -1,2 +1,2 @@\n a\n-b\n+B\n", "a.txt").with_peek(Some(side));
        let drawn = |review: Review| -> Vec<String> {
            let anchors = Anchors { markers: &review.markers(), stretch: None };
            let lines = review.rows().into_iter().filter(|row| matches!(row, Row::Line { .. }));
            lines
                .map(|row| spans_text(&row_line(&review, &anchors, &row, false, Gutter::Both, Overflow::Cut(40), Theme::default()).spans))
                .collect()
        };
        assert_eq!(drawn(peek(Side::Old)), ["      1       a", "      2       b"]);
        assert_eq!(drawn(peek(Side::New)), ["           1  a", "           2  B"]);
    }

    #[test]
    fn table_pipes_stay_raw_outside_markdown_and_in_the_diff_text() {
        let file = pricing("docs/pricing.txt");
        let theme = Theme::default();
        assert!(drawn_lines(&file, theme).iter().all(|spans| !spans_text(spans).contains('│')));
        let markdown = pricing("docs/pricing.md");
        assert!(drawn_lines(&markdown, theme).iter().all(|spans| !spans_text(&spans[..2]).contains('│')), "the gutter stays");
        assert_eq!(markdown.hunks[0].lines[3].text, "| Plan | Seats \\| max |", "only the drawing changes");
    }

    #[test]
    fn a_wrapped_line_continues_under_its_text_whatever_the_gutter() {
        let review = long_added_line();
        let anchors = Anchors { markers: &review.markers(), stretch: None };
        let row = Row::Line { file: 0, hunk: 0, index: 1 };
        let wrapped = |gutter: Gutter| -> Vec<String> {
            let line = row_line(&review, &anchors, &row, false, gutter, Overflow::Wrap(30), Theme::default());
            wrap_row(line, 30, gutter.wrap_indent()).iter().map(|line| spans_text(&line.spans)).collect()
        };
        let x = |n: usize| "x".repeat(n);
        let under = |indent: usize, n: usize, pad: usize| format!("{}{}{}", " ".repeat(indent), x(n), " ".repeat(pad));
        assert_eq!(wrapped(Gutter::Both), [format!("           2 +{}", x(16)), under(14, 16, 0), under(14, 8, 8)]);
        assert_eq!(wrapped(Gutter::One), [format!("      2 +{}", x(21)), under(9, 19, 2)]);
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
}
