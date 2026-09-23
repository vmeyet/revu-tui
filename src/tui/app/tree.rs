//! The file tree in the right pane (`t`) and viewed files (`zv`).
use super::{Action, App, Focus, Open};
use crate::diff::fold::Fold;
use crate::review::Row;
use crate::review::tree::{self, TreeFolds, TreeRow};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// The tree while it is open: which folders the reader flipped, and the row under the cursor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tree {
    pub folds: TreeFolds,
    pub selected: usize,
}

impl Open {
    pub fn tree_rows(&self) -> Vec<TreeRow> {
        self.tree.as_ref().map_or_else(Vec::new, |t| tree::rows(&self.review.files, &t.folds))
    }

    fn with_tree(&self, tree: Option<Tree>) -> Self {
        Self { tree, ..self.clone() }
    }
}

impl App {
    /// `t`: the tree takes the right pane, on the file under the cursor; `t` again gives it back.
    pub(super) fn toggle_tree(&mut self) {
        let Some(open) = &self.open else { return };
        if open.tree.is_some() {
            self.open = Some(open.with_tree(None));
            self.focus = Focus::Review;
            return;
        }
        let here = open.row().and_then(Row::file);
        let path = here.map(|i| open.review.files[i].new_path.clone()).unwrap_or_default();
        let folds = TreeFolds::default().revealing(&path);
        let rows = tree::rows(&open.review.files, &folds);
        let selected = rows.iter().position(|r| matches!(r, TreeRow::File { index, .. } if Some(*index) == here)).unwrap_or(0);
        self.open = Some(Open { pane: None, ..open.with_tree(Some(Tree { folds, selected })) });
        self.focus = Focus::Side;
    }

    pub(super) fn handle_tree_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let Some(open) = self.open.clone() else { return vec![] };
        let Some(tree) = open.tree.clone() else { return vec![] };
        let rows = open.tree_rows();
        let last = rows.len().saturating_sub(1);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let moved = |selected: usize| Some(Tree { selected: selected.min(last), ..tree.clone() });
        let next = match key.code {
            KeyCode::Char('j') | KeyCode::Down => moved(tree.selected + 1),
            KeyCode::Char('k') | KeyCode::Up => moved(tree.selected.saturating_sub(1)),
            KeyCode::Char('d') if ctrl => moved(tree.selected + 10),
            KeyCode::Char('u') if ctrl => moved(tree.selected.saturating_sub(10)),
            KeyCode::Char('g') => moved(0),
            KeyCode::Char('G') => moved(last),
            KeyCode::Esc | KeyCode::Char('t') => None,
            KeyCode::Char('v') => match rows.get(tree.selected) {
                Some(TreeRow::File { index, .. }) => return self.view_file(*index),
                _ => Some(tree.clone()),
            },
            KeyCode::Enter => match rows.get(tree.selected) {
                Some(TreeRow::Folder { path, .. }) => Some(Tree { folds: tree.folds.toggled(path), ..tree.clone() }),
                Some(TreeRow::File { index, .. }) => return self.show_file(*index),
                None => Some(tree.clone()),
            },
            _ => Some(tree.clone()),
        };
        if next.is_none() {
            self.focus = Focus::Review;
        }
        self.open = Some(open.with_tree(next));
        vec![]
    }

    /// Puts the review's cursor on file `index`, unfolded, and hands the keys to the review.
    fn show_file(&mut self, index: usize) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let path = open.review.files[index].new_path.clone();
        let actions = if open.review.fold.file_is_open(&path) { vec![] } else { self.set_file_fold(&path, Fold::Open) };
        self.review_jump_to(|row| matches!(row, Row::File { index: i, .. } if *i == index));
        self.focus = Focus::Review;
        actions
    }

    /// `zv`: marks the file under the cursor (in the review or the tree) viewed and folds it, or the reverse.
    pub(super) fn toggle_viewed(&mut self) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let index = match (&open.tree, self.focus) {
            (Some(tree), Focus::Side) => match open.tree_rows().get(tree.selected) {
                Some(TreeRow::File { index, .. }) => Some(*index),
                _ => None,
            },
            _ => open.row().and_then(Row::file),
        };
        let Some(index) = index else { return vec![] };
        let path = open.review.files[index].new_path.clone();
        let mut viewed = open.review.viewed.clone();
        let now_viewed = viewed.insert(path.clone());
        if !now_viewed {
            viewed.remove(&path);
        }
        self.open = Some(open.with_review(open.review.with_viewed(viewed)));
        let actions = self.set_file_fold(&path, if now_viewed { Fold::Closed } else { Fold::Open });
        self.review_jump_to(|row| matches!(row, Row::File { index: i, .. } if *i == index));
        actions
    }
}
