//! What frames read from the open review, built again only when what it is built from changed:
//! moving a cursor or scrolling rebuilds nothing.
use super::pane::ConversationList;
use super::{Open, Pane, Versions};
use crate::review::tree::{self, TreeFolds, TreeRow};
use crate::review::{Draft, File, Markers, Thread};
use crate::tui::prose_view::{self, Rows};
use crate::tui::theme::Theme;
use std::sync::Arc;

/// A shared slice compared by identity: a review keeps it until a fetch replaces it, and the held
/// copy stops its memory from being reused by the next one.
#[derive(Debug)]
struct ByIdentity<T: ?Sized>(Arc<T>);

impl<T: ?Sized> ByIdentity<T> {
    pub(crate) fn new(shared: &Arc<T>) -> Self {
        Self(Arc::clone(shared))
    }
}

impl<T: ?Sized> PartialEq for ByIdentity<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// The parts of the review the marks and the pane's conversations are made of.
#[derive(Debug, PartialEq)]
pub(crate) struct ReviewInputs {
    threads: ByIdentity<[Thread]>,
    files: ByIdentity<[File]>,
    drafts: Vec<Draft>,
}

impl ReviewInputs {
    pub(crate) fn from_open(open: &Open) -> Self {
        Self {
            threads: ByIdentity::new(&open.review.threads),
            files: ByIdentity::new(&open.review.files),
            drafts: open.review.drafts.clone(),
        }
    }
}

/// A value and what it was built from, built again when asked with anything else.
#[derive(Debug)]
struct KeptValue<K, V>(Option<(K, V)>);

impl<K, V> Default for KeptValue<K, V> {
    fn default() -> Self {
        Self(None)
    }
}

impl<K: PartialEq, V> KeptValue<K, V> {
    fn get_or_build(&mut self, from: K, build: impl FnOnce() -> V) -> &mut V {
        if self.0.as_ref().is_some_and(|(built_from, _)| *built_from != from) {
            self.0 = None;
        }
        &mut self.0.get_or_insert_with(|| (from, build())).1
    }
}

#[derive(Debug, Default)]
pub(crate) struct Kept {
    markers: KeptValue<ReviewInputs, Markers>,
    pane_view: KeptValue<(ReviewInputs, Pane), ConversationList>,
    tree: KeptValue<(ByIdentity<[File]>, TreeFolds), Vec<TreeRow>>,
    /// One per layout, so `D` in the prose view goes back and forth without rendering again.
    prose: KeptValue<(ByIdentity<Versions>, usize, Theme), Rows>,
    prose_beside: KeptValue<(ByIdentity<Versions>, usize, Theme), Rows>,
}

impl Kept {
    /// What the anchor column of every line shows.
    pub(crate) fn markers(&mut self, open: &Open) -> &Markers {
        self.markers.get_or_build(ReviewInputs::from_open(open), || open.review.markers())
    }

    /// `Open::pane_view`, kept while the review and the pane stay the same; its cursor only picks the focused stop.
    pub(crate) fn pane_view(&mut self, open: &Open) -> Option<&ConversationList> {
        let pane = open.pane.as_ref()?;
        let view = self.pane_view.get_or_build((ReviewInputs::from_open(open), pane.unmoved()), || open.pane_view_of(pane));
        view.focused = super::pane::focused(&view.entries, pane);
        Some(view)
    }

    /// The rows of the file tree; none while it is closed.
    pub(crate) fn tree_rows(&mut self, open: &Open) -> &[TreeRow] {
        let Some(folds) = open.tree.as_ref().map(|t| &t.folds) else { return &[] };
        self.tree.get_or_build((ByIdentity::new(&open.review.files), folds.clone()), || tree::rows(&open.review.files, folds))
    }

    /// The prose view's rows in `width` columns, inline or `beside`, rendered again only for other texts, width or theme.
    pub(crate) fn prose_rows(&mut self, versions: &Arc<Versions>, width: usize, beside: bool, theme: Theme) -> &Rows {
        let kept = if beside { &mut self.prose_beside } else { &mut self.prose };
        kept.get_or_build((ByIdentity::new(versions), width, theme), || prose_view::rows(versions, width, beside, theme))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::tree;
    use crate::tui::app::test_support::*;

    #[test]
    fn a_kept_value_builds_once_per_input() {
        let mut memo = KeptValue::default();
        let mut builds = 0;
        let mut get = |memo: &mut KeptValue<u8, u8>, from: u8| {
            *memo.get_or_build(from, || {
                builds += 1;
                from * 2
            })
        };
        assert_eq!((get(&mut memo, 1), get(&mut memo, 1), get(&mut memo, 2)), (2, 2, 4));
        assert_eq!(builds, 2, "the same input reuses the value");
    }

    #[test]
    fn shared_slices_compare_by_identity_not_by_value() {
        let (a, b): (Arc<[u8]>, Arc<[u8]>) = (Arc::from([1]), Arc::from([1]));
        assert_eq!(ByIdentity::new(&a), ByIdentity::new(&a));
        assert_ne!(ByIdentity::new(&a), ByIdentity::new(&b), "equal contents from another fetch still count as new");
    }

    #[test]
    fn markers_follow_a_new_draft() {
        let mut app = with_review();
        let before = app.kept.markers(app.open.as_ref().unwrap()).clone();
        on_line(&mut app);
        press(&mut app, "c");
        type_text(&mut app, "nit");
        let open = app.open.as_ref().unwrap();
        assert_ne!(*app.kept.markers(open), before, "the draft marks its line at once");
        assert_eq!(*app.kept.markers(open), open.review.markers());
    }

    /// The kept pane view and the one built afresh, owned so they compare.
    fn both_pane_views(app: &mut App) -> (Option<ConversationList>, Option<ConversationList>) {
        let open = app.open.as_ref().unwrap();
        (app.kept.pane_view(open).cloned(), open.pane_view())
    }

    #[test]
    fn the_pane_view_follows_its_cursor_and_its_filter() {
        let mut app = with_review();
        press(&mut app, "T");
        let (kept, fresh) = both_pane_views(&mut app);
        assert_eq!(kept, fresh);
        press(&mut app, "j");
        let (moved, fresh) = both_pane_views(&mut app);
        assert_eq!(moved, fresh);
        assert_ne!(moved.map(|v| v.focused), kept.map(|v| v.focused), "the cursor moved to the next stop");
        press(&mut app, "m");
        let (kept, fresh) = both_pane_views(&mut app);
        assert_eq!(kept, fresh, "only mine");
    }

    #[test]
    fn tree_rows_follow_a_folded_folder() {
        let mut app = with_review();
        assert_eq!(app.kept.tree_rows(app.open.as_ref().unwrap()), [] as [TreeRow; 0], "no tree, no rows");
        press(&mut app, "t");
        let before = app.kept.tree_rows(app.open.as_ref().unwrap()).to_vec();
        press(&mut app, "kk");
        app.handle_key(code(KeyCode::Enter));
        let open = app.open.as_ref().unwrap();
        assert_ne!(app.kept.tree_rows(open), before.as_slice());
        assert_eq!(app.kept.tree_rows(open), tree::rows(&open.review.files, &open.tree.as_ref().unwrap().folds));
    }
}
