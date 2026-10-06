//! The prose view (`v`): the Markdown file under the cursor drawn rendered in the diff area (`specs/11-prose-diff.md`).
use super::{Action, App, MrKey, Open};
use crate::outline::Sides;
use crate::review::FileKind;
use crossterm::event::{KeyCode, KeyEvent};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prose {
    pub sides: Sides,
    pub texts: Texts,
    /// The first row in view; the draw keeps it inside the rows.
    pub scroll: usize,
    /// Every unchanged run shown, `zR`.
    pub unfolded: bool,
}

impl Prose {
    /// The file's path at head, at base for a deleted file.
    pub fn path(&self) -> &str {
        self.sides.head.as_deref().or(self.sides.base.as_deref()).unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Texts {
    Waiting,
    /// Shared, so the rows rendered from it are kept until another read replaces it.
    Ready(Arc<Versions>),
    Failed(String),
}

/// The whole file at base and at head, empty on the side it does not exist.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Versions {
    pub base: String,
    pub head: String,
}

impl Open {
    fn with_prose(self, prose: Option<Prose>) -> Self {
        Self { prose, ..self }
    }
}

impl App {
    pub(super) fn prose_open(&self) -> bool {
        self.open.as_ref().is_some_and(|o| o.prose.is_some())
    }

    /// `v`: the Markdown file under the cursor as prose.
    pub(super) fn open_prose(&mut self) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let Some(file) = open.row().and_then(crate::review::Row::file).map(|i| &open.review.files[i]) else { return vec![] };
        let sides = Sides {
            base: (file.kind != FileKind::Added).then(|| file.old_path.clone()),
            head: (file.kind != FileKind::Deleted).then(|| file.new_path.clone()),
        };
        let prose = Prose { sides, texts: Texts::Waiting, scroll: 0, unfolded: false };
        if !crate::syntax::is_markdown(prose.path()) {
            self.toast("prose shows Markdown files only");
            return vec![];
        }
        self.read_prose(prose)
    }

    /// The view waits for both sides of its file, read again.
    fn read_prose(&mut self, prose: Prose) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let refs = &open.review.mr.refs;
        let action =
            Action::LoadProse { key: open.key.clone(), base: refs.base.clone(), head: refs.head.clone(), sides: prose.sides.clone() };
        self.update_open(|open| open.with_prose(Some(Prose { texts: Texts::Waiting, ..prose })));
        vec![action]
    }

    pub(super) fn handle_prose_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let Some(prose) = self.open.as_ref().and_then(|o| o.prose.clone()) else { return vec![] };
        if let Some(scroll) = super::keys::scroll_key(prose.scroll, usize::MAX, key) {
            self.update_prose(|prose| Prose { scroll, ..prose });
            return vec![];
        }
        match key.code {
            KeyCode::Char('r') => self.read_prose(prose),
            KeyCode::Char('D') => self.toggle_side_by_side(),
            KeyCode::Char('v' | 'x') | KeyCode::Esc => {
                self.update_open(|open| open.with_prose(None));
                vec![]
            }
            _ => vec![],
        }
    }

    /// `zR` shows every unchanged run and `zM` folds them again; `zz` and `zh` act as anywhere; the diff's other
    /// prefixed keys would move a cursor out of sight, so they do nothing.
    pub(super) fn prose_prefixed(&mut self, prefix: char, c: char) -> Vec<Action> {
        match (prefix, c) {
            ('z', 'R' | 'M') => self.update_prose(|prose| Prose { unfolded: c == 'R', ..prose }),
            ('z', 'z') => return self.toggle_zen(),
            ('z', 'h') => self.header_folded = !self.header_folded,
            _ => {}
        }
        vec![]
    }

    /// Both sides of `sides`, unless the reader moved on to another MR or file meanwhile.
    pub(super) fn apply_prose(&mut self, key: &MrKey, sides: &Sides, versions: Versions) {
        if self.open.as_ref().is_some_and(|o| &o.key == key && o.prose.as_ref().is_some_and(|p| &p.sides == sides)) {
            self.settle_prose(Texts::Ready(Arc::new(versions)));
        }
    }

    /// What was read, or why it could not be; dropped once the view closed.
    pub(super) fn settle_prose(&mut self, texts: Texts) {
        self.update_prose(|prose| Prose { texts, ..prose });
    }

    fn update_prose(&mut self, change: impl FnOnce(Prose) -> Prose) {
        self.update_open(|open| {
            let prose = open.prose.clone().map(change);
            open.with_prose(prose)
        });
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::tui::app::test_support::*;

    const BASE: &str = "# Widgets\n\nAcme widgets.\n";
    const HEAD: &str = "# Widgets\n\nAcme widgets for nina.\n\n- one\n- two\n";

    fn with_markdown(change: FileKind) -> App {
        let mut app = with_review();
        let diff = "@@ -1,3 +1,6 @@\n # Widgets\n \n-Acme widgets.\n+Acme widgets for nina.\n+\n+- one\n+- two\n";
        let (old_path, new_path) = ("docs/old.md".into(), "docs/README.md".into());
        let file = DiffFile { diff: diff.into(), old_path, new_path, change, ..DiffFile::default() };
        let review = Review::new(mr(), &[file], vec![], &[]);
        app.apply(Incoming::Review { key: mr_key(), review: Box::new(review), cached: None });
        app
    }

    fn prose(app: &App) -> Option<&Prose> {
        app.open.as_ref().unwrap().prose.as_ref()
    }

    fn sides(base: Option<&str>, head: Option<&str>) -> Sides {
        Sides { base: base.map(Into::into), head: head.map(Into::into) }
    }

    fn read(app: &mut App) {
        read_texts(app, BASE, HEAD);
    }

    fn read_texts(app: &mut App, base: &str, head: &str) {
        let versions = Versions { base: base.into(), head: head.into() };
        app.apply(Incoming::Prose { key: mr_key(), sides: prose(app).unwrap().sides.clone(), versions });
    }

    /// A README with a removed heading, an edited paragraph and an added bullet.
    #[cfg(feature = "prose")]
    fn read_edited(app: &mut App) {
        let base = "# Widgets\n\n## Install\n\nRun the installer once.\n\n- one\n- two\n\nThe end.\n";
        let head = base.replace("## Install\n\n", "").replace("once", "twice").replace("- two\n", "- two\n- three\n");
        read_texts(app, base, &head);
    }

    #[test]
    fn v_reads_both_sides_of_the_file_under_the_cursor_and_v_goes_back_to_the_diff() {
        let mut app = with_markdown(FileKind::Renamed);
        let selected = app.open.as_ref().unwrap().selected;
        let actions = press(&mut app, "v");
        let sides = sides(Some("docs/old.md"), Some("docs/README.md"));
        assert_eq!(actions, vec![Action::LoadProse { key: mr_key(), base: "aaaa".into(), head: "bbbb".into(), sides }]);
        app.apply(Incoming::Prose { key: mr_key(), sides: super::tests::sides(None, Some("other.md")), versions: Versions::default() });
        assert_eq!(prose(&app).unwrap().texts, Texts::Waiting, "an answer for another file is dropped");
        read(&mut app);
        assert!(matches!(prose(&app).unwrap().texts, Texts::Ready(_)));
        press(&mut app, "v");
        assert_eq!((prose(&app), app.open.as_ref().unwrap().selected), (None, selected));
    }

    #[test]
    fn an_added_file_has_no_base_side_and_a_deleted_one_no_head_side() {
        let mut added = with_markdown(FileKind::Added);
        assert!(
            matches!(press(&mut added, "v").as_slice(), [Action::LoadProse { sides, .. }] if *sides == super::tests::sides(None, Some("docs/README.md")))
        );
        let mut deleted = with_markdown(FileKind::Deleted);
        press(&mut deleted, "v");
        assert_eq!((prose(&deleted).unwrap().path(), &prose(&deleted).unwrap().sides), ("docs/old.md", &sides(Some("docs/old.md"), None)));
    }

    #[test]
    fn v_on_a_file_that_is_not_markdown_says_so() {
        let mut app = with_review();
        assert_eq!(press(&mut app, "v"), vec![]);
        assert_eq!(app.live_toast().map(|t| t.text.as_str()), Some("prose shows Markdown files only"));
    }

    #[test]
    fn a_failure_says_so_and_r_reads_again() {
        let mut app = with_markdown(FileKind::Modified);
        press(&mut app, "v");
        app.apply(Incoming::Failed { what: Failure::Prose, message: "HTTP 500".into() });
        assert!(render(&mut app, 120, 20).contains("HTTP 500"));
        assert!(matches!(press(&mut app, "r").as_slice(), [Action::LoadProse { .. }]));
        assert_eq!(prose(&app).unwrap().texts, Texts::Waiting);
    }

    #[test]
    fn zr_shows_every_unchanged_run_zm_folds_them_and_the_hidden_diff_stays_put() {
        let mut app = with_markdown(FileKind::Modified);
        let selected = app.open.as_ref().unwrap().selected;
        press(&mut app, "v");
        press(&mut app, "zR");
        assert!(prose(&app).unwrap().unfolded);
        press(&mut app, "zM]cza");
        assert!(!prose(&app).unwrap().unfolded);
        assert_eq!(app.open.as_ref().unwrap().selected, selected, "the hidden diff does not move");
    }

    #[cfg(feature = "prose")]
    #[test]
    fn g_scrolls_to_the_last_screen_and_g_back_to_the_top() {
        let mut app = with_markdown(FileKind::Modified);
        press(&mut app, "v");
        read(&mut app);
        press(&mut app, "G");
        let screen = render(&mut app, 120, 12);
        assert!(screen.contains("• two") && !screen.contains("Widgets"), "{screen}");
        press(&mut app, "Gjkg");
        assert_eq!(prose(&app).unwrap().scroll, 0);
    }

    #[cfg(feature = "prose")]
    #[test]
    fn snapshot_prose_view() {
        let mut app = with_markdown(FileKind::Modified);
        press(&mut app, "v");
        read(&mut app);
        insta::assert_snapshot!("prose", render(&mut app, 120, 20));
    }

    #[cfg(feature = "prose")]
    #[test]
    fn snapshot_prose_side_by_side() {
        let mut app = with_markdown(FileKind::Modified);
        press(&mut app, "vD");
        read_edited(&mut app);
        insta::assert_snapshot!("prose_side_by_side", render(&mut app, 180, 26));
    }

    #[cfg(feature = "prose")]
    #[test]
    fn d_in_a_narrow_pane_keeps_prose_inline_and_says_so() {
        let mut app = with_markdown(FileKind::Modified);
        press(&mut app, "v");
        read_edited(&mut app);
        render(&mut app, 100, 30);
        press(&mut app, "D");
        assert_eq!(app.live_toast().map(|t| t.text.as_str()), Some("side by side needs a wider window"));
        let screen = render(&mut app, 100, 30);
        assert!(screen.lines().any(|row| row.contains("once") && !row.contains("twice")), "{screen}");
        assert!(prose(&app).is_some() && app.open.as_ref().unwrap().review.side_by_side, "the choice stays for a wider window");
    }
}
