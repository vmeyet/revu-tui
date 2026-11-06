//! The keys `revu local` has no use for.
use super::{App, Focus};
use crate::act::{self, Pane};
use crate::forge::Kind;
use crossterm::event::KeyEvent;

impl App {
    /// The forge this revu reads: every row of a queue is local, or none is.
    pub(crate) fn kind(&self) -> Kind {
        self.hosts.main.1
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
}

#[cfg(test)]
mod tests {
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
}
