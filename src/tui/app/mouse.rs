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
    /// The diff and the right pane were both on screen.
    pub fn both_shown(self) -> bool {
        !self.review.is_empty() && !self.side.is_empty()
    }

    fn under(self, at: Position) -> Option<Focus> {
        [(self.queue, Focus::Queue), (self.review, Focus::Review), (self.side, Focus::Side)]
            .into_iter()
            .find_map(|(area, pane)| area.contains(at).then_some(pane))
    }
}

impl App {
    /// The wheel, a left drag and a click on a link; over an overlay only its links answer, over a prompt nothing.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) -> Vec<Action> {
        let at = Position::new(mouse.column, mouse.row);
        if self.confirm.is_some() || self.react.is_some() {
            return vec![];
        }
        if self.modal() {
            let released = matches!(mouse.kind, MouseEventKind::Up(MouseButton::Left));
            return self.link_at(at).filter(|_| released).map(|url| vec![Action::OpenUrl(url)]).unwrap_or_default();
        }
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crate::tui::app::test_support::*;

    #[test]
    fn the_wheel_scrolls_the_pane_under_the_pointer_and_leaves_the_focus() {
        let mut app = with_long_review();
        let mut walked = with_long_review();
        render(&mut app, 160, 30);
        wheel(&mut app, true, 80, 10);
        press(&mut walked, "jjj");
        assert_eq!(app.open.as_ref().unwrap().selected, walked.open.as_ref().unwrap().selected, "a notch is three rows of the diff");
        let queue_row = app.queue_selected;
        wheel(&mut app, true, 5, 5);
        assert_ne!(app.queue_selected, queue_row, "the queue moves under the pointer");
        assert_eq!(app.focus, Focus::Review, "the keys stay in the diff");
        wheel(&mut app, false, 5, 5);
        assert_eq!(app.queue_selected, queue_row);
    }

    #[test]
    fn the_wheel_does_nothing_under_an_overlay() {
        let mut app = with_long_review();
        render(&mut app, 160, 30);
        let before = app.open.as_ref().unwrap().selected;
        press(&mut app, "?");
        wheel(&mut app, true, 80, 10);
        assert_eq!(app.open.as_ref().unwrap().selected, before);
    }

    #[test]
    fn a_click_without_a_drag_or_from_the_gutter_copies_nothing() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut app = with_review();
        let at = spot(&mut app, "let client = Client::new()", 120, 24);
        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), at);
        assert_eq!(mouse(&mut app, MouseEventKind::Up(MouseButton::Left), at), vec![]);
        assert!(app.drag.is_none());
        let gutter = (at.0 - 8, at.1);
        assert_eq!(drag(&mut app, gutter, (at.0 + 5, at.1), 120, 24), vec![]);
    }

    #[test]
    fn a_click_on_a_link_opens_it_and_a_drag_over_it_does_not() {
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut app = with_review();
        render(&mut app, 160, 30);
        let title = app.links.iter().find(|link| link.text == "acme/widgets!42").expect("the diff's title links to the MR").clone();
        let at = (title.x + 3, title.y);
        mouse(&mut app, MouseEventKind::Down(MouseButton::Left), at);
        assert_eq!(mouse(&mut app, MouseEventKind::Up(MouseButton::Left), at), vec![Action::OpenUrl(title.url)]);
        let code = spot(&mut app, "let client = Client::new()", 160, 30);
        let dragged = drag(&mut app, code, at, 160, 30);
        assert!(dragged.iter().all(|action| !matches!(action, Action::OpenUrl(_))), "{dragged:?}");
    }

    #[test]
    fn a_drag_side_by_side_stays_in_the_half_it_started_in() {
        let mut app = with_review();
        app.focus = Focus::Review;
        press(&mut app, "D");
        let old = spot(&mut app, "let client = Client::new()", 320, 24);
        let new = spot(&mut app, "let client = Client::with_key", 320, 24);
        assert_eq!(old.1, new.1, "both halves on one row");
        let actions = drag(&mut app, (old.0, old.1 - 1), (new.0 + 3, new.1), 320, 24);
        let Some(Action::Copy { text, .. }) = actions.first() else { panic!("a copy: {actions:?}") };
        assert!(text.ends_with("\n    let client = Client::new();"), "{text}");
        assert!(!text.contains("with_key"), "{text}");
    }
}
