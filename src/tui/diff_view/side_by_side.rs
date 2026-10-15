//! A diff row as the old side beside the new one.
use super::anchors::{Anchors, anchor_spans, range_spans};
use super::line::{number, numbered_spans, quiet, with_quiet_sign};
use super::rows::{context_line, with_table_lines};
use super::wrap::{padded, wrap_row};
use super::{ANCHOR_W, GUTTER_W, Overflow, UNCUT};
use crate::diff::words::same_but_whitespace;
use crate::diff::{Line as DiffLine, LineKind};
use crate::review::{Review, Row, Side};
use crate::syntax::Token;
use crate::tui::theme::Theme;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use std::ops::Range;

/// A side by side half before its text: the anchor column, its one number and the sign.
pub(super) const HALF_INDENT: usize = ANCHOR_W + GUTTER_W + 2;
/// Columns of code each half shows at least; narrower, side by side falls back to inline.
const HALF_MIN_CODE: usize = 50;
pub(super) const SIDE_BY_SIDE_MIN_W: usize = 1 + 2 * (HALF_INDENT + HALF_MIN_CODE);

/// One side of a side by side row: a line of that side, drawn as context when it changed only in whitespace under `W`.
pub(super) struct Half<'r> {
    file: usize,
    side: Side,
    pub(super) line: DiffLine,
    code: &'r [(Range<usize>, Token)],
    quiet: bool,
}

/// What the old side and the new side of a diff row show; nothing on a side beside a longer run of the other.
pub(super) fn halves<'r>(review: &'r Review, row: &Row) -> Option<[Option<Half<'r>>; 2]> {
    let file = row.file()?;
    let shown = &review.files[file];
    let half = |side, line: &DiffLine, code, quiet| {
        Some(Half { file, side, line: if quiet { self::quiet(line) } else { line.clone() }, code, quiet })
    };
    match row {
        Row::Line { hunk, index, .. } => {
            let (line, code) = (&shown.hunks[*hunk].lines[*index], shown.spans(*hunk, *index));
            Some(match line.kind {
                LineKind::Context => [half(Side::Old, line, code, false), half(Side::New, line, code, false)],
                LineKind::Removed => [half(Side::Old, line, code, false), None],
                LineKind::Added => [None, half(Side::New, line, code, false)],
            })
        }
        Row::Pair { hunk, removed, added, .. } => {
            let (old, new) = (&shown.hunks[*hunk].lines[*removed], &shown.hunks[*hunk].lines[*added]);
            let quiet = review.quiet_whitespace && same_but_whitespace(&old.text, &new.text);
            Some([half(Side::Old, old, shown.spans(*hunk, *removed), quiet), half(Side::New, new, shown.spans(*hunk, *added), quiet)])
        }
        Row::Context { old, new, .. } => {
            let line = context_line(review, file, *old, *new);
            Some([half(Side::Old, &line, &[], false), half(Side::New, &line, &[], false)])
        }
        Row::Header | Row::File { .. } | Row::Hunk { .. } | Row::Thread { .. } | Row::Gap => None,
    }
}

/// A diff row as the old side beside the new one, each half cut, or wrapped under its own text.
pub(super) fn side_by_side_lines(
    review: &Review,
    anchors: &Anchors<'_>,
    row: &Row,
    marked: bool,
    overflow: Overflow,
    theme: Theme,
) -> Option<Vec<Line<'static>>> {
    let [old, new] = halves(review, row)?;
    let (Overflow::Cut(width) | Overflow::Wrap(width)) = overflow;
    let (old_w, new_w) = half_widths(width);
    let draw = |half: Option<Half<'_>>, width: usize| -> Vec<Line<'static>> {
        let Some(half) = half else { return vec![] };
        match overflow {
            Overflow::Cut(_) => vec![half_line(review, anchors, &half, marked, width, theme)],
            Overflow::Wrap(_) => wrap_row(half_line(review, anchors, &half, marked, UNCUT, theme), width, HALF_INDENT),
        }
    };
    let (old, new) = (draw(old, old_w), draw(new, new_w));
    let bar = Span::styled(if marked { "▎" } else { " " }, Style::default().fg(theme.accent));
    let spans_at = |half: &[Line<'static>], i: usize| half.get(i).map(|l| l.spans.clone()).unwrap_or_default();
    let rows = (0..old.len().max(new.len())).map(|i| {
        let first = if i == 0 { bar.clone() } else { Span::raw(" ") };
        let old_half = padded(spans_at(&old, i), old_w, Some(Style::default()));
        Line::from([vec![first], old_half.spans, spans_at(&new, i)].concat())
    });
    Some(rows.collect())
}

/// The columns of the old half and the new half, past the cursor bar.
pub(super) fn half_widths(width: usize) -> (usize, usize) {
    let body = width.saturating_sub(1);
    (body / 2, body - body / 2)
}

/// One half: its anchor column, its side's number, the sign, the text in its side's syntax colours.
fn half_line(review: &Review, anchors: &Anchors<'_>, half: &Half<'_>, marked: bool, width: usize, theme: Theme) -> Line<'static> {
    let file = &review.files[half.file];
    let (path, number_here) = match half.side {
        Side::Old => (&file.old_path, half.line.old),
        Side::New => (&file.new_path, half.line.new),
    };
    let marker = number_here.and_then(|n| anchors.markers.at(path, half.side, n));
    let stretched = marker.is_none() && number_here.is_some_and(|n| anchors.stretch.is_some_and(|s| s.holds(half.file, half.side, n)));
    let anchor = if stretched { range_spans(theme) } else { anchor_spans(marker, theme) };
    let drawn = numbered_spans(format!("{} ", number(number_here)), &half.line, half.code, marked, width.saturating_sub(ANCHOR_W), theme);
    let drawn = if half.quiet { with_quiet_sign(drawn, theme) } else { drawn };
    Line::from([anchor, with_table_lines(drawn, file, &half.line.text, HALF_INDENT, Overflow::Cut(width), theme)].concat())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::diff_view::test_support::{side_by_side_of, spans_text};

    /// Every screen row of the diff lines as its old half and its new half, trimmed, halves 20 columns wide.
    fn halves_of(review: &Review, overflow: Overflow) -> Vec<(String, String)> {
        let anchors = Anchors { markers: &review.markers(), stretch: None };
        let rows = review.rows();
        let lines = rows.iter().filter_map(|row| side_by_side_lines(review, &anchors, row, false, overflow, Theme::default()));
        let split = |line: Line<'_>| {
            let text: Vec<char> = spans_text(&line.spans).chars().collect();
            let half = |range: Range<usize>| text.get(range).unwrap_or_default().iter().collect::<String>().trim_end().to_owned();
            (half(1..21), half(21..text.len()))
        };
        lines.flatten().map(split).collect()
    }

    #[test]
    fn side_by_side_numbers_each_side_and_leaves_a_blank_beside_the_longer_run() {
        let review = side_by_side_of("@@ -1,3 +1,2 @@\n a\n-b\n-c\n+B\n", "a.txt");
        let rows = halves_of(&review, Overflow::Cut(41));
        let expected = [("     1  a", "     1  a"), ("     2 -b", "     2 +B"), ("     3 -c", "")];
        assert_eq!(rows, expected.map(|(old, new)| (old.to_owned(), new.to_owned())));
    }

    #[test]
    fn side_by_side_wraps_each_half_under_its_own_text() {
        let review = side_by_side_of(&format!("@@ -1 +1 @@\n-{}\n+short\n", "x".repeat(30)), "a.txt");
        let rows = halves_of(&review, Overflow::Wrap(41));
        let x = |n: usize| "x".repeat(n);
        let expected = [(format!("     1 -{}", x(12)), "     1 +short".to_owned()), (format!("        {}", x(12)), String::new())];
        assert_eq!(rows[..2], expected);
        assert_eq!(rows[2].0, format!("        {}", x(6)));
    }

    #[test]
    fn side_by_side_reads_a_whitespace_only_change_as_quiet_on_both_sides_under_w() {
        let review = side_by_side_of("@@ -1 +1 @@\n-  one();\n+    one();\n", "a.txt").with_quiet_whitespace(true);
        let rows = halves_of(&review, Overflow::Cut(41));
        assert_eq!(rows, [("     1 ≈  one();".to_owned(), "     1 ≈    one();".to_owned())]);
    }

    #[test]
    fn side_by_side_colours_each_side_with_its_own_syntax() {
        let diff = include_str!("../../review/fixtures/cart.ts.diff");
        let review = side_by_side_of(diff, "src/cart.ts");
        let theme = Theme::named("tokyonight").unwrap();
        let anchors = Anchors { markers: &review.markers(), stretch: None };
        let pair = Row::Pair { file: 0, hunk: 0, removed: 1, added: 2 };
        let line = &side_by_side_lines(&review, &anchors, &pair, false, Overflow::Cut(200), theme).unwrap()[0];
        let keywords = line.spans.iter().filter(|s| s.content == "const" && s.style.fg == Some(theme.syntax.keyword)).count();
        assert_eq!(keywords, 2, "both sides colour their own `const`");
    }
}
