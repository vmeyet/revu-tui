//! The outline pane (`O`): the functions, methods and classes the MR changes, by file.
use super::{Action, App, Focus, Open};
use crate::diff::fold::FoldState;
mod rows;

pub(crate) use rows::{PaneLine, Rows};

use crate::act::{Act, Pane};
use crate::outline::Reading;
use crate::review::{Place, Review, Row, Side};
use crossterm::event::KeyEvent;
use std::sync::Arc;

/// The pane while it is open: what was read, the row under the cursor, how it shows, and the rows that makes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Outline {
    pub(crate) symbols: Symbols,
    pub(crate) selected: usize,
    /// Built whenever the rest changes, so drawing only slices it.
    pub(crate) rows: Arc<Rows>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Symbols {
    Waiting,
    /// Shared, so moving the cursor copies none of it.
    Ready(Arc<Reading>),
    Failed(String),
}

impl Outline {
    fn waiting() -> Self {
        Self { symbols: Symbols::Waiting, selected: 0, rows: Arc::default() }
    }

    /// The cursor on change `selected`.
    pub(super) fn at(self, selected: usize) -> Self {
        Self { selected, ..self }
    }

    /// The rows built again for what changed, the cursor kept on them.
    fn rebuilt(self) -> Self {
        let Symbols::Ready(reading) = &self.symbols else { return self };
        let rows = Rows::build(reading);
        let selected = self.selected.min(rows.line_of.len().saturating_sub(1));
        Self { rows: Arc::new(rows), selected, ..self }
    }
}

impl Open {
    fn with_outline(self, outline: Option<Outline>) -> Self {
        Self { outline, ..self }
    }
}

impl App {
    pub(super) fn outline_open(&self) -> bool {
        self.open.as_ref().is_some_and(|o| o.outline.is_some())
    }

    /// `O`: the outline takes the right pane; `O` again gives it back.
    pub(super) fn toggle_outline(&mut self) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        if open.outline.is_some() {
            self.close_outline();
            return vec![];
        }
        self.update_open(|open| Open { pane: None, tree: None, pipeline: None, ..open });
        self.focus = Focus::Side;
        self.read_outline()
    }

    /// The pane waits for the symbols of every file, read again.
    fn read_outline(&mut self) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let action = load(open);
        self.update_open(|open| open.with_outline(Some(Outline::waiting())));
        vec![action]
    }

    /// `:outline`: the pane, open or opened.
    pub(super) fn show_outline(&mut self) -> Vec<Action> {
        if self.outline_open() {
            self.focus = Focus::Side;
            return vec![];
        }
        self.toggle_outline()
    }

    fn close_outline(&mut self) {
        self.update_open(|open| open.with_outline(None));
        self.focus = Focus::Review;
    }

    pub(super) fn handle_outline_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let act = self.act_here(Pane::Outline, key);
        let Some(outline) = self.open.as_ref().and_then(|o| o.outline.as_ref()) else { return vec![] };
        let last = outline.rows.line_of.len().saturating_sub(1);
        let moved = |selected: usize| Outline { selected: selected.min(last), ..outline.clone() };
        let page = super::keys::full_page(self.areas.side).unsigned_abs();
        let next = match act {
            Some(Act::MoveDown) => moved(outline.selected + 1),
            Some(Act::MoveUp) => moved(outline.selected.saturating_sub(1)),
            Some(Act::PageDown) => moved(outline.selected + 10),
            Some(Act::PageUp) => moved(outline.selected.saturating_sub(10)),
            Some(Act::FullPageDown) => moved(outline.selected.saturating_add(page)),
            Some(Act::FullPageUp) => moved(outline.selected.saturating_sub(page)),
            Some(Act::Top) => moved(0),
            Some(Act::Bottom) => moved(last),
            Some(Act::Open) => return self.jump_from_outline(),
            Some(Act::Refresh) => return self.read_outline(),
            Some(Act::Back | Act::Outline | Act::ClosePane) => {
                self.close_outline();
                return vec![];
            }
            _ => {
                self.did_nothing();
                return vec![];
            }
        };
        self.update_open(|open| open.with_outline(Some(next)));
        vec![]
    }

    /// `enter`: the diff's cursor on the symbol's definition, and the keys to the diff.
    fn jump_from_outline(&mut self) -> Vec<Action> {
        let Some(outline) = self.open.as_ref().and_then(|o| o.outline.as_ref()) else { return vec![] };
        let Symbols::Ready(reading) = &outline.symbols else { return vec![] };
        let Some(change) = reading.changes.get(outline.selected) else { return vec![] };
        let Some(target) = self.open.as_ref().and_then(|open| row_at(&open.review, &change.path, change.side(), change.symbol.lines))
        else {
            self.toast("its lines are not in the diff");
            return vec![];
        };
        let actions = self.jump_to_row(&target);
        self.focus = Focus::Review;
        actions
    }

    /// What was read for `key`, unless the reader moved on to another MR meanwhile.
    pub(super) fn apply_outline(&mut self, key: &super::MrKey, reading: Reading) {
        if self.open.as_ref().is_some_and(|o| &o.key == key) {
            self.settle_outline(Symbols::Ready(Arc::new(reading)));
        }
    }

    /// The symbols read, or why they could not be; dropped once the pane closed.
    pub(super) fn settle_outline(&mut self, symbols: Symbols) {
        if self.outline_open() {
            self.update_open(|open| open.with_outline(Some(Outline { symbols, ..Outline::waiting() }.rebuilt())));
        }
    }
}

/// Every file the outline can read, each with its path at base and at head, absent on the side it does not exist.
fn load(open: &Open) -> Action {
    let refs = &open.review.mr.refs;
    let files = open
        .review
        .files
        .iter()
        .filter(|f| !f.binary && crate::outline::readable(&f.new_path))
        .map(|f| super::Sides {
            base: (f.kind != crate::review::FileKind::Added).then(|| f.old_path.clone()),
            head: (f.kind != crate::review::FileKind::Deleted).then(|| f.new_path.clone()),
        })
        .collect();
    Action::LoadOutline { key: open.key.clone(), base: refs.base.clone(), head: refs.head.clone(), files }
}

/// The first row showing one of `lines` of `path` on `side`, every fold open.
fn row_at(review: &Review, path: &str, side: Side, (first, last): (u32, u32)) -> Option<Row> {
    let file = review.files.iter().position(|f| f.new_path == path || f.old_path == path)?;
    let inside = |row: &Row| match review.place_of(row) {
        Some(Place::Line { file: at, new, old }) if at == file => {
            let line = if side == Side::New { new } else { old };
            line.is_some_and(|line| (first..=last).contains(&line))
        }
        _ => false,
    };
    review.clone().with_fold(FoldState::default()).rows().into_iter().find(inside)
}

#[cfg(test)]
mod tests {
    use crate::tui::app::List;
    use crate::tui::app::test_support::*;

    const BASE: &str =
        "class Cart:\n    def total(self):\n        return 1\n\n\ndef pay(card):\n    return card.charge()\n\n\ndef _log():\n    pass\n";
    const HEAD: &str = "class Cart:\n    def total(self):\n        return pay(self, 2)\n\n\ndef pay(card, amount):\n    return _fee(card)\n\n\ndef refund(card):\n    return pay(card, 0)\n\n\ndef _fee(card):\n    return card.cost\n";
    const DIFF: &str = "@@ -1,11 +1,15 @@\n class Cart:\n     def total(self):\n-        return 1\n+        return pay(self, 2)\n \n \n-def pay(card):\n-    return card.charge()\n+def pay(card, amount):\n+    return _fee(card)\n \n \n-def _log():\n-    pass\n+def refund(card):\n+    return pay(card, 0)\n+\n+\n+def _fee(card):\n+    return card.cost\n";

    fn with_outline() -> App {
        let mut app = with_review();
        let file = DiffFile { diff: DIFF.into(), old_path: "shop/cart.py".into(), new_path: "shop/cart.py".into(), ..DiffFile::default() };
        let review = Review::new(mr(), &[file], vec![], &[]);
        app.apply(Incoming::Review { key: mr_key(), review: Box::new(review), cached: None });
        let actions = press(&mut app, "O");
        let files = vec![Sides { base: Some("shop/cart.py".into()), head: Some("shop/cart.py".into()) }];
        assert_eq!(actions, vec![Action::LoadOutline { key: mr_key(), base: "aaaa".into(), head: "bbbb".into(), files }]);
        let reading = crate::outline::read(&[("shop/cart.py".into(), BASE.into(), HEAD.into())]);
        app.apply(Incoming::Outline { key: mr_key(), reading });
        app
    }

    fn names(app: &App) -> Vec<String> {
        let outline = app.open.as_ref().unwrap().outline.as_ref().unwrap();
        let Symbols::Ready(reading) = &outline.symbols else { return vec![] };
        reading.changes.iter().map(|change| change.symbol.name.clone()).collect()
    }

    fn place(app: &App) -> Option<Place> {
        let open = app.open.as_ref().unwrap();
        open.review.place_of(open.row().unwrap())
    }

    #[test]
    fn o_lists_the_changed_symbols_and_closes_them_again() {
        let mut app = with_outline();
        assert_eq!(app.focus, Focus::Side);
        assert_eq!(names(&app), ["pay", "refund", "Cart.total", "_log", "_fee"]);
        press(&mut app, "O");
        assert!(app.open.as_ref().unwrap().outline.is_none());
        assert_eq!(app.focus, Focus::Review);
    }

    #[test]
    fn a_full_page_moves_in_the_outline() {
        let selected = |app: &App| app.open.as_ref().unwrap().outline.as_ref().unwrap().selected;
        let mut paged = with_outline();
        let mut last = with_outline();
        press(&mut last, "G");
        paged.handle_key(ctrl('f'));
        assert_eq!(selected(&paged), selected(&last));
        paged.handle_key(ctrl('b'));
        assert_eq!(selected(&paged), 0);
    }

    #[test]
    fn enter_goes_to_the_definition_of_the_symbol() {
        let mut app = with_outline();
        press(&mut app, "j");
        app.handle_key(code(KeyCode::Enter));
        assert_eq!(app.focus, Focus::Review);
        assert_eq!(place(&app), Some(Place::Line { file: 0, new: Some(10), old: None }), "refund's definition");
        press(&mut app, "l");
        press(&mut app, "Gk");
        app.handle_key(code(KeyCode::Enter));
        let Some(Place::Line { old, .. }) = place(&app) else { panic!("on a line") };
        assert_eq!(old, Some(10), "_log was on base line 10");
    }

    #[test]
    fn a_click_on_a_symbol_picks_it_and_a_second_click_goes_to_its_definition() {
        let mut app = with_outline();
        app.focus = Focus::Review;
        assert_eq!(click_row(&mut app, List::Outline, 1), vec![]);
        assert_eq!((app.open.as_ref().unwrap().outline.as_ref().unwrap().selected, app.focus), (1, Focus::Side));
        click_row(&mut app, List::Outline, 1);
        assert_eq!(app.focus, Focus::Review);
        assert_eq!(place(&app), Some(Place::Line { file: 0, new: Some(10), old: None }), "refund's definition");
    }

    #[test]
    fn a_failure_says_so_and_r_reads_again() {
        let mut app = with_outline();
        app.apply(Incoming::Failed { what: Failure::Outline, message: "HTTP 500".into() });
        assert!(render(&mut app, 150, 20).contains("HTTP 500"));
        assert!(matches!(press(&mut app, "r").as_slice(), [Action::LoadOutline { .. }]));
    }

    #[test]
    fn snapshot_outline_pane() {
        let mut app = with_outline();
        insta::assert_snapshot!("outline", render(&mut app, 150, 20));
    }
}
