//! The anchor column: the mark a line carries and the range comment the pane is on.
use super::ANCHOR_W;
use crate::review::{Mark, Marker, Markers, Place, Review, Row, Side};
use crate::tui::app::{ConversationList, EntryKind};
use crate::tui::theme::Theme;
use ratatui::style::Style;
use ratatui::text::Span;

/// What the anchor column draws from: the marks of every line, and the range comment the pane is on.
pub(super) struct Anchors<'m> {
    pub(super) markers: &'m Markers,
    pub(super) stretch: Option<Stretch>,
}

/// The lines of a range comment before its last, which carries the mark.
#[derive(Clone, Copy)]
pub(super) struct Stretch {
    file: usize,
    side: Side,
    first: u32,
    last: u32,
}

impl Stretch {
    pub(super) fn covers(self, review: &Review, row: &Row) -> bool {
        let Some(Place::Line { file, new, old }) = review.place_of(row) else { return false };
        let number = if self.side == Side::New { new } else { old };
        number.is_some_and(|n| self.holds(file, self.side, n))
    }

    pub(super) fn holds(self, file: usize, side: Side, number: u32) -> bool {
        file == self.file && side == self.side && (self.first..self.last).contains(&number)
    }
}

/// The range of the thread or draft under the pane's cursor, when it spans several lines.
pub(super) fn focused_range(review: &Review, pane: Option<&ConversationList>) -> Option<Stretch> {
    let pane = pane?;
    let focused = pane.focused?;
    let position = match (&pane.conversations.get(focused.conversation)?.thread, focused.kind) {
        (Some(id), _) => review.thread(id)?.first().position.clone()?,
        (None, EntryKind::Draft(index)) => review.drafts.get(index)?.position.clone()?,
        (None, EntryKind::Note(_)) => return None,
    };
    let side = position.line.side();
    let number = |line: crate::forge::LineRef| if side == Side::New { line.new } else { line.old };
    let (first, last) = (number(position.start?)?, number(position.line)?);
    let file = review.files.iter().position(|f| f.new_path == position.new_path || f.old_path == position.old_path)?;
    Some(Stretch { file, side, first, last })
}

pub(super) fn range_spans<'a>(theme: Theme) -> Vec<Span<'a>> {
    vec![Span::styled("│", Style::default().fg(theme.accent)), Span::raw(" ")]
}

/// The anchor column: the most pressing mark of the line, then how many conversations it holds when more than one.
pub(super) fn anchor_spans<'a>(marker: Option<Marker>, theme: Theme) -> Vec<Span<'a>> {
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
