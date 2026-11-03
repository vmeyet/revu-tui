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
        let (sides, path) = (Sides::from_file(file), file.new_path.clone());
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
            Some(Act::ViewedNext) => {
                self.update_open(|open| Open { packages: None, ..open });
                return self.view_and_move_on();
            }
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
        let packages = Packages { summary: Some(Arc::new(summary)), scroll: 0, ..packages };
        self.update_open(|open| Open { packages: Some(packages), ..open });
    }

    /// No summary is no error screen: the raw diff takes over, and the toast says why. Only the read the view
    /// still waits for counts; an older one's failure leaves what is on screen.
    pub(super) fn packages_failed(&mut self, key: &MrKey, head: &Sha, sides: &Sides, message: &str) {
        let current = self.open.as_ref().filter(|o| &o.key == key && &o.review.mr.refs.head == head);
        if current.and_then(|o| o.packages.as_ref()).is_some_and(|p| &p.sides == sides && p.summary.is_none()) {
            self.update_open(|open| Open { packages: None, ..open });
            self.warn(format!("raw diff shown: {message}"));
        }
    }

    /// The view shows the file under the cursor: a pane that moves the cursor to another file ends it.
    pub(super) fn close_packages_off_their_file(&mut self) {
        let Some(open) = &self.open else { return };
        let Some(packages) = &open.packages else { return };
        let here = open.row().and_then(Row::file).map(|i| Sides::from_file(&open.review.files[i]));
        if here.as_ref() != Some(&packages.sides) {
            self.update_open(|open| Open { packages: None, ..open });
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
        let sides = packages(&app).unwrap().sides.clone();
        app.apply(Incoming::Failed { what: Failure::Packages { key: mr_key(), head: "bbbb".into(), sides }, message: "version 9".into() });
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

    fn package(name: &str, version: &str, checksum: &str) -> String {
        format!(
            "[[package]]\nname = \"{name}\"\nversion = \"{version}\"\nsource = \"registry+https://index.example\"\nchecksum = \"{checksum}\"\n"
        )
    }

    fn lock(entries: &[String]) -> String {
        format!("version = 4\n{}", entries.concat())
    }

    fn answer_with(app: &mut App, base: &str, head: &str) {
        let summary = crate::lockfile::summarize("Cargo.lock", Some(base), Some(head)).unwrap();
        let sides = packages(app).unwrap().sides.clone();
        app.apply(Incoming::Packages { key: mr_key(), head: "bbbb".into(), sides, summary });
    }

    /// The view open on one warning and `updates` routine rows under it.
    fn view_of_many(updates: usize) -> App {
        let mut app = on_lockfile();
        press(&mut app, "v");
        let row = |version: &str, i: usize| package(&format!("p{i:02}"), version, "a");
        let base = [vec![package("a", "1.0.0", "x")], (0..updates).map(|i| row("1.0.0", i)).collect()].concat();
        let head = [vec![package("a", "1.0.0", "y")], (0..updates).map(|i| row("1.0.1", i)).collect()].concat();
        answer_with(&mut app, &lock(&base), &lock(&head));
        app
    }

    fn failure(key: MrKey, head: &str, sides: Sides) -> Incoming {
        Incoming::Failed { what: Failure::Packages { key, head: head.into(), sides }, message: "HTTP 500".into() }
    }

    fn other_file() -> Sides {
        Sides { base: None, head: Some("web/package-lock.json".into()) }
    }

    fn other_mr() -> MrKey {
        MrKey { number: 7, ..mr_key() }
    }

    #[test]
    fn an_answer_for_another_mr_is_dropped() {
        let mut app = on_lockfile();
        press(&mut app, "v");
        let sides = packages(&app).unwrap().sides.clone();
        app.apply(Incoming::Packages { key: other_mr(), head: "bbbb".into(), sides, summary: Summary::default() });
        assert_eq!(packages(&app).unwrap().summary, None);
    }

    #[test]
    fn an_answer_for_another_file_is_dropped() {
        let mut app = on_lockfile();
        press(&mut app, "v");
        app.apply(Incoming::Packages { key: mr_key(), head: "bbbb".into(), sides: other_file(), summary: Summary::default() });
        assert_eq!(packages(&app).unwrap().summary, None);
    }

    #[test]
    fn a_failure_for_another_mr_leaves_the_view_waiting() {
        let mut app = on_lockfile();
        press(&mut app, "v");
        let sides = packages(&app).unwrap().sides.clone();
        app.apply(failure(other_mr(), "bbbb", sides));
        assert!(packages(&app).is_some());
    }

    #[test]
    fn a_failure_for_another_file_leaves_the_view_waiting() {
        let mut app = on_lockfile();
        press(&mut app, "v");
        app.apply(failure(mr_key(), "bbbb", other_file()));
        assert!(packages(&app).is_some());
    }

    #[test]
    fn a_failure_for_an_older_head_leaves_the_view_waiting() {
        let mut app = on_lockfile();
        press(&mut app, "v");
        let sides = packages(&app).unwrap().sides.clone();
        app.apply(failure(mr_key(), "dddd", sides));
        assert!(packages(&app).is_some());
    }

    #[test]
    fn a_failure_from_an_older_read_leaves_a_good_summary_on_screen() {
        let mut app = on_lockfile();
        press(&mut app, "v");
        answer(&mut app);
        let sides = packages(&app).unwrap().sides.clone();
        app.apply(failure(mr_key(), "bbbb", sides));
        assert!(packages(&app).is_some_and(|p| p.summary.is_some()));
    }

    #[test]
    fn an_answer_opens_at_the_top_so_warnings_show() {
        let mut app = on_lockfile();
        press(&mut app, "vG");
        answer(&mut app);
        assert_eq!(packages(&app).unwrap().scroll, 0);
    }

    #[test]
    fn g_scrolls_to_the_last_screen_and_stays_inside_the_rows() {
        let mut app = view_of_many(60);
        press(&mut app, "G");
        let screen = render(&mut app, 100, 20);
        assert!(screen.contains("p59") && !screen.contains("same version"), "{screen}");
        assert!(packages(&app).unwrap().scroll < 61);
    }

    #[test]
    fn j_scrolls_one_row_and_k_back() {
        let mut app = view_of_many(60);
        press(&mut app, "j");
        assert_eq!(packages(&app).unwrap().scroll, 1);
        press(&mut app, "k");
        assert_eq!(packages(&app).unwrap().scroll, 0);
    }

    #[test]
    fn r_reads_again_and_waits() {
        let mut app = on_lockfile();
        press(&mut app, "v");
        answer(&mut app);
        assert!(asks_packages(&press(&mut app, "r")));
        assert_eq!(packages(&app).unwrap().summary, None);
    }

    #[test]
    fn zz_goes_zen_and_keeps_the_view() {
        let mut app = on_lockfile();
        press(&mut app, "v");
        press(&mut app, "zz");
        assert!(app.zen && packages(&app).is_some());
    }

    #[test]
    fn a_new_draft_on_the_lockfile_is_counted() {
        let mut app = on_lockfile();
        press(&mut app, "v");
        answer(&mut app);
        let anchor = Some(crate::review::Anchor { path: "Cargo.lock".into(), side: crate::review::Side::New, line: 1 });
        let template = with_saved_draft().open.as_ref().unwrap().review.drafts[0].clone();
        app.update_open(|open| open.with_drafts_changed(|_| vec![Draft { anchor, reply_to: None, ..template }]));
        assert!(render(&mut app, 120, 24).contains("1 thread on this file"));
    }

    #[test]
    fn an_outdated_thread_on_the_lockfile_is_counted() {
        let mut app = on_lockfile();
        app.update_open(|open| {
            open.relaid(|review| {
                let outdated = |t: &crate::review::Thread| crate::review::Thread {
                    anchor: t.anchor.clone().map(|a| crate::review::Anchor { path: "Cargo.lock".into(), ..a }),
                    outdated: true,
                    ..t.clone()
                };
                let threads: Vec<_> = review.threads.iter().map(outdated).filter(|t| t.anchor.is_some()).take(1).collect();
                Review { threads: threads.into(), ..review }
            })
        });
        press(&mut app, "v");
        answer(&mut app);
        assert!(render(&mut app, 120, 24).contains("1 thread on this file"));
    }

    fn drawn_with(base: &[String], head: &[String]) -> String {
        let mut app = on_lockfile();
        press(&mut app, "v");
        answer_with(&mut app, &lock(base), &lock(head));
        render(&mut app, 100, 20)
    }

    #[test]
    fn a_downgrade_draws_a_down_arrow() {
        assert!(drawn_with(&[package("a", "2.0.0", "x")], &[package("a", "1.0.0", "x")]).contains("↓ a"));
    }

    #[test]
    fn an_upgrade_draws_an_up_arrow() {
        assert!(drawn_with(&[package("a", "1.0.0", "x")], &[package("a", "1.0.1", "x")]).contains("↑ a"));
    }

    #[test]
    fn a_release_turned_pre_release_draws_a_down_arrow() {
        assert!(drawn_with(&[package("a", "2.0.0", "x")], &[package("a", "2.0.0-rc.1", "x")]).contains("↓ a"));
    }

    #[test]
    fn a_name_of_300_columns_leaves_the_reason_on_screen() {
        let long = "a".repeat(300);
        let screen = drawn_with(&[package(&long, "1.0.0", "x")], &[package(&long, "1.0.0", "y")]);
        assert!(screen.contains("same version, checksum changed"), "{screen}");
    }

    #[test]
    fn a_name_of_65536_columns_draws_with_its_reason_on_screen() {
        let long = "a".repeat(65_536);
        let screen = drawn_with(&[package(&long, "1.0.0", "x")], &[package(&long, "1.0.0", "y")]);
        assert!(screen.contains("same version, checksum changed"), "{screen}");
    }

    #[test]
    fn a_long_version_leaves_the_reason_on_screen() {
        let long = "9".repeat(300);
        let screen = drawn_with(&[package("a", &long, "x")], &[package("a", &long, "y")]);
        assert!(screen.contains("same version, checksum changed"), "{screen}");
    }

    #[test]
    fn a_long_path_keeps_the_warning_count_in_the_title() {
        let mut app = view_of_many(1);
        let long = format!("{}/Cargo.lock", "services/payments/internal/third_party/vendored".repeat(3));
        app.update_open(|open| {
            let packages = open.packages.clone().map(|p| Packages { sides: Sides { base: None, head: Some(long) }, ..p });
            Open { packages, ..open }
        });
        assert!(render(&mut app, 80, 20).contains("1 to check"));
    }

    #[test]
    fn no_row_over_a_changed_file_says_how_many_raw_lines_changed() {
        let mut app = on_lockfile();
        press(&mut app, "v");
        let same = lock(&[package("a", "1.0.0", "x")]);
        answer_with(&mut app, &same, &same);
        let screen = render(&mut app, 100, 20);
        assert!(
            screen.contains("no package rows · 2 raw lines changed · v reads them") && !screen.contains("no package changed"),
            "{screen}"
        );
    }

    #[test]
    fn m_marks_the_lockfile_viewed_and_leaves_the_view() {
        let mut app = on_lockfile();
        press(&mut app, "v");
        answer(&mut app);
        press(&mut app, "m");
        let open = app.open.as_ref().unwrap();
        assert!(open.packages.is_none() && open.review.viewed.contains("Cargo.lock"));
    }

    #[test]
    fn enter_in_the_tree_on_another_file_leaves_the_view() {
        let mut app = on_lockfile();
        press(&mut app, "t");
        app.focus = Focus::Review;
        press(&mut app, "v");
        answer(&mut app);
        app.focus = Focus::Side;
        render(&mut app, 160, 30);
        press(&mut app, "gjj");
        app.handle_key(code(KeyCode::Enter));
        let open = app.open.as_ref().unwrap();
        assert_ne!(open.row().and_then(Row::file), Some(1), "the tree moved the cursor off the lockfile");
        assert!(open.packages.is_none());
    }

    #[test]
    fn a_jump_to_a_thread_on_another_file_leaves_the_view() {
        let mut app = on_lockfile();
        press(&mut app, "T");
        app.focus = Focus::Review;
        press(&mut app, "v");
        answer(&mut app);
        app.focus = Focus::Side;
        render(&mut app, 160, 30);
        app.handle_key(code(KeyCode::Enter));
        let open = app.open.as_ref().unwrap();
        assert_ne!(open.row().and_then(Row::file), Some(1), "the thread lives on another file");
        assert!(open.packages.is_none());
    }

    #[test]
    fn a_mouse_event_after_the_cursor_left_the_file_leaves_the_view() {
        let mut app = on_lockfile();
        press(&mut app, "v");
        app.update_open(|open| open.move_to(0));
        let moved = crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Moved,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        app.handle_mouse(moved);
        assert_eq!(packages(&app), None);
    }

    #[test]
    fn snapshot_packages_view() {
        let mut app = on_lockfile();
        press(&mut app, "v");
        answer(&mut app);
        insta::assert_snapshot!("packages", render(&mut app, 100, 22));
    }
}
