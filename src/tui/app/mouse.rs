//! The mouse wheel scrolls the pane under the pointer, as its arrow keys would, and leaves the focus where it is.
//! A drag with the left button selects the text of the pane it starts in and copies it on release.
//! A click on a link opens it: the terminal hands revu the clicks, so it no longer opens links itself.
//! A click on a list row picks it, and a click on the picked row does what `enter` does on it.
use super::{Action, App, Focus, Open, Pane, Tree};
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

/// The lists a click can pick a row in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum List {
    Queue,
    Tree,
    Outline,
    Threads,
}

/// A list row as the last frame drew it: which list, and the index its cursor takes on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListRow {
    pub area: Rect,
    pub list: List,
    pub index: usize,
}

impl App {
    /// The wheel, a left drag, a click on a link or on a list row; over an overlay only its links answer, over a prompt nothing.
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
                self.pressed = Some(at);
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
        (0..times).flat_map(|_| self.pane_key(pane, KeyEvent::from(arrow))).collect()
    }

    /// A release where the press was is a click, on a link or a list row; any other ends the drag.
    fn release(&mut self, at: Position) -> Vec<Action> {
        if self.pressed.take() != Some(at) {
            return self.copy_drag();
        }
        self.drag = None;
        match self.link_at(at) {
            Some(url) => vec![Action::OpenUrl(url)],
            None => self.click_row(at),
        }
    }

    /// The row takes the cursor and its pane the keys; on the row the cursor is already on, `enter`.
    /// While text is being typed the keys belong to it, so rows do not answer.
    fn click_row(&mut self, at: Position) -> Vec<Action> {
        if self.input.is_some() || self.filtering {
            return vec![];
        }
        let Some(row) = self.list_rows.iter().find(|row| row.area.contains(at)).copied() else { return vec![] };
        self.focus = row.list.pane();
        if self.picked(row.list) == Some(row.index) {
            return self.pane_key(self.focus, KeyEvent::from(KeyCode::Enter));
        }
        self.pick(row.list, row.index);
        vec![]
    }

    fn picked(&self, list: List) -> Option<usize> {
        let open = self.open.as_ref();
        match list {
            List::Queue => Some(self.queue_selected),
            List::Tree => open?.tree.as_ref().map(|tree| tree.selected),
            List::Outline => open?.outline.as_ref().map(|outline| outline.selected),
            List::Threads => open?.pane.as_ref().map(|pane| pane.note),
        }
    }

    fn pick(&mut self, list: List, index: usize) {
        match list {
            List::Queue => self.queue_selected = index,
            List::Tree => self.update_open(|open| {
                let tree = open.tree.clone().map(|tree| Tree { selected: index, ..tree });
                open.with_tree(tree)
            }),
            List::Outline => self.update_open(|open| Open { outline: open.outline.map(|outline| outline.at(index)), ..open }),
            List::Threads => self.update_open(|open| {
                let pane = open.pane.clone().map(|pane| Pane { note: index, ..pane });
                open.with_pane(pane)
            }),
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

    fn pane_key(&mut self, pane: Focus, key: KeyEvent) -> Vec<Action> {
        match pane {
            Focus::Queue => self.handle_queue_key(key),
            Focus::Review => self.handle_review_key(key),
            Focus::Side => self.handle_side_key(key),
        }
    }

    /// An overlay holds the screen: help, publish, the cover, the palette or a share preview.
    pub fn modal(&self) -> bool {
        self.help.is_some() || self.publish.is_some() || self.brief.is_some() || self.palette.is_some() || self.sharing.is_some()
    }
}

impl List {
    fn pane(self) -> Focus {
        match self {
            Self::Queue => Focus::Queue,
            Self::Tree | Self::Outline | Self::Threads => Focus::Side,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::{List, ListRow};
    use crate::review::tree::TreeRow;
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

    /// The first row of `list` drawn that its cursor is not on.
    fn other_row(app: &mut App, list: List) -> ListRow {
        render(app, 160, 30);
        let picked = app.picked(list);
        *app.list_rows.iter().find(|row| row.list == list && Some(row.index) != picked).expect("a row besides the picked one")
    }

    #[test]
    fn a_click_on_a_queue_row_picks_it_and_a_second_click_opens_its_mr() {
        let mut app = with_queue();
        app.focus = Focus::Review;
        let row = other_row(&mut app, List::Queue);
        assert_eq!(click_row(&mut app, row.list, row.index), vec![]);
        assert_eq!((app.queue_selected, app.focus), (row.index, Focus::Queue));
        let key = app.selected_mr().expect("an MR row").key();
        assert_eq!(click_row(&mut app, row.list, row.index), vec![Action::Open(key)]);
        assert_eq!(app.focus, Focus::Review, "the MR opens in the diff");
    }

    #[test]
    fn a_drag_over_queue_rows_picks_and_opens_nothing() {
        let mut app = with_queue();
        let row = other_row(&mut app, List::Queue);
        let before = app.queue_selected;
        let from = (row.area.right() - 1, row.area.y);
        assert_eq!(drag(&mut app, (from.0 - 1, from.1), from, 160, 30), vec![]);
        assert_eq!(app.queue_selected, before);
    }

    #[test]
    fn rows_do_not_answer_while_the_filter_is_typed() {
        let mut app = with_queue();
        let row = other_row(&mut app, List::Queue);
        let before = app.queue_selected;
        press(&mut app, "/");
        assert_eq!(click_row(&mut app, row.list, row.index), vec![]);
        assert_eq!(app.queue_selected, before);
    }

    #[test]
    fn a_click_on_a_tree_row_picks_it_and_a_second_click_shows_the_file() {
        let with_tree = || {
            let mut app = with_review();
            press(&mut app, "t");
            app.focus = Focus::Review;
            app
        };
        let mut app = with_tree();
        render(&mut app, 160, 30);
        let picked = app.picked(List::Tree);
        let rows = app.kept.tree_rows(app.open.as_ref().unwrap()).to_vec();
        let unpicked_file =
            |row: &&ListRow| row.list == List::Tree && Some(row.index) != picked && matches!(rows[row.index], TreeRow::File { .. });
        let row = *app.list_rows.iter().find(unpicked_file).expect("a file besides the picked row");
        click_row(&mut app, row.list, row.index);
        assert_eq!((app.picked(List::Tree), app.focus), (Some(row.index), Focus::Side));
        let mut walked = with_tree();
        walked.pick(List::Tree, row.index);
        walked.focus = Focus::Side;
        walked.handle_key(code(KeyCode::Enter));
        click_row(&mut app, row.list, row.index);
        assert_eq!(app.focus, Focus::Review);
        assert_eq!(app.open.as_ref().unwrap().selected, walked.open.as_ref().unwrap().selected, "the diff's cursor lands as enter puts it");
    }

    #[test]
    fn a_click_on_a_listed_thread_picks_it_and_a_second_click_jumps_to_its_line() {
        let with_list = || {
            let mut app = with_review();
            press(&mut app, "T");
            app
        };
        let mut app = with_list();
        render(&mut app, 160, 30);
        let row = *app.list_rows.iter().rfind(|row| row.list == List::Threads).expect("a listed thread");
        let before = app.open.as_ref().unwrap().selected;
        click_row(&mut app, row.list, row.index);
        assert_eq!(app.picked(List::Threads), Some(row.index));
        assert_eq!(app.open.as_ref().unwrap().selected, before, "a pick alone leaves the diff where it was");
        let mut walked = with_list();
        walked.pick(List::Threads, row.index);
        walked.handle_key(code(KeyCode::Enter));
        assert_ne!(walked.open.as_ref().unwrap().selected, before, "enter moves the diff's cursor");
        click_row(&mut app, row.list, row.index);
        assert_eq!(app.open.as_ref().unwrap().selected, walked.open.as_ref().unwrap().selected);
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
