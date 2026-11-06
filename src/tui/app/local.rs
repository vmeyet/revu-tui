//! What only `revu local` does: `B`, `I`, and the keys it has no use for.
use super::{Action, App, Focus, Open, QueueRow};
use crate::act::{self, Pane};
use crate::forge::Kind;
use crate::forge::local::Change;
use crate::tui::palette::{Mode, Palette};
use crossterm::event::KeyEvent;

impl App {
    /// The forge this revu reads: every row of a queue is local, or none is.
    pub(crate) fn kind(&self) -> Kind {
        self.hosts.main.1
    }

    pub(crate) fn is_local(&self) -> bool {
        self.kind() == Kind::Local
    }

    /// `key` stands for an action this diff has no use for, so it does nothing: the one place keys read an action's reach.
    pub(super) fn out_of_reach(&self, key: KeyEvent) -> bool {
        let pane = match self.focus {
            Focus::Queue => Pane::Queue,
            Focus::Review => Pane::Diff,
            Focus::Side => Pane::Threads,
        };
        let act = act::act_in(Pane::Anywhere, key).or_else(|| act::act_in(pane, key));
        act.is_some_and(|act| !act.row().reach.serves(self.kind()))
    }

    /// Out of zen; on a local diff, on to the worktree screen, the keys on its rows.
    pub(super) fn leave_zen_on_a_screen(&mut self) -> Vec<Action> {
        if self.is_local() {
            self.focus = Focus::Queue;
        }
        self.leave_zen()
    }

    /// What every key, click and paste leaves through: the terminal is in front, rows may have moved under the cursor,
    /// by a key, a sort or a filter, and the cursor's worktree shows beside them. With the keys elsewhere, the rows are
    /// left alone: another way of opening may have moved the diff away from the cursor.
    pub(super) fn after_input(&mut self, actions: Vec<Action>) -> Vec<Action> {
        self.terminal_focused = true;
        let sent = self.sending(actions);
        [sent, self.follow_the_row()].concat()
    }

    /// The cursor's worktree, while the keys are on the rows.
    pub(super) fn follow_the_row(&mut self) -> Vec<Action> {
        if self.focus != Focus::Queue || self.zen {
            return vec![];
        }
        self.show_selected_worktree()
    }

    /// The diff of the row under the cursor, asked for unless it is on its way or on screen.
    pub(super) fn show_selected_worktree(&mut self) -> Vec<Action> {
        if !self.is_local() {
            return vec![];
        }
        let Some(key) = self.selected_mr().map(crate::forge::QueueMr::key) else { return vec![] };
        if self.shown_worktree() == Some(&key) {
            return vec![];
        }
        self.opening = Some(key.clone());
        vec![Action::Open(key)]
    }

    /// A look failed: looks again soon, says why once and marks the header; the pane may be saying it already.
    pub(super) fn local_look_failed(&mut self, reason: String) {
        let said = self
            .open
            .as_ref()
            .is_some_and(|open| open.look_failed.as_ref() == Some(&reason) || open.review.mr.no_diff.as_ref() == Some(&reason));
        if !said {
            self.warn(reason.clone());
            self.update_open(|open| Open { look_failed: Some(reason), ..open });
        }
        self.back_off();
    }

    /// The worktree being opened, else the open one.
    fn shown_worktree(&self) -> Option<&super::MrKey> {
        self.opening.as_ref().or(self.open.as_ref().map(|open| &open.key))
    }

    /// The folded section whose header the cursor sits on, on a local screen, before new rows move it.
    pub(super) fn folded_header_here(&self) -> Option<&'static str> {
        match self.queue_rows().get(self.queue_selected).filter(|_| self.is_local()) {
            Some(QueueRow::Section { name, open: false, .. }) => Some(*name),
            _ => None,
        }
    }

    /// New rows may sort another way: the cursor stays on its folded header, else on the worktree that shows.
    pub(super) fn keep_cursor(&mut self, folded_header: Option<&str>) {
        let header = self.queue_rows().iter().position(|row| matches!(row, QueueRow::Section { name, .. } if Some(*name) == folded_header));
        if let Some(header) = header {
            self.queue_selected = header;
        } else if let Some(key) = self.shown_worktree().filter(|key| key.is_local()).cloned() {
            self.select_in_queue(&key);
        }
    }

    /// The worktree that shows is in no list any more, a folded section being still a list.
    pub(super) fn shown_worktree_left(&self) -> bool {
        let listed = |key: &&super::MrKey| self.sections.iter().flat_map(crate::forge::Sections::all).any(|mr| mr.key() == **key);
        self.shown_worktree().is_some_and(|key| !listed(&key))
    }

    /// A look found the head still there but the merge base gone elsewhere: the files on screen no longer match.
    pub(super) fn local_base_moved(&self, key: &super::MrKey, mr: &crate::forge::Mr) -> bool {
        key.is_local() && self.open.as_ref().is_some_and(|open| open.key == *key && open.review.mr.refs.base != mr.refs.base)
    }

    /// The terminal came back into focus: a local diff looks at once for what changed behind it, unless a look is on its way.
    pub(crate) fn focus_gained(&mut self) -> Vec<Action> {
        self.terminal_focus(true);
        let Some(open) = self.open.as_ref().filter(|open| open.key.is_local() && self.poll.mr_due.is_some()) else { return vec![] };
        let action = Action::PollMr { key: open.key.clone(), head: open.review.mr.refs.head.clone() };
        self.poll.mr_due = None;
        vec![action]
    }

    /// `B`: the palette, on `:target ` for the branch to come.
    pub(super) fn ask_target(&mut self) {
        let mut palette = Palette::new(Mode::Commands, self.palette_history.clone());
        "target ".chars().for_each(|c| palette.type_char(c));
        self.palette = Some(palette);
    }

    /// `B` and `I` answer for every worktree; the open one's diff comes back whole, the only way to see the change.
    pub(super) fn change_local(&mut self, change: Change) -> Vec<Action> {
        let Some(key) = self.open.as_ref().map(|open| open.key.clone()) else {
            self.toast("open a worktree first");
            return vec![];
        };
        if matches!(change, Change::Target(_)) {
            self.poll.queue_due = Some(self.now);
        }
        vec![Action::ChangeLocal { key, change }]
    }
}

#[cfg(test)]
mod tests {
    use crate::forge::QueueMr;
    use crate::forge::local::Change;
    use crate::tui::app::test_support::*;

    #[test]
    fn every_forge_key_does_nothing_in_the_diff() {
        for key in ["c", "C", "V", "s", "E", "R", "P", "A", "M", "H", "Y", "+", "o", "y", "p", "i"] {
            let mut app = local_app();
            let before = app.open.clone();
            assert_eq!(press(&mut app, key), vec![], "{key}");
            assert_eq!(app.open, before, "{key}");
            assert!(app.input.is_none() && app.brief.is_none() && app.publish.is_none(), "{key}");
            assert_eq!(app.live_toast(), None, "{key} says nothing");
        }
    }

    #[test]
    fn every_forge_key_does_nothing_in_the_queue() {
        for key in ["Y", "o", "y", "i", "*"] {
            let mut app = local_app();
            app.focus = Focus::Queue;
            assert_eq!(press(&mut app, key), vec![], "{key}");
            assert_eq!(app.brief, None, "{key}");
        }
    }

    #[test]
    fn a_forge_command_is_refused_and_not_offered() {
        let mut app = local_app();
        press(&mut app, ":");
        let offered = app.completions_for("");
        assert!(!offered.contains(&"approve".to_owned()) && offered.contains(&"go".to_owned()), "{offered:?}");
        press(&mut app, "approve");
        assert_eq!(app.handle_key(code(KeyCode::Enter)), vec![]);
        assert!(app.live_toast().unwrap().danger);
    }

    #[test]
    fn the_help_of_a_local_diff_leaves_out_what_needs_a_forge() {
        let mut app = local_app();
        press(&mut app, "??");
        let screen = render(&mut app, 160, 60);
        assert!(!screen.contains("publish every draft") && !screen.contains("pipeline"), "{screen}");
        let mut forge = with_review();
        press(&mut forge, "??");
        assert!(render(&mut forge, 160, 60).contains("publish every draft"));
    }

    #[test]
    fn the_tap_bar_of_a_local_diff_has_no_label_that_needs_a_forge() {
        let mut app = local_app();
        let labels: Vec<&str> = app.taps_here().iter().map(|tap| tap.label).collect();
        assert!(!labels.contains(&"comment") && labels.contains(&"file"), "{labels:?}");
        app.focus = Focus::Queue;
        assert!(!app.taps_here().iter().any(|tap| tap.label == "copy"));
    }

    #[test]
    fn the_help_lists_b_and_i_on_a_local_diff_only() {
        let mut app = local_app();
        press(&mut app, "??");
        assert!(render(&mut app, 160, 60).contains("`revu local`: show uncommitted work"));
        let mut forge = with_review();
        press(&mut forge, "??");
        assert!(!render(&mut forge, 160, 60).contains("`revu local`"));
    }

    #[test]
    fn b_and_i_do_nothing_on_a_forge() {
        let mut app = with_review();
        assert_eq!(press(&mut app, "BI"), vec![]);
        assert!(app.palette.is_none());
    }

    #[test]
    fn i_reads_the_diff_again_the_other_way() {
        let mut app = local_app();
        assert_eq!(press(&mut app, "I"), vec![Action::ChangeLocal { key: local_key(), change: Change::Uncommitted }]);
    }

    #[test]
    fn b_asks_for_a_branch_and_sends_it() {
        let mut app = local_app();
        press(&mut app, "B");
        press(&mut app, "dev");
        assert_eq!(
            app.handle_key(code(KeyCode::Enter)),
            vec![Action::ChangeLocal { key: local_key(), change: Change::Target("dev".into()) }]
        );
    }

    #[test]
    fn a_bad_target_toasts_and_keeps_the_diff() {
        let mut app = local_app();
        let before = app.open.clone();
        app.apply(Incoming::Failed { what: Failure::Local, message: "nope is not a commit here".into() });
        assert_eq!(app.open, before);
        assert!(app.live_toast().unwrap().danger);
    }

    #[test]
    fn a_new_head_says_no_new_commits_on_a_worktree() {
        let mut app = local_app();
        let moved = local_mr(0, "cccc");
        app.apply(Incoming::Review { key: local_key(), review: Box::new(Review::new(moved, &diffs(), vec![], &[])), cached: None });
        assert_eq!(app.news, None);
    }

    #[test]
    fn snapshot_the_zen_header_says_what_is_hidden() {
        let mut app = local_opened(local_mr(3, "bbbb"), &diffs());
        app.zen = true;
        insta::assert_snapshot!("local_zen_header", first_rows(&mut app, 3));
    }

    #[test]
    fn snapshot_the_pane_header_has_no_ref_approvals_or_pipeline() {
        let mut app = local_opened(local_mr(3, "bbbb"), &diffs());
        insta::assert_snapshot!("local_header", first_rows(&mut app, 4));
    }

    #[test]
    fn snapshot_no_commits_ahead_says_where_the_work_is() {
        let mut app = local_opened(at_the_base(3), &[]);
        app.zen = true;
        insta::assert_snapshot!("local_empty", render(&mut app, 100, 12));
    }

    #[test]
    fn opening_a_worktree_with_no_merge_base_says_why_there_is_no_diff() {
        let no_diff = Some("main shares no history with this branch".to_owned());
        let mut app = local_opened(Mr { no_diff, ..local_mr(0, "bbbb") }, &[]);
        let screen = render(&mut app, 120, 12);
        assert!(screen.contains("no diff: main shares no history with this branch"), "{screen}");
    }

    #[test]
    fn with_nothing_hidden_the_empty_diff_only_says_nothing_is_ahead() {
        let mut app = local_opened(at_the_base(0), &[]);
        let screen = render(&mut app, 120, 12);
        assert!(screen.contains("no commits ahead") && !screen.contains("uncommitted"), "{screen}");
    }

    /// The worktree screen as the backend sorts it, the cursor on `here`.
    fn on_the_screen() -> App {
        let mut app = local_app();
        app.focus = Focus::Queue;
        app.apply(Incoming::Queue { scope: None, me: "widgets".into(), sections: worktrees(), opened: HashMap::new(), cached: false });
        app.take_actions();
        app
    }

    fn worktrees() -> Sections {
        let row = |number: u64, branch: &str, minutes: i64, commits: u32, uncommitted: u32, (additions, deletions): (u32, u32)| QueueMr {
            title: branch.into(),
            source_branch: branch.into(),
            number,
            updated_at: today() - chrono::Duration::minutes(minutes),
            size: Some(crate::forge::Size { additions, deletions, files: 1 }),
            worktree: Some(crate::forge::WorktreeRow {
                commits,
                uncommitted,
                no_diff: None,
                dir: format!("/code/widgets/{number}").into(),
            }),
            ..QueueMr::from_mr(&local_mr(0, "bbbb"), local_key().host).unwrap()
        };
        Sections {
            mine: vec![row(7, "feat/local-diff", 2, 3, 2, (182, 31))],
            to_review: vec![row(9, "fix/header-width", 60, 1, 0, (12, 4)), row(8, "fix/old", 600, 2, 0, (40, 2))],
            open: vec![row(10, "main", 0, 0, 1, (0, 0))],
            ..Sections::default()
        }
    }

    #[test]
    fn snapshot_the_worktree_screen() {
        let mut app = on_the_screen();
        insta::assert_snapshot!("worktree_screen", render(&mut app, 110, 18));
    }

    #[test]
    fn esc_in_zen_lands_on_the_worktree_screen() {
        let mut app = on_the_screen();
        app.zen = true;
        app.focus = Focus::Review;
        app.handle_key(code(KeyCode::Esc));
        assert_eq!((app.zen, app.focus), (false, Focus::Queue));
    }

    #[test]
    fn moving_on_the_screen_asks_for_the_diff_of_the_row_and_keeps_the_keys_there() {
        let mut app = on_the_screen();
        let actions = press(&mut app, "j");
        let key = app.selected_mr().unwrap().key();
        assert_ne!(key, local_key());
        assert_eq!(actions, vec![Action::Open(key.clone())]);
        assert_eq!((app.focus, app.opening.as_ref()), (Focus::Queue, Some(&key)));
        assert_eq!(press(&mut app, "/"), vec![], "not asked twice for the same row");
    }

    #[test]
    fn a_diff_is_looked_at_every_2s_and_not_for_threads() {
        let mut app = local_app();
        assert_eq!(app.poll.discussions_due, None);
        assert_eq!(app.tick(), vec![]);
        app.now += Duration::from_secs(2);
        assert_eq!(app.tick(), vec![Action::PollMr { key: local_key(), head: "bbbb".into() }]);
        assert_eq!(app.tick(), vec![], "one look at a time");
    }

    #[test]
    fn terminal_focus_looks_at_once_unless_a_look_is_on_its_way() {
        let mut app = local_app();
        assert_eq!(app.focus_gained(), vec![Action::PollMr { key: local_key(), head: "bbbb".into() }]);
        assert_eq!(app.focus_gained(), vec![]);
        let mut forge = with_review();
        assert_eq!(forge.focus_gained(), vec![]);
    }

    #[test]
    fn a_look_that_finds_the_head_still_there_keeps_the_diff_and_the_cursor() {
        let mut app = local_app();
        press(&mut app, "]cj");
        let before = app.open.clone().unwrap();
        app.apply(Incoming::Mr { key: local_key(), mr: Box::new(local_mr(4, "bbbb")) });
        let open = app.open.as_ref().unwrap();
        assert!(Arc::ptr_eq(&open.review.files, &before.review.files), "nothing built again");
        assert_eq!((open.selected, open.review.mr.uncommitted), (before.selected, 4));
    }

    #[test]
    fn rows_that_sort_another_way_leave_the_cursor_on_the_shown_worktree() {
        let mut app = on_the_screen();
        let before = worktrees();
        let moved = Sections { mine: before.to_review.clone(), to_review: before.mine, ..before };
        app.apply(Incoming::Queue { scope: None, me: "widgets".into(), sections: moved, opened: HashMap::new(), cached: false });
        assert_eq!(app.selected_mr().map(QueueMr::key), Some(local_key()));
    }

    #[test]
    fn a_click_on_a_row_shows_its_worktree_too() {
        let mut app = on_the_screen();
        render(&mut app, 110, 18);
        let at = app.drawn.list_rows.iter().find(|row| row.index != app.queue_selected).copied().unwrap().area;
        let click = |kind| crossterm::event::MouseEvent { kind, column: at.x + 1, row: at.y, modifiers: KeyModifiers::NONE };
        let left = crossterm::event::MouseButton::Left;
        app.handle_mouse(click(crossterm::event::MouseEventKind::Down(left)));
        let actions = app.handle_mouse(click(crossterm::event::MouseEventKind::Up(left)));
        assert_eq!(actions, vec![Action::Open(app.selected_mr().unwrap().key())]);
    }

    #[test]
    fn a_worktree_with_no_merge_base_keeps_its_row_with_the_reason() {
        let mut app = on_the_screen();
        let mut sections = worktrees();
        let island =
            sections.open[0].worktree.take().map(|row| crate::forge::WorktreeRow { no_diff: Some("no shared history".into()), ..row });
        sections.open[0].worktree = island;
        app.apply(Incoming::Queue { scope: None, me: "widgets".into(), sections, opened: HashMap::new(), cached: false });
        let screen = render(&mut app, 110, 18);
        assert!(screen.contains("no shared history") && screen.contains("feat/local-diff"), "{screen}");
    }

    #[test]
    fn a_new_target_also_refreshes_the_rows_soon() {
        let mut app = on_the_screen();
        app.queue_loading = false;
        press(&mut app, "B");
        press(&mut app, "dev");
        app.handle_key(code(KeyCode::Enter));
        assert!(app.tick().contains(&Action::LoadQueue { scope: None, from_cache: false }));
    }

    fn one_line_file(path: &str, line: &str) -> DiffFile {
        DiffFile { diff: format!("@@ -1 +1 @@\n-old\n+{line}\n"), old_path: path.into(), new_path: path.into(), ..DiffFile::default() }
    }

    /// `files` arriving at a new head, as a look at the worktree brings them.
    fn head_moved(app: &mut App, head: &str, files: &[DiffFile]) {
        app.apply(Incoming::Review {
            key: local_key(),
            review: Box::new(Review::new(local_mr(0, head), files, vec![], &[])),
            cached: None,
        });
    }

    #[test]
    fn a_viewed_file_stays_viewed_across_a_head_move_until_its_diff_changes() {
        let files = [one_line_file("a.rs", "a"), one_line_file("b.rs", "b")];
        let mut app = local_opened(local_mr(0, "aaaa"), &files);
        app.review_jump_to(|row| matches!(row, Row::File { index: 1, .. }));
        press(&mut app, "zv");
        let viewed = |app: &App| app.open.as_ref().unwrap().review.viewed.contains("b.rs");
        head_moved(&mut app, "bbbb", &files);
        assert!(viewed(&app));
        head_moved(&mut app, "cccc", &[files[0].clone(), one_line_file("b.rs", "edited")]);
        assert!(!viewed(&app));
    }

    #[test]
    fn a_head_move_over_the_same_files_keeps_the_cursor() {
        let files = [one_line_file("a.rs", "a"), one_line_file("b.rs", "b")];
        let mut app = local_opened(local_mr(0, "aaaa"), &files);
        app.review_jump_to(|row| matches!(row, Row::Line { file: 1, .. }));
        let before = app.open.as_ref().unwrap().row().cloned();
        head_moved(&mut app, "bbbb", &files);
        assert_eq!(app.open.as_ref().unwrap().row().cloned(), before);
    }

    fn row_key(number: u64) -> MrKey {
        MrKey { number, ..local_key() }
    }

    #[test]
    fn a_review_of_the_open_worktree_leaves_the_one_being_opened_alone() {
        let mut app = on_the_screen();
        press(&mut app, "j");
        let moved = app.opening.clone().unwrap();
        app.apply(Incoming::Review { key: local_key(), review: Box::new(review()), cached: None });
        assert_eq!(app.opening, Some(moved));
    }

    #[test]
    fn a_moved_merge_base_rebuilds_the_diff_and_keeps_the_old_refs_until_then() {
        let mut app = local_app();
        let before = app.open.as_ref().unwrap().review.mr.refs.clone();
        let moved = crate::forge::Refs { base: "cccc".into(), start: "cccc".into(), ..before.clone() };
        app.apply(Incoming::Mr { key: local_key(), mr: Box::new(Mr { refs: moved, ..local_mr(0, "bbbb") }) });
        assert_eq!(app.take_actions(), vec![Action::RefreshMr(local_key())]);
        assert_eq!(app.open.as_ref().unwrap().review.mr.refs, before);
    }

    #[test]
    fn a_forge_mr_whose_base_moved_only_takes_the_new_refs() {
        let mut app = with_review();
        let moved = crate::forge::Refs { base: "cccc".into(), ..mr().refs };
        app.apply(Incoming::Mr { key: mr_key(), mr: Box::new(Mr { refs: moved.clone(), ..mr() }) });
        assert_eq!(app.take_actions(), vec![]);
        assert_eq!(app.open.as_ref().unwrap().review.mr.refs, moved);
    }

    #[test]
    fn a_failed_look_says_its_reason_and_never_offline() {
        let mut app = local_app();
        app.apply(Incoming::Failed { what: Failure::Poll { key: local_key() }, message: "git broke".into() });
        assert_eq!((app.offline, app.live_toast().map(|toast| toast.text.as_str())), (None, Some("git broke")));
    }

    #[test]
    fn a_failed_open_says_its_reason_and_never_offline() {
        let mut app = on_the_screen();
        press(&mut app, "j");
        let key = app.opening.clone().unwrap();
        app.apply(Incoming::Failed { what: Failure::Open { key }, message: "git broke".into() });
        assert_eq!((app.offline, app.opening.clone(), app.live_toast().map(|toast| toast.text.as_str())), (None, None, Some("git broke")));
    }

    fn look_failed(app: &mut App, message: &str) {
        app.apply(Incoming::Failed { what: Failure::Poll { key: local_key() }, message: message.into() });
    }

    #[test]
    fn a_failure_looks_again_soon_and_leaves_the_queue_rescan_at_its_pace() {
        let mut app = on_the_screen();
        app.queue_loading = false;
        let due = app.poll.queue_due;
        for _ in 0..40 {
            app.now += Duration::from_secs(1);
            look_failed(&mut app, "git broke");
            assert_eq!(app.poll.mr_due, Some(app.now + Duration::from_secs(2)));
            assert_eq!(app.poll.queue_due, due, "many failures do not put it off");
        }
    }

    #[test]
    fn failed_looks_do_not_cancel_the_rescan_a_new_target_asked_for() {
        let mut app = on_the_screen();
        app.queue_loading = false;
        press(&mut app, "B");
        press(&mut app, "dev");
        app.handle_key(code(KeyCode::Enter));
        look_failed(&mut app, "git broke");
        assert!(app.tick().contains(&Action::LoadQueue { scope: None, from_cache: false }));
    }

    #[test]
    fn a_failing_look_says_why_once_and_marks_the_header_until_a_look_works() {
        let mut app = local_app();
        look_failed(&mut app, "git broke");
        assert!(app.live_toast().is_some());
        app.toast = None;
        look_failed(&mut app, "git broke");
        assert_eq!(app.live_toast(), None, "the same reason stays quiet");
        assert!(first_rows(&mut app, 4).contains("git broke"), "the mark outlives the toast");
        app.apply(Incoming::Mr { key: local_key(), mr: Box::new(local_mr(0, "bbbb")) });
        assert!(!first_rows(&mut app, 4).contains("git broke"));
    }

    #[test]
    fn zen_marks_a_failing_look_too() {
        let mut app = local_app();
        app.zen = true;
        look_failed(&mut app, "git broke");
        assert!(first_rows(&mut app, 3).contains("git broke"));
    }

    #[test]
    fn an_r_the_reader_asked_for_always_answers() {
        let mut app = local_app();
        look_failed(&mut app, "git broke");
        app.toast = None;
        assert_eq!(press(&mut app, "r"), vec![Action::RefreshMr(local_key())]);
        look_failed(&mut app, "git broke");
        assert!(app.live_toast().is_some());
    }

    #[test]
    fn an_r_on_the_screen_answers_too() {
        let mut app = on_the_screen();
        app.queue_loading = false;
        look_failed(&mut app, "git broke");
        app.toast = None;
        press(&mut app, "r");
        look_failed(&mut app, "git broke");
        assert!(app.live_toast().is_some());
    }

    #[test]
    fn a_look_failing_with_the_reason_the_pane_already_says_stays_quiet() {
        let no_diff = Some("HEAD~1 is not a commit here".to_owned());
        let mut app = local_opened(Mr { no_diff, ..local_mr(0, "bbbb") }, &[]);
        look_failed(&mut app, "HEAD~1 is not a commit here");
        assert_eq!(app.live_toast(), None);
        assert_eq!(first_rows(&mut app, 4).matches("HEAD~1").count(), 0, "the pane says it, not the header");
    }

    #[test]
    fn a_rescan_that_adds_a_row_leaves_the_cursor_on_its_folded_section_and_opens_nothing() {
        let mut app = on_the_screen();
        press(&mut app, "G");
        let key = app.opening.clone().unwrap();
        app.apply(Incoming::Review { key, review: Box::new(review()), cached: None });
        press(&mut app, "zc");
        let at_header =
            |app: &App| matches!(app.queue_rows().get(app.queue_selected), Some(QueueRow::Section { name: "NOTHING AHEAD", .. }));
        assert!(at_header(&app));
        let mut more = worktrees();
        more.to_review.push(QueueMr { number: 11, ..more.to_review[0].clone() });
        app.apply(Incoming::Queue { scope: None, me: "widgets".into(), sections: more, opened: HashMap::new(), cached: false });
        assert!(!app.take_actions().iter().any(|action| matches!(action, Action::Open(_))));
        assert!(at_header(&app));
    }

    #[test]
    fn a_shown_worktree_gone_from_the_rows_gives_way_to_the_row_under_the_cursor() {
        let mut app = on_the_screen();
        let without = Sections { mine: vec![], ..worktrees() };
        app.apply(Incoming::Queue { scope: None, me: "widgets".into(), sections: without, opened: HashMap::new(), cached: false });
        assert_eq!(app.take_actions(), vec![Action::Open(app.selected_mr().unwrap().key())]);
    }

    #[test]
    fn a_failed_look_keeps_the_diff_and_the_viewed_marks() {
        let mut app = local_app();
        press(&mut app, "zv");
        let before = app.open.as_ref().unwrap().review.clone();
        assert!(!before.viewed.is_empty());
        look_failed(&mut app, "dev is not a commit here");
        assert_eq!(app.open.as_ref().unwrap().review, before);
    }

    #[test]
    fn the_wheel_over_the_rows_shows_the_row_with_the_diff_focused() {
        let mut app = on_the_screen();
        app.focus = Focus::Review;
        render(&mut app, 220, 30);
        let at = app.drawn.list_rows.iter().find(|row| row.index == app.queue_selected).copied().unwrap().area;
        let kind = crossterm::event::MouseEventKind::ScrollDown;
        let actions = app.handle_mouse(crossterm::event::MouseEvent { kind, column: at.x + 1, row: at.y, modifiers: KeyModifiers::NONE });
        let row = app.selected_mr().unwrap().key();
        assert_ne!(row, local_key());
        assert_eq!(actions, vec![Action::Open(row)]);
    }

    #[test]
    fn a_key_with_the_diff_focused_does_not_open_the_row_under_the_cursor() {
        let mut app = on_the_screen();
        press(&mut app, "j");
        app.focus = Focus::Review;
        app.opening = None;
        assert_eq!(press(&mut app, "w"), vec![]);
    }

    #[test]
    fn a_key_says_the_terminal_is_in_front_after_a_lost_focus() {
        let mut app = local_app();
        app.terminal_focus(false);
        press(&mut app, "w");
        app.now += Duration::from_secs(10);
        assert_eq!(app.tick(), vec![Action::PollMr { key: local_key(), head: "bbbb".into() }]);
    }

    #[test]
    fn coming_back_from_a_program_says_the_terminal_is_in_front() {
        let mut app = local_app();
        app.terminal_focus(false);
        let view = crate::open::View { argv: vec!["hx".into()], shown: "a.rs:1".into(), note: None, _copy: None };
        app.apply(Incoming::Viewed { view, outcome: Ok(()) });
        app.now += Duration::from_secs(10);
        assert_eq!(app.tick(), vec![Action::PollMr { key: local_key(), head: "bbbb".into() }]);
    }

    #[test]
    fn an_empty_commit_ahead_says_no_changes_and_not_no_commits() {
        let mut app = local_opened(local_mr(0, "bbbb"), &[]);
        let screen = render(&mut app, 120, 12);
        assert!(screen.contains("no changes ahead") && !screen.contains("no commits ahead"), "{screen}");
    }

    /// A worktree at its merge base: no commit ahead of the target.
    fn at_the_base(hidden: u32) -> Mr {
        let at: Sha = "bbbb".into();
        Mr { refs: crate::forge::Refs { base: at.clone(), start: at.clone(), head: at }, ..local_mr(hidden, "bbbb") }
    }

    #[test]
    fn a_queue_load_keeps_the_cursor_on_the_worktree_being_opened() {
        let mut app = on_the_screen();
        press(&mut app, "j");
        let moved = app.opening.clone().unwrap();
        app.apply(Incoming::Queue { scope: None, me: "widgets".into(), sections: worktrees(), opened: HashMap::new(), cached: false });
        assert_eq!(app.selected_mr().map(QueueMr::key), Some(moved));
    }

    #[test]
    fn typing_a_filter_shows_the_worktree_it_leaves_the_cursor_on() {
        let mut app = on_the_screen();
        let actions = press(&mut app, "/main");
        assert_eq!(actions, vec![Action::Open(row_key(10))]);
    }

    #[test]
    fn pasting_a_filter_shows_the_worktree_it_leaves_the_cursor_on() {
        let mut app = on_the_screen();
        press(&mut app, "/");
        assert_eq!(app.handle_paste("main"), vec![Action::Open(row_key(10))]);
    }

    #[test]
    fn sorting_the_screen_keeps_the_diff_on_the_row_under_the_cursor() {
        let mut app = on_the_screen();
        press(&mut app, "j");
        let key = app.opening.clone().unwrap();
        app.apply(Incoming::Review { key, review: Box::new(review()), cached: None });
        for _ in 0..4 {
            press(&mut app, "s");
            assert_eq!(app.selected_mr().map(QueueMr::key).as_ref(), app.shown_worktree());
        }
    }

    #[test]
    fn the_first_question_mark_lists_b_and_i_on_the_screen_and_in_the_diff() {
        let mut screen = on_the_screen();
        press(&mut screen, "?");
        assert!(render(&mut screen, 160, 60).contains("`revu local`: show uncommitted work"));
        let mut diff = local_app();
        press(&mut diff, "?");
        assert!(render(&mut diff, 160, 60).contains("`revu local`: show uncommitted work"));
    }

    #[test]
    fn the_diff_is_not_looked_at_while_the_terminal_is_away_and_focus_looks_at_once() {
        let mut app = local_app();
        app.terminal_focus(false);
        app.now += Duration::from_secs(5);
        assert_eq!(app.tick(), vec![]);
        assert_eq!(app.focus_gained(), vec![Action::PollMr { key: local_key(), head: "bbbb".into() }]);
    }

    fn asked_branch(actions: &[Action]) -> crate::agent::Branch {
        let [Action::Session { subject, .. }] = actions else { panic!("no session asked: {actions:?}") };
        let crate::agent::Subject::Branch(branch) = subject.as_ref() else { panic!("not a branch: {subject:?}") };
        branch.clone()
    }

    #[test]
    fn a_asks_for_the_session_of_the_branch_under_the_cursor_in_its_worktree_and_of_the_open_one_in_the_diff() {
        let mut app = on_the_screen();
        app.agent = true;
        press(&mut app, "j");
        let row = app.selected_mr().unwrap().clone();
        let branch = asked_branch(&press(&mut app, "a"));
        let crate::agent::Branch { key, name, dir, target } = branch;
        assert_eq!((key, name, target), (row.key(), row.source_branch, row.target_branch));
        assert_eq!(dir, row.worktree.unwrap().dir);
        assert_eq!(app.live_toast().unwrap().text, "looking for the session of fix/header-width…");
        let mut app = on_the_screen();
        app.agent = true;
        app.focus = Focus::Review;
        assert_eq!(asked_branch(&press(&mut app, "a")).name, "feat/local-diff");
    }

    fn first_rows(app: &mut App, rows: usize) -> String {
        render(app, 120, 20).lines().take(rows).collect::<Vec<_>>().join("\n")
    }
}
