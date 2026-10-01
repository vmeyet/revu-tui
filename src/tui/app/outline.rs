//! The outline pane (`O`): the functions, methods and classes the MR changes, by file, riskiest first.
use super::{Action, App, Focus, Open};
use crate::diff::fold::FoldState;
use crate::outline::{Change, State};
use crate::review::{Place, Review, Row, Side};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::sync::Arc;

/// The pane while it is open: what was read, the symbol under the cursor, and whether private ones list too.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outline {
    pub symbols: Symbols,
    pub selected: usize,
    pub all: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Symbols {
    Waiting,
    /// Shared, so moving the cursor copies none of them.
    Ready(Arc<[Change]>),
    Failed(String),
}

impl Outline {
    fn waiting() -> Self {
        Self { symbols: Symbols::Waiting, selected: 0, all: false }
    }

    /// The changes listed: the public ones, or every one after `a`.
    pub fn shown(&self) -> Vec<&Change> {
        match &self.symbols {
            Symbols::Ready(changes) => changes.iter().filter(|c| self.all || c.symbol.public).collect(),
            _ => vec![],
        }
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
        let Some(open) = &self.open else { return vec![] };
        let Some(outline) = &open.outline else { return vec![] };
        let last = outline.shown().len().saturating_sub(1);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let selected = match key.code {
            KeyCode::Char('j') | KeyCode::Down => outline.selected + 1,
            KeyCode::Char('k') | KeyCode::Up => outline.selected.saturating_sub(1),
            KeyCode::Char('d') if ctrl => outline.selected + 10,
            KeyCode::Char('u') if ctrl => outline.selected.saturating_sub(10),
            KeyCode::Char('g') => 0,
            KeyCode::Char('G') => last,
            KeyCode::Enter => return self.jump_to_symbol(),
            KeyCode::Char('a') => {
                let outline = Outline { all: !outline.all, selected: 0, ..outline.clone() };
                self.update_open(|open| open.with_outline(Some(outline)));
                return vec![];
            }
            KeyCode::Char('r') => return self.read_outline(),
            KeyCode::Esc | KeyCode::Char('O' | 'x') => {
                self.close_outline();
                return vec![];
            }
            _ => return vec![],
        };
        let outline = Outline { selected: selected.min(last), ..outline.clone() };
        self.update_open(|open| open.with_outline(Some(outline)));
        vec![]
    }

    /// `enter`: the diff's cursor on the symbol, its file and hunk opened; the keys go to the diff.
    fn jump_to_symbol(&mut self) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let Some(outline) = &open.outline else { return vec![] };
        let Some(change) = outline.shown().get(outline.selected).copied() else { return vec![] };
        let Some(target) = row_of(&open.review, change) else {
            self.toast("its lines are not in the diff");
            return vec![];
        };
        let actions = self.jump_to_row(&target);
        self.focus = Focus::Review;
        actions
    }

    /// The symbols read for `key`, unless the reader moved on to another MR meanwhile.
    pub(super) fn apply_outline(&mut self, key: &super::MrKey, changes: Vec<Change>) {
        if self.open.as_ref().is_some_and(|o| &o.key == key) {
            self.settle_outline(Symbols::Ready(changes.into()));
        }
    }

    /// The symbols read, or why they could not be; dropped once the pane closed.
    pub(super) fn settle_outline(&mut self, symbols: Symbols) {
        if self.outline_open() {
            self.update_open(|open| open.with_outline(Some(Outline { symbols, ..Outline::waiting() })));
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

/// The first row showing one of the symbol's lines, every fold open: on head, or on base for a removed one.
fn row_of(review: &Review, change: &Change) -> Option<Row> {
    let file = review.files.iter().position(|f| f.new_path == change.path || f.old_path == change.path)?;
    let side = if change.state == State::Removed { Side::Old } else { Side::New };
    let (first, last) = change.symbol.lines;
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
    const HEAD: &str = "class Cart:\n    def total(self):\n        return 2\n\n\ndef pay(card, amount):\n    return card.charge()\n\n\ndef refund(card):\n    return card.back()\n";
    const DIFF: &str = "@@ -1,11 +1,11 @@\n class Cart:\n     def total(self):\n-        return 1\n+        return 2\n \n \n-def pay(card):\n+def pay(card, amount):\n     return card.charge()\n \n \n-def _log():\n-    pass\n+def refund(card):\n+    return card.back()\n";

    fn with_outline() -> App {
        let mut app = with_review();
        let file = DiffFile { diff: DIFF.into(), old_path: "shop/cart.py".into(), new_path: "shop/cart.py".into(), ..DiffFile::default() };
        let review = Review::new(mr(), &[file], vec![], &[]);
        app.apply(Incoming::Review { key: mr_key(), review: Box::new(review), cached: None });
        let actions = press(&mut app, "O");
        let files = vec![crate::outline::Sides { base: Some("shop/cart.py".into()), head: Some("shop/cart.py".into()) }];
        assert_eq!(actions, vec![Action::LoadOutline { key: mr_key(), base: "aaaa".into(), head: "bbbb".into(), files }]);
        app.apply(Incoming::Outline { key: mr_key(), changes: crate::outline::changes("shop/cart.py", BASE, HEAD) });
        app
    }

    fn names(app: &App) -> Vec<String> {
        app.open.as_ref().unwrap().outline.as_ref().unwrap().shown().iter().map(|c| c.symbol.name.clone()).collect()
    }

    #[test]
    fn o_lists_public_symbols_riskiest_first_and_a_lists_private_ones_too() {
        let mut app = with_outline();
        assert_eq!(app.focus, Focus::Side);
        assert_eq!(names(&app), vec!["pay", "refund", "Cart.total"]);
        press(&mut app, "a");
        assert_eq!(names(&app), vec!["pay", "refund", "Cart.total", "_log"]);
        press(&mut app, "O");
        assert!(app.open.as_ref().unwrap().outline.is_none());
        assert_eq!(app.focus, Focus::Review);
    }

    #[test]
    fn enter_puts_the_diff_cursor_on_the_symbol_head_line_or_base_line_when_removed() {
        let mut app = with_outline();
        app.handle_key(code(KeyCode::Enter));
        assert_eq!(app.focus, Focus::Review);
        let open = app.open.as_ref().unwrap();
        assert_eq!(open.review.place_of(open.row().unwrap()), Some(Place::Line { file: 0, new: Some(6), old: None }));
        press(&mut app, "l");
        press(&mut app, "aG");
        app.handle_key(code(KeyCode::Enter));
        let open = app.open.as_ref().unwrap();
        let Some(Place::Line { old, .. }) = open.review.place_of(open.row().unwrap()) else { panic!("on a line") };
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
}
