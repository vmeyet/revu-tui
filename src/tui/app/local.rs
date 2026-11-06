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
        vec![Action::ChangeLocal { key, change }]
    }
}

#[cfg(test)]
mod tests {
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

    fn first_rows(app: &mut App, rows: usize) -> String {
        render(app, 120, 20).lines().take(rows).collect::<Vec<_>>().join("\n")
    }
}
