//! `:local` and `:forge`: this session ends and the loop opens the other side in the same screen.
use super::App;

/// Where a session reads from: the worktrees of the checkout, or the forge's queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    Local,
    Forge,
}

impl App {
    /// The loop takes the switch once this key is handled; a draft the forge does not hold yet keeps the reader here.
    pub(super) fn switch_side(&mut self, side: Side) {
        let unsaved = self.unsaved_drafts();
        if unsaved > 0 {
            self.warn(format!("{unsaved} draft{} not saved yet: switch once the forge holds them", if unsaved == 1 { "" } else { "s" }));
            return;
        }
        self.switch = Some(side);
    }

    /// Taken by the loop, which ends this session and opens the side asked for.
    pub(crate) fn take_switch(&mut self) -> Option<Side> {
        self.switch.take()
    }

    /// The first words of a session the reader switched to.
    pub(crate) fn switched(&mut self) {
        self.toast(if self.is_local() {
            "local: the worktrees of this checkout · :forge goes back"
        } else {
            "forge: your queue · :local reads the worktrees of this checkout"
        });
    }

    /// The other side did not open: this session goes on as it was.
    pub(crate) fn switch_failed(&mut self, side: Side, reason: &str) {
        let name = match side {
            Side::Local => "local",
            Side::Forge => "the forge",
        };
        self.warn(format!("cannot switch to {name}: {reason}"));
    }
}

#[cfg(test)]
mod tests {
    use super::Side;
    use crate::tui::app::test_support::*;

    #[test]
    fn local_on_a_forge_asks_the_loop_for_the_worktrees() {
        let mut app = with_queue();
        type_text(&mut app, ":local");
        assert_eq!(app.take_switch(), Some(Side::Local));
        assert_eq!(app.take_switch(), None, "taken once");
    }

    #[test]
    fn forge_on_a_local_diff_asks_the_loop_for_the_forge() {
        let mut app = local_app();
        type_text(&mut app, ":forge");
        assert_eq!(app.take_switch(), Some(Side::Forge));
    }

    #[test]
    fn the_side_already_on_screen_is_no_switch() {
        let mut app = with_queue();
        type_text(&mut app, ":forge");
        assert_eq!(app.take_switch(), None);
        assert_eq!(toast(&app), "`forge` does nothing here");
    }

    #[test]
    fn a_draft_the_forge_does_not_hold_yet_keeps_the_reader_on_the_forge() {
        let mut app = with_review();
        on_line(&mut app);
        press(&mut app, "c");
        type_text(&mut app, "nit");
        type_text(&mut app, ":local");
        assert_eq!(app.take_switch(), None);
        assert_eq!(toast(&app), "1 draft not saved yet: switch once the forge holds them");
    }

    #[test]
    fn a_side_that_did_not_open_says_why_and_the_session_goes_on() {
        let mut app = with_queue();
        app.switch_failed(Side::Forge, "no token for gitlab.com: run `revu login`");
        assert_eq!(toast(&app), "cannot switch to the forge: no token for gitlab.com: run `revu login`");
        assert!(!app.should_quit);
    }

    #[test]
    fn a_switched_session_says_which_side_it_reads() {
        let mut app = local_app();
        app.switched();
        assert_eq!(toast(&app), "local: the worktrees of this checkout · :forge goes back");
    }
}
