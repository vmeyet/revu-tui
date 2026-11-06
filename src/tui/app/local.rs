//! What only `revu local` does: `B`, `I`, and the keys it has no use for.
use super::{Action, App, Focus};
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

    /// The cursor's worktree shows beside the rows: its diff is asked for as the cursor lands on it.
    pub(super) fn show_selected_worktree(&mut self) -> Vec<Action> {
        if !self.is_local() {
            return vec![];
        }
        let Some(key) = self.selected_mr().map(crate::forge::QueueMr::key) else { return vec![] };
        let shown = self.opening.as_ref().or(self.open.as_ref().map(|open| &open.key));
        if shown == Some(&key) {
            return vec![];
        }
        self.opening = Some(key.clone());
        vec![Action::Open(key)]
    }

    /// New rows may sort another way: the cursor stays on the worktree whose diff shows.
    pub(super) fn keep_cursor_on_the_shown_worktree(&mut self) {
        if let Some(key) = self.open.as_ref().filter(|open| open.key.is_local()).map(|open| open.key.clone()) {
            self.select_in_queue(&key);
        }
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
        for key in ["c", "C", "V", "s", "E", "R", "P", "A", "M", "H", "Y", "a", "+", "o", "y", "p", "i"] {
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
        for key in ["Y", "a", "o", "y", "i", "*"] {
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
        let mut app = local_opened(local_mr(3, "bbbb"), &[]);
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
        let mut app = local_opened(local_mr(0, "bbbb"), &[]);
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
            worktree: Some(crate::forge::WorktreeRow { commits, uncommitted, no_diff: None }),
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

    fn first_rows(app: &mut App, rows: usize) -> String {
        render(app, 120, 20).lines().take(rows).collect::<Vec<_>>().join("\n")
    }
}
