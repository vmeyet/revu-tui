//! The mouse wheel scrolls the pane under the pointer, as its arrow keys would, and leaves the focus where it is.
use super::{Action, App, Focus};
use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

/// Rows of the diff one notch of the wheel moves; the queue and the right pane move one item.
const DIFF_ROWS: usize = 3;

/// Where each pane was drawn in the last frame; a hidden pane has no area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Areas {
    pub queue: Rect,
    pub review: Rect,
    pub side: Rect,
}

impl Areas {
    fn under(self, column: u16, row: u16) -> Option<Focus> {
        let at = Position::new(column, row);
        [(self.queue, Focus::Queue), (self.review, Focus::Review), (self.side, Focus::Side)]
            .into_iter()
            .find_map(|(area, pane)| area.contains(at).then_some(pane))
    }
}

impl App {
    /// Anything but the wheel, and the wheel over an overlay or a prompt, does nothing.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) -> Vec<Action> {
        let arrow = match mouse.kind {
            MouseEventKind::ScrollDown => KeyCode::Down,
            MouseEventKind::ScrollUp => KeyCode::Up,
            _ => return vec![],
        };
        if self.modal() || self.confirm.is_some() || self.react.is_some() {
            return vec![];
        }
        let Some(pane) = self.areas.under(mouse.column, mouse.row) else { return vec![] };
        self.repeat = None;
        let times = if pane == Focus::Review { DIFF_ROWS } else { 1 };
        (0..times).flat_map(|_| self.scroll(pane, KeyEvent::from(arrow))).collect()
    }

    fn scroll(&mut self, pane: Focus, arrow: KeyEvent) -> Vec<Action> {
        match pane {
            Focus::Queue => self.handle_queue_key(arrow),
            Focus::Review => self.handle_review_key(arrow),
            Focus::Side => self.handle_side_key(arrow),
        }
    }

    /// An overlay holds the screen: help, publish, the cover, the palette or a share preview.
    pub fn modal(&self) -> bool {
        self.help.is_some() || self.publish.is_some() || self.brief.is_some() || self.palette.is_some() || self.sharing.is_some()
    }
}
