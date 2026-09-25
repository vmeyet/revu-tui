//! The mouse wheel scrolls the pane under the pointer, as its arrow keys would, and leaves the focus where it is.
//! A drag with the left button selects the text of the pane it starts in and copies it on release.
//! A click on a link opens it: the terminal hands revu the clicks, so it no longer opens links itself.
use super::{Action, App, Focus};
use crate::tui::drag::Drag;
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
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
    fn under(self, at: Position) -> Option<Focus> {
        [(self.queue, Focus::Queue), (self.review, Focus::Review), (self.side, Focus::Side)]
            .into_iter()
            .find_map(|(area, pane)| area.contains(at).then_some(pane))
    }
}

impl App {
    /// The wheel and a left drag; nothing works over an overlay or a prompt.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) -> Vec<Action> {
        if self.modal() || self.confirm.is_some() || self.react.is_some() {
            return vec![];
        }
        let at = Position::new(mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::ScrollDown => self.wheel(at, KeyCode::Down),
            MouseEventKind::ScrollUp => self.wheel(at, KeyCode::Up),
            MouseEventKind::Down(MouseButton::Left) => {
                self.drag = Drag::start(&self.text_rows, at);
                vec![]
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                self.drag = self.drag.map(|drag| drag.moved_to(&self.text_rows, at));
                vec![]
            }
            MouseEventKind::Up(MouseButton::Left) => self.release(at),
            _ => vec![],
        }
    }

    fn wheel(&mut self, at: Position, arrow: KeyCode) -> Vec<Action> {
        self.drag = None;
        let Some(pane) = self.areas.under(at) else { return vec![] };
        self.repeat = None;
        let times = if pane == Focus::Review { DIFF_ROWS } else { 1 };
        (0..times).flat_map(|_| self.scroll(pane, KeyEvent::from(arrow))).collect()
    }

    /// A click on a link opens it; any other release ends the drag.
    fn release(&mut self, at: Position) -> Vec<Action> {
        let clicked = self.drag.is_none_or(Drag::is_click);
        match self.link_at(at).filter(|_| clicked) {
            Some(url) => {
                self.drag = None;
                vec![Action::OpenUrl(url)]
            }
            None => self.copy_drag(),
        }
    }

    fn link_at(&self, at: Position) -> Option<String> {
        let covers = |link: &&crate::tui::ui::Link| {
            let width = u16::try_from(link.text.chars().count()).unwrap_or(u16::MAX);
            link.y == at.y && (link.x..link.x.saturating_add(width)).contains(&at.x)
        };
        self.links.iter().find(covers).map(|link| link.url.clone())
    }

    /// A release ends the drag: the text goes to the clipboard, the highlight stays until the next key or click.
    fn copy_drag(&mut self) -> Vec<Action> {
        self.drag = self.drag.filter(|drag| !drag.is_click());
        let Some(text) = self.drag.map(|drag| drag.text(&self.text_rows)).filter(|text| !text.is_empty()) else {
            return vec![];
        };
        let lines = text.lines().count();
        vec![Action::Copy { text, done: format!("copied {lines} line{}", if lines == 1 { "" } else { "s" }) }]
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
