//! The file tree in the right pane (`t`) and viewed files (`zv`, `m`).
use super::{Action, App, Focus, Open};
use crate::act::{Act, Pane};
use crate::diff::LineKind;
use crate::diff::fold::Fold;
use crate::review::tree::{self, TreeFolds, TreeRow};
use crate::review::{File, Row};
use crossterm::event::KeyEvent;

/// The tree while it is open: which folders the reader flipped, and the row under the cursor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tree {
    pub folds: TreeFolds,
    pub selected: usize,
}

impl Open {
    pub(super) fn with_tree(self, tree: Option<Tree>) -> Self {
        Self { tree, ..self }
    }
}

impl App {
    /// `t`: the tree takes the right pane, on the file under the cursor; `t` again gives it back.
    pub(super) fn toggle_tree(&mut self) {
        let Some(open) = &self.open else { return };
        if open.tree.is_some() {
            self.update_open(|open| open.with_tree(None));
            self.focus = Focus::Review;
            return;
        }
        let here = open.row().and_then(Row::file);
        let path = here.map(|i| open.review.files[i].new_path.clone()).unwrap_or_default();
        let folds = TreeFolds::default().revealing(&path);
        let rows = tree::rows(&open.review.files, &folds);
        let selected = rows.iter().position(|r| matches!(r, TreeRow::File { index, .. } if Some(*index) == here)).unwrap_or(0);
        self.update_open(|open| Open {
            pane: None,
            answer: None,
            pipeline: None,
            outline: None,
            ..open.with_tree(Some(Tree { folds, selected }))
        });
        self.focus = Focus::Side;
    }

    pub(super) fn handle_tree_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let Some(tree) = open.tree.clone() else { return vec![] };
        let rows = self.kept.tree_rows(open);
        let (last, here) = (rows.len().saturating_sub(1), rows.get(tree.selected).cloned());
        let moved = |selected: usize| Some(Tree { selected: selected.min(last), ..tree.clone() });
        let page = super::keys::full_page(self.areas.side).unsigned_abs();
        let next = match self.act_here(Pane::Tree, key) {
            Some(Act::MoveDown) => moved(tree.selected + 1),
            Some(Act::MoveUp) => moved(tree.selected.saturating_sub(1)),
            Some(Act::PageDown) => moved(tree.selected + 10),
            Some(Act::PageUp) => moved(tree.selected.saturating_sub(10)),
            Some(Act::FullPageDown) => moved(tree.selected.saturating_add(page)),
            Some(Act::FullPageUp) => moved(tree.selected.saturating_sub(page)),
            Some(Act::Top) => moved(0),
            Some(Act::Bottom) => moved(last),
            Some(Act::Back | Act::Tree | Act::ClosePane) => None,
            Some(Act::ViewFile) => {
                if let Some(TreeRow::File { index, .. }) = here {
                    return self.view_file(index);
                }
                self.did_nothing();
                Some(tree.clone())
            }
            Some(Act::Open) => match here {
                Some(TreeRow::Folder { path, .. }) => Some(Tree { folds: tree.folds.toggled(&path), ..tree.clone() }),
                Some(TreeRow::File { index, .. }) => return self.show_file(index),
                None => Some(tree.clone()),
            },
            _ => {
                self.did_nothing();
                Some(tree.clone())
            }
        };
        if next.is_none() {
            self.focus = Focus::Review;
        }
        self.update_open(|open| open.with_tree(next));
        vec![]
    }

    /// Puts the review's cursor on file `index`, unfolded, and hands the keys to the review.
    pub(super) fn show_file(&mut self, index: usize) -> Vec<Action> {
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
            (Some(tree), Focus::Side) => match self.kept.tree_rows(open).get(tree.selected) {
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
        self.update_open(|open| open.relaid(|review| review.with_viewed(viewed)));
        let actions = self.set_file_fold(&path, if now_viewed { Fold::Closed } else { Fold::Open });
        self.review_jump_to(|row| matches!(row, Row::File { index: i, .. } if *i == index));
        actions
    }

    /// `m`: `zv`, then, once the file is viewed, on to the first change of the next file not viewed yet, wrapping.
    pub(super) fn view_and_move_on(&mut self) -> Vec<Action> {
        let actions = self.toggle_viewed();
        let Some(open) = &self.open else { return actions };
        let files = open.review.files.clone();
        let unviewed: Vec<bool> = files.iter().map(|file| !open.review.viewed.contains(&file.new_path)).collect();
        let is_viewed_here = open.row().and_then(Row::file).is_some_and(|index| !unviewed[index]);
        if !is_viewed_here {
            return actions;
        }
        if !unviewed.contains(&true) {
            self.toast("every file viewed");
            return actions;
        }
        self.review_jump(true, move |row| is_first_stop(&files, &unviewed, row));
        actions
    }
}

/// Where `m` lands in a file not viewed yet: a changed line, or the header of a file that waits folded.
fn is_first_stop(files: &[File], unviewed: &[bool], row: &Row) -> bool {
    match *row {
        Row::File { index, open: false } => unviewed[index],
        Row::Pair { file, .. } => unviewed[file],
        Row::Line { file, hunk, index } => unviewed[file] && files[file].hunks[hunk].lines[index].kind != LineKind::Context,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crate::tui::app::test_support::*;

    fn tree_row(app: &App) -> usize {
        app.open.as_ref().unwrap().tree.as_ref().unwrap().selected
    }

    #[test]
    fn a_full_page_moves_in_the_tree_and_x_closes_it() {
        let mut paged = with_review();
        let mut last = with_review();
        press(&mut paged, "tg");
        press(&mut last, "tG");
        paged.handle_key(ctrl('f'));
        assert_eq!(tree_row(&paged), tree_row(&last), "a page longer than the tree reaches its end");
        paged.handle_key(ctrl('b'));
        assert_eq!(tree_row(&paged), 0);
        press(&mut paged, "x");
        assert!(paged.open.as_ref().unwrap().tree.is_none());
        assert_eq!(paged.focus, Focus::Review);
    }

    #[test]
    fn t_opens_the_tree_on_the_file_under_the_cursor_and_enter_jumps_back() {
        let mut app = with_review();
        press(&mut app, "]cj");
        press(&mut app, "t");
        assert_eq!(app.focus, Focus::Side);
        let open = app.open.clone().unwrap();
        let rows = app.kept.tree_rows(&open);
        let selected = &rows[open.tree.as_ref().unwrap().selected];
        assert!(matches!(selected, crate::review::tree::TreeRow::File { index: 0, .. }), "{rows:?}");
        press(&mut app, "G");
        app.handle_key(code(KeyCode::Enter));
        assert_eq!(app.focus, Focus::Review, "enter on a file hands the keys to the review");
        let last_file = app.open.as_ref().unwrap().review.files.len() - 1;
        assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::File { index, .. }) if *index == last_file));
        assert!(app.open.as_ref().unwrap().tree.is_some(), "the tree stays open beside the diff");
        press(&mut app, "t");
        assert!(app.open.as_ref().unwrap().tree.is_none());
    }

    #[test]
    fn zv_marks_the_file_viewed_folds_it_and_saves_its_fingerprint() {
        let mut app = with_review();
        press(&mut app, "]cj");
        let actions = press(&mut app, "zv");
        let open = app.open.clone().unwrap();
        let path = open.review.files[0].new_path.clone();
        assert!(open.review.viewed.contains(&path));
        assert!(!open.review.fold.file_is_open(&path), "a viewed file folds");
        assert!(
            matches!(actions.as_slice(), [Action::SaveState { viewed, .. }] if viewed.get(&path) == Some(&open.review.files[0].fingerprint())),
            "{actions:?}"
        );
        press(&mut app, "zv");
        assert!(!app.open.as_ref().unwrap().review.viewed.contains(&path));
        assert!(app.open.as_ref().unwrap().review.fold.file_is_open(&path));
    }

    fn viewed(app: &App) -> Vec<String> {
        app.open.as_ref().unwrap().review.viewed.iter().cloned().collect()
    }

    fn row(app: &App) -> Row {
        app.open.as_ref().unwrap().row().cloned().unwrap()
    }

    #[test]
    fn m_marks_the_file_viewed_as_zv_does_then_moves_to_the_first_change_of_the_next_unviewed_file() {
        let mut marked = with_long_review();
        let mut zv = with_long_review();
        to_step(&mut marked, 3);
        to_step(&mut zv, 3);
        assert_eq!(press(&mut marked, "m"), press(&mut zv, "zv"));
        assert_eq!(viewed(&marked), viewed(&zv));
        assert_eq!(row(&zv), Row::File { index: 0, open: false }, "zv stays on the folded file");
        assert_eq!(row(&marked), Row::Line { file: 1, hunk: 0, index: 0 }, "mod.rs, its removed line");
    }

    #[test]
    fn m_wraps_to_the_first_file_and_stays_once_every_file_is_viewed() {
        let mut app = with_long_review();
        app.review_jump_to(|row| matches!(row, Row::Line { file: 1, .. }));
        press(&mut app, "m");
        assert_eq!(row(&app), Row::Line { file: 0, hunk: 0, index: 0 }, "from the last file, on to the first");
        press(&mut app, "m");
        assert_eq!(row(&app), Row::File { index: 0, open: false });
        assert_eq!(app.live_toast().map(|t| t.text.clone()).as_deref(), Some("every file viewed"));
    }

    #[test]
    fn m_on_a_viewed_file_unmarks_it_and_stays() {
        let mut app = with_long_review();
        to_step(&mut app, 3);
        press(&mut app, "zvm");
        assert_eq!(viewed(&app), [] as [String; 0]);
        assert_eq!(row(&app), Row::File { index: 0, open: true });
    }

    #[test]
    fn m_stops_on_the_header_of_a_next_file_that_opens_folded() {
        let mut app = with_review();
        on_line(&mut app);
        press(&mut app, "m");
        assert_eq!(row(&app), Row::File { index: 1, open: false }, "Cargo.lock waits folded until viewed");
    }

    /// `m` on charge.rs: it is viewed, and the cursor waits on the folded Cargo.lock.
    fn assert_m_moves_on(place: &str, mut app: App) {
        press(&mut app, "m");
        assert_eq!(viewed(&app), ["src/pay/charge.rs"], "{place}");
        assert_eq!(row(&app), Row::File { index: 1, open: false }, "{place}");
    }

    #[test]
    fn m_works_in_zen_in_a_narrow_or_compact_window_and_on_a_thread_row() {
        let mut zen = with_review();
        on_line(&mut zen);
        press(&mut zen, "zz");
        assert_m_moves_on("zen", zen);
        let mut narrow = with_review();
        render(&mut narrow, 100, 30);
        on_line(&mut narrow);
        assert_m_moves_on("narrow", narrow);
        let mut compact = with_review();
        phone(&mut compact);
        on_line(&mut compact);
        assert_m_moves_on("compact", compact);
        let mut thread_row = zen_with_threads(vec![thread_on_line_13("t1", 1)]);
        press(&mut thread_row, "j");
        assert!(thread_row.on_thread_row());
        assert_m_moves_on("thread row", thread_row);
    }

    #[test]
    fn the_tree_counts_only_unresolved_threads() {
        let mut app = with_review();
        press(&mut app, "t");
        let charge_row = |app: &mut App| {
            let screen = render(app, 120, 20);
            screen.lines().filter_map(|l| l.split("││").nth(1)).find(|l| l.contains("charge.rs")).unwrap().to_owned()
        };
        assert!(charge_row(&mut app).contains("+4 −2 ◆1"), "the resolved thread is left out");
        resolve_the_diff_note(&mut app);
        assert!(!charge_row(&mut app).contains('◆'), "no mark once all are resolved");
    }
}
