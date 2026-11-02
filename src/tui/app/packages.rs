//! The packages of a lockfile in the diff area, in place of its raw diff: opened and left as the prose view is.
use super::{Action, App, MrKey, Open};
use crate::act::{Act, Pane};
use crate::diff::fold::Fold;
use crate::forge::Sha;
use crate::lockfile::Summary;
use crate::outline::Sides;
use crate::review::Row;
use crossterm::event::KeyEvent;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Packages {
    pub(crate) sides: Sides,
    /// `None` until the file is read; shared, so a key does not copy it.
    pub(crate) summary: Option<Arc<Summary>>,
    /// The first row in view; the draw keeps it inside the rows.
    pub(crate) scroll: usize,
}

impl App {
    pub(super) fn packages_open(&self) -> bool {
        self.open.as_ref().is_some_and(|o| o.packages.is_some())
    }

    /// The lockfile under the cursor as its packages; its file opens, so leaving shows the raw diff.
    pub(super) fn open_packages(&mut self) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let Some(file) = open.row().and_then(Row::file).map(|i| &open.review.files[i]) else { return vec![] };
        let (sides, path) = (Sides::of(file), file.new_path.clone());
        let unfolded = if open.review.fold.file_is_open(&path) { vec![] } else { self.set_file_fold(&path, Fold::Open) };
        unfolded.into_iter().chain(self.read_packages(sides)).collect()
    }

    fn read_packages(&mut self, sides: Sides) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let (key, refs) = (open.key.clone(), &open.review.mr.refs);
        let action = Action::LoadPackages { key, base: refs.base.clone(), head: refs.head.clone(), sides: sides.clone() };
        let packages = Packages { sides, summary: None, scroll: 0 };
        self.update_open(|open| Open { packages: Some(packages), ..open });
        vec![action]
    }

    /// A key that needs a real line does nothing here, as in the prose view; `v` is the way to the lines.
    pub(super) fn handle_packages_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let Some(packages) = self.open.as_ref().and_then(|o| o.packages.clone()) else { return vec![] };
        if let Some(scroll) = super::keys::scroll_key(packages.scroll, usize::MAX, key) {
            self.act_here(Pane::Page, key);
            self.update_open(|open| Open { packages: Some(Packages { scroll, ..packages }), ..open });
            return vec![];
        }
        match self.act_here(Pane::Prose, key) {
            Some(Act::Refresh) => return self.read_packages(packages.sides),
            Some(Act::Prose | Act::ClosePane | Act::Back) => {
                self.key_did = Some(Act::Prose);
                self.update_open(|open| Open { packages: None, ..open });
            }
            _ => self.did_nothing(),
        }
        vec![]
    }

    /// The summary of `sides` at `head`, unless the reader moved on or the MR moved meanwhile.
    pub(super) fn apply_packages(&mut self, key: &MrKey, head: &Sha, sides: &Sides, summary: Summary) {
        let current = self.open.as_ref().filter(|o| &o.key == key && &o.review.mr.refs.head == head);
        let Some(packages) = current.and_then(|o| o.packages.clone()).filter(|p| &p.sides == sides) else { return };
        let packages = Packages { summary: Some(Arc::new(summary)), ..packages };
        self.update_open(|open| Open { packages: Some(packages), ..open });
    }

    /// No summary is no error screen: the raw diff takes over, and the toast says why.
    pub(super) fn packages_failed(&mut self, message: &str) {
        if self.packages_open() {
            self.update_open(|open| Open { packages: None, ..open });
            self.warn(format!("raw diff shown: {message}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::test_support::*;

    const BASE: &str = include_str!("../../lockfile/fixtures/cargo_base.lock");
    const HEAD: &str = include_str!("../../lockfile/fixtures/cargo_head.lock");

    /// The cursor on the folded `Cargo.lock` of the fixture review.
    fn on_lockfile() -> App {
        let mut app = with_review();
        let at = app.open.as_ref().unwrap().rows.iter().position(|r| matches!(r, Row::File { index: 1, .. })).unwrap();
        app.update_open(|open| open.move_to(at));
        app
    }

    fn packages(app: &App) -> Option<&Packages> {
        app.open.as_ref().unwrap().packages.as_ref()
    }

    fn answer(app: &mut App) {
        let summary = crate::lockfile::summarize("Cargo.lock", Some(BASE), Some(HEAD)).unwrap();
        let sides = packages(app).unwrap().sides.clone();
        app.apply(Incoming::Packages { key: mr_key(), head: "bbbb".into(), sides, summary });
    }

    fn asks_packages(actions: &[Action]) -> bool {
        actions.iter().any(|a| matches!(a, Action::LoadPackages { .. }))
    }

    #[test]
    fn enter_on_a_folded_lockfile_opens_its_packages_and_v_goes_to_its_raw_diff() {
        let mut app = on_lockfile();
        assert!(asks_packages(&app.handle_key(code(KeyCode::Enter))));
        answer(&mut app);
        assert!(render(&mut app, 120, 24).contains("v: raw diff"));
        press(&mut app, "v");
        assert_eq!(packages(&app), None);
        assert!(app.open.as_ref().unwrap().rows.iter().any(|r| matches!(r, Row::Line { file: 1, .. })), "the raw lines are one key away");
        assert!(asks_packages(&press(&mut app, "v")), "v goes back");
    }

    #[test]
    fn za_on_a_folded_lockfile_only_unfolds_its_raw_diff() {
        let mut app = on_lockfile();
        assert!(!asks_packages(&press(&mut app, "za")));
        assert_eq!(packages(&app), None);
    }

    #[test]
    fn a_line_action_works_in_the_raw_diff_and_does_nothing_in_the_summary() {
        let mut app = on_lockfile();
        press(&mut app, "v");
        answer(&mut app);
        press(&mut app, "c");
        assert_eq!(app.input, None, "no line to comment on");
        press(&mut app, "v]cjc");
        assert!(app.input.is_some(), "the raw diff comments as any file");
    }

    #[test]
    fn a_thread_on_the_lockfile_is_counted_and_points_to_the_raw_diff() {
        let mut app = on_lockfile();
        app.update_open(|open| {
            open.relaid(|review| {
                let thread = |t: &crate::review::Thread| {
                    let anchor = t.anchor.clone().map(|a| crate::review::Anchor { path: "Cargo.lock".into(), ..a });
                    crate::review::Thread { anchor, outdated: false, ..t.clone() }
                };
                let threads: Vec<_> = review.threads.iter().map(thread).filter(|t| t.anchor.is_some()).take(2).collect();
                Review { threads: threads.into(), ..review }
            })
        });
        press(&mut app, "v");
        answer(&mut app);
        assert!(render(&mut app, 120, 24).contains("2 threads on this file · v: raw diff to read them"));
    }

    #[test]
    fn a_failed_read_goes_back_to_the_raw_diff_and_says_so() {
        let mut app = on_lockfile();
        press(&mut app, "v");
        app.apply(Incoming::Failed { what: Failure::Packages, message: "version 9".into() });
        assert_eq!(packages(&app), None);
        assert!(app.open.as_ref().unwrap().rows.iter().any(|r| matches!(r, Row::Line { file: 1, .. })));
        assert_eq!(app.live_toast().map(|t| t.text.as_str()), Some("raw diff shown: version 9"));
    }

    #[test]
    fn an_answer_for_an_older_head_is_dropped() {
        let mut app = on_lockfile();
        press(&mut app, "v");
        let sides = packages(&app).unwrap().sides.clone();
        app.apply(Incoming::Packages { key: mr_key(), head: "dddd".into(), sides, summary: Summary::default() });
        assert_eq!(packages(&app).unwrap().summary, None);
    }

    #[test]
    fn snapshot_packages_view() {
        let mut app = on_lockfile();
        press(&mut app, "v");
        answer(&mut app);
        insta::assert_snapshot!("packages", render(&mut app, 100, 22));
    }
}
