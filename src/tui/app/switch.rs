//! `:local` and `:forge`: this session ends and the loop opens the other space in the same screen.
use super::App;

/// What a session reads: the worktrees of the checkout, or the forge's queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Space {
    Local,
    Forge,
}

impl Space {
    fn name(self) -> &'static str {
        match self {
            Self::Local => "the worktrees of this checkout",
            Self::Forge => "the forge",
        }
    }
}

impl App {
    /// The loop takes the switch once this key is handled; a draft the forge does not hold yet keeps the reader here.
    pub(super) fn switch_space(&mut self, space: Space) {
        let unsaved = self.unsaved_drafts();
        if unsaved > 0 {
            self.warn(format!("{unsaved} draft{} not saved yet: switch once the forge holds them", if unsaved == 1 { "" } else { "s" }));
            return;
        }
        self.switch = Some(space);
    }

    /// The loop stops: the reader quits, or asked for the other space.
    pub(crate) fn session_over(&self) -> bool {
        self.should_quit || self.switch.is_some()
    }

    /// Taken by the loop, which ends this session and opens the space asked for.
    pub(crate) fn take_switch(&mut self) -> Option<Space> {
        self.switch.take()
    }

    /// Shown while the loop opens `space`, which keeps the screen still.
    pub(crate) fn opening(&mut self, space: Space) {
        self.toast(format!("opening {}…", space.name()));
    }

    /// The first words of a session the reader switched to; a toast the session started with says more, and stays.
    pub(crate) fn switched(&mut self) {
        if self.live_toast().is_some() {
            return;
        }
        self.toast(if self.is_local() {
            "local: the worktrees of this checkout · :forge goes back"
        } else {
            "forge: your queue · :local reads the worktrees of this checkout"
        });
    }

    /// The other space did not open: this session goes on as it was.
    pub(crate) fn switch_failed(&mut self, space: Space, reason: &str) {
        self.warn(format!("cannot open {}: {reason}", space.name()));
    }
}

#[cfg(test)]
mod tests {
    use super::Space;
    use crate::tui::app::test_support::*;

    #[test]
    fn local_on_a_forge_asks_the_loop_for_the_worktrees() {
        let mut app = with_queue();
        type_text(&mut app, ":local");
        assert!(app.session_over());
        assert_eq!(app.take_switch(), Some(Space::Local));
        assert_eq!(app.take_switch(), None, "taken once");
    }

    #[test]
    fn forge_on_a_local_diff_asks_the_loop_for_the_forge() {
        let mut app = local_app();
        type_text(&mut app, ":forge");
        assert_eq!(app.take_switch(), Some(Space::Forge));
    }

    #[test]
    fn forge_on_the_forge_is_no_switch() {
        let mut app = with_queue();
        type_text(&mut app, ":forge");
        assert!(!app.session_over());
        assert_eq!(toast(&app), "`forge` does nothing here");
    }

    #[test]
    fn local_on_a_local_diff_is_no_switch() {
        let mut app = local_app();
        type_text(&mut app, ":local");
        assert!(!app.session_over());
        assert_eq!(toast(&app), "`local` does nothing here");
    }

    #[test]
    fn a_draft_the_forge_does_not_hold_yet_keeps_the_reader_on_the_forge() {
        let mut app = with_review();
        on_line(&mut app);
        press(&mut app, "c");
        type_text(&mut app, "nit");
        type_text(&mut app, ":local");
        assert!(!app.session_over());
        assert_eq!(toast(&app), "1 draft not saved yet: switch once the forge holds them");
    }

    #[test]
    fn a_space_that_did_not_open_leaves_the_session_running_and_says_why() {
        let mut app = with_queue();
        type_text(&mut app, ":local");
        let space = app.take_switch().unwrap();
        app.opening(space);
        assert_eq!(toast(&app), "opening the worktrees of this checkout…");
        app.switch_failed(space, "not a git repository");
        assert!(!app.session_over(), "the loop runs this session again");
        assert_eq!(toast(&app), "cannot open the worktrees of this checkout: not a git repository");
    }

    #[test]
    fn a_switched_local_session_says_it_reads_the_worktrees() {
        let mut app = local_app();
        app.toast = None;
        app.switched();
        assert_eq!(toast(&app), "local: the worktrees of this checkout · :forge goes back");
    }

    #[test]
    fn a_switched_forge_session_says_it_reads_the_queue() {
        let mut app = with_queue();
        app.toast = None;
        app.switched();
        assert_eq!(toast(&app), "forge: your queue · :local reads the worktrees of this checkout");
    }

    #[test]
    fn a_switched_session_keeps_the_toast_it_started_with() {
        let mut app = with_queue();
        app.warn("config: `tui.colour` is not a setting");
        app.switched();
        assert_eq!(toast(&app), "config: `tui.colour` is not a setting");
    }
}
