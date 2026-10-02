//! The outline pane (`O`): the functions, methods and classes the MR changes, as a call tree or a flat list.
use super::{Action, App, Focus, Open};
use crate::diff::fold::FoldState;
mod rows;

pub use rows::{Entry, PaneLine, Rows};

use crate::outline::{Direction, Reading, Shape};
use crate::review::{Place, Review, Row, Side};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use rows::View;
use std::collections::BTreeSet;
use std::sync::Arc;

/// The pane while it is open: what was read, the row under the cursor, how it shows, and the rows that makes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outline {
    pub symbols: Symbols,
    pub selected: usize,
    /// Private symbols too, `a`.
    pub all: bool,
    /// The list of changes instead of the tree, `t`.
    pub flat: bool,
    /// Every unchanged function called too, `s`.
    pub stack: bool,
    pub direction: Direction,
    /// The tree's folded branches, each by its child indexes from the top.
    folded: BTreeSet<Vec<usize>>,
    /// Built whenever the rest changes, so drawing only slices it.
    pub rows: Arc<Rows>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Symbols {
    Waiting,
    /// Shared, so moving the cursor copies none of it.
    Ready(Arc<Reading>),
    Failed(String),
}

impl Outline {
    fn waiting() -> Self {
        Self {
            symbols: Symbols::Waiting,
            selected: 0,
            all: false,
            flat: false,
            stack: false,
            direction: Direction::Calls,
            folded: BTreeSet::new(),
            rows: Arc::default(),
        }
    }

    /// The rows built again for what changed, the cursor kept on them.
    fn rebuilt(self) -> Self {
        let Symbols::Ready(reading) = &self.symbols else { return self };
        let view = View { shape: Shape { direction: self.direction, all: self.all, stack: self.stack }, flat: self.flat };
        let rows = Rows::build(reading, view, &self.folded);
        let selected = self.selected.min(rows.entries.len().saturating_sub(1));
        Self { rows: Arc::new(rows), selected, ..self }
    }

    /// The same pane shown another way: the cursor back on top, every branch open.
    fn shown_as(&self, change: impl FnOnce(Self) -> Self) -> Self {
        change(Self { selected: 0, folded: BTreeSet::new(), ..self.clone() }).rebuilt()
    }

    /// `zo` `zc` `za` on the branch under the cursor; `None` opens or closes it, whichever it is not.
    fn folding(&self, open: Option<bool>) -> Self {
        let Some(entry) = self.rows.entries.get(self.selected).filter(|e| e.children) else { return self.clone() };
        let mut folded = self.folded.clone();
        if open.unwrap_or(entry.folded) {
            folded.remove(&entry.at);
        } else {
            folded.insert(entry.at.clone());
        }
        Self { folded, ..self.clone() }.rebuilt()
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
        self.update_open(|open| Open { pane: None, tree: None, answer: None, pipeline: None, ..open });
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
        let Some(outline) = self.open.as_ref().and_then(|o| o.outline.as_ref()) else { return vec![] };
        let last = outline.rows.entries.len().saturating_sub(1);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let moved = |selected: usize| Outline { selected: selected.min(last), ..outline.clone() };
        let next = match key.code {
            KeyCode::Char('j') | KeyCode::Down => moved(outline.selected + 1),
            KeyCode::Char('k') | KeyCode::Up => moved(outline.selected.saturating_sub(1)),
            KeyCode::Char('d') if ctrl => moved(outline.selected + 10),
            KeyCode::Char('u') if ctrl => moved(outline.selected.saturating_sub(10)),
            KeyCode::Char('g') => moved(0),
            KeyCode::Char('G') => moved(last),
            KeyCode::Char('a') => outline.shown_as(|o| Outline { all: !o.all, ..o }),
            KeyCode::Char('t') => outline.shown_as(|o| Outline { flat: !o.flat, ..o }),
            KeyCode::Char('u') => outline.shown_as(|o| Outline { direction: o.direction.reversed(), ..o }),
            KeyCode::Char('s') => outline.shown_as(|o| Outline { stack: !o.stack, ..o }),
            KeyCode::Enter => return self.jump_from_outline(),
            KeyCode::Char('r') => return self.read_outline(),
            KeyCode::Esc | KeyCode::Char('O' | 'x') => {
                self.close_outline();
                return vec![];
            }
            _ => return vec![],
        };
        self.update_open(|open| open.with_outline(Some(next)));
        vec![]
    }

    /// `zo` `zc` `za` while the outline has the keys.
    pub(super) fn fold_outline(&mut self, key: char) {
        let open = match key {
            'o' => Some(true),
            'c' => Some(false),
            'a' => None,
            _ => return,
        };
        let Some(outline) = self.open.as_ref().and_then(|o| o.outline.as_ref()) else { return };
        let folded = outline.folding(open);
        self.update_open(|open| open.with_outline(Some(folded)));
    }

    /// `enter`: the diff's cursor on the call under the cursor, or on the symbol's definition at the top; the keys go to the diff.
    fn jump_from_outline(&mut self) -> Vec<Action> {
        let Some(outline) = self.open.as_ref().and_then(|o| o.outline.as_ref()) else { return vec![] };
        let Symbols::Ready(reading) = &outline.symbols else { return vec![] };
        let Some(entry) = outline.rows.entries.get(outline.selected) else { return vec![] };
        let place = match (&entry.site, entry.change.map(|c| &reading.changes[c])) {
            (Some(site), _) => Some((site.path.clone(), site.side, (site.line, site.line))),
            (None, Some(change)) => Some((change.path.clone(), change.side(), change.symbol.lines)),
            (None, None) => None,
        };
        let Some((path, side, lines)) = place else { return vec![] };
        let Some(target) = self.open.as_ref().and_then(|open| row_at(&open.review, &path, side, lines)) else {
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
        .map(|f| crate::outline::Sides {
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
    #![allow(clippy::unwrap_used, clippy::expect_used)]
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
        let files = vec![crate::outline::Sides { base: Some("shop/cart.py".into()), head: Some("shop/cart.py".into()) }];
        assert_eq!(actions, vec![Action::LoadOutline { key: mr_key(), base: "aaaa".into(), head: "bbbb".into(), files }]);
        let reading = crate::outline::read(&[("shop/cart.py".into(), BASE.into(), HEAD.into())]);
        app.apply(Incoming::Outline { key: mr_key(), reading });
        app
    }

    /// Each row as its lines and the name of its change, or a word for the rest.
    fn rows(app: &App) -> Vec<String> {
        let outline = app.open.as_ref().unwrap().outline.as_ref().unwrap();
        let Symbols::Ready(reading) = &outline.symbols else { return vec![] };
        let name = |entry: &super::Entry| entry.change.map_or("?".into(), |c| reading.changes[c].symbol.name.clone());
        outline.rows.entries.iter().map(|entry| format!("{}{}", entry.lines, name(entry))).collect()
    }

    fn place(app: &App) -> Option<Place> {
        let open = app.open.as_ref().unwrap();
        open.review.place_of(open.row().unwrap())
    }

    #[test]
    fn o_shows_the_call_tree_t_the_flat_list_and_a_private_symbols_too() {
        let mut app = with_outline();
        assert_eq!(app.focus, Focus::Side);
        assert_eq!(rows(&app), ["refund", "└─ pay", "   └─ _fee", "Cart.total", "└─ pay"]);
        press(&mut app, "a");
        assert_eq!(rows(&app)[5..], ["_log"]);
        press(&mut app, "t");
        assert_eq!(rows(&app), ["pay", "refund", "Cart.total", "_log", "_fee"]);
        press(&mut app, "a");
        assert_eq!(rows(&app), ["pay", "refund", "Cart.total"]);
        press(&mut app, "O");
        assert!(app.open.as_ref().unwrap().outline.is_none());
        assert_eq!(app.focus, Focus::Review);
    }

    #[test]
    fn u_lists_who_calls_each_symbol() {
        let mut app = with_outline();
        press(&mut app, "u");
        assert_eq!(rows(&app), ["pay", "├─ refund", "└─ Cart.total"]);
    }

    #[test]
    fn s_shows_the_whole_stack_and_says_so_in_the_title() {
        let mut app = with_outline();
        press(&mut app, "s");
        assert!(app.open.as_ref().unwrap().outline.as_ref().unwrap().stack);
        assert!(render(&mut app, 150, 20).contains("Outline · calls · public · stack"));
        press(&mut app, "s");
        assert!(render(&mut app, 150, 20).contains("Outline · calls · public ─"));
    }

    #[test]
    fn zc_folds_the_branch_under_the_cursor_and_zo_opens_it() {
        let mut app = with_outline();
        press(&mut app, "zc");
        assert_eq!(rows(&app)[..2], ["refund", "Cart.total"]);
        press(&mut app, "zo");
        assert_eq!(rows(&app)[..2], ["refund", "└─ pay"]);
    }

    #[test]
    fn enter_on_a_call_goes_to_its_line_and_on_a_symbol_to_its_definition() {
        let mut app = with_outline();
        press(&mut app, "j");
        app.handle_key(code(KeyCode::Enter));
        assert_eq!(app.focus, Focus::Review);
        assert_eq!(place(&app), Some(Place::Line { file: 0, new: Some(11), old: None }), "the call to pay in refund");
        press(&mut app, "l");
        press(&mut app, "g");
        app.handle_key(code(KeyCode::Enter));
        assert_eq!(place(&app), Some(Place::Line { file: 0, new: Some(10), old: None }), "refund's definition");
        press(&mut app, "l");
        press(&mut app, "taGk");
        app.handle_key(code(KeyCode::Enter));
        let Some(Place::Line { old, .. }) = place(&app) else { panic!("on a line") };
        assert_eq!(old, Some(10), "_log was on base line 10");
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

    #[test]
    fn snapshot_outline_flat() {
        let mut app = with_outline();
        press(&mut app, "t");
        insta::assert_snapshot!("outline_flat", render(&mut app, 150, 20));
    }
}
