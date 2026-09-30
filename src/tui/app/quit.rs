//! `q` and `ctrl-c` ask for a second press, so one stray key never throws the session away.
use super::{Action, App};
use std::time::Duration;

/// How long the first press waits for the second.
pub const QUIT_WINDOW: Duration = Duration::from_millis(1500);

/// The key that started a quit: only the same key finishes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuitKey {
    Q,
    CtrlC,
}

impl QuitKey {
    fn name(self) -> &'static str {
        match self {
            QuitKey::Q => "q",
            QuitKey::CtrlC => "ctrl-c",
        }
    }
}

impl App {
    /// Quits on the second press of the same key inside the window, or at once when
    /// `[keys] quit_confirm = false`; the first press only asks.
    pub(super) fn quit_key(&mut self, key: QuitKey) -> Vec<Action> {
        let again = self.quitting.is_some_and(|(pending, at)| pending == key && self.now.duration_since(at) < QUIT_WINDOW);
        if again || !self.quit_confirm {
            self.should_quit = true;
            self.quitting = None;
        } else {
            self.quitting = Some((key, self.now));
        }
        vec![]
    }

    /// The status line while a quit waits for its second press, naming what would be lost.
    pub fn quit_prompt(&self) -> Option<String> {
        let (key, at) = self.quitting?;
        if self.now.duration_since(at) >= QUIT_WINDOW {
            return None;
        }
        let unsaved = self.unsaved_drafts();
        let mut lost = vec![];
        if unsaved > 0 {
            lost.push(format!("{unsaved} draft{} unsaved", if unsaved == 1 { "" } else { "s" }));
        }
        if self.input.is_some() && !self.buffer.text().trim().is_empty() {
            lost.push("a comment in the box".to_owned());
        }
        Some(if lost.is_empty() {
            format!("press {} again to quit", key.name())
        } else {
            format!("{} · {} again to quit", lost.join(" · "), key.name())
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crate::tui::app::test_support::*;

    #[test]
    fn q_closes_the_right_pane_before_it_asks_to_quit() {
        let mut app = with_review();
        press(&mut app, "]Nl");
        assert!(app.open.as_ref().unwrap().pane.is_some());
        press(&mut app, "q");
        assert!(app.open.as_ref().unwrap().pane.is_none(), "the thread pane closes first");
        assert_eq!((app.focus, app.quit_prompt()), (Focus::Review, None));
        press(&mut app, "tq");
        assert!(app.open.as_ref().unwrap().tree.is_none(), "so does the tree");
        assert_eq!(app.quit_prompt(), None);
        press(&mut app, "qq");
        assert!(app.should_quit, "nothing left to close: the double press quits");
    }

    #[test]
    fn q_from_the_queue_asks_to_quit_even_with_a_pane_open() {
        let mut app = with_review();
        press(&mut app, "t");
        app.focus = Focus::Queue;
        press(&mut app, "q");
        assert!(app.open.as_ref().unwrap().tree.is_some());
        assert!(app.quit_prompt().is_some());
    }

    #[test]
    fn q_asks_first_and_quits_on_the_second_press() {
        let mut app = with_review();
        press(&mut app, "q");
        assert!(!app.should_quit);
        assert_eq!(app.quit_prompt().as_deref(), Some("press q again to quit"));
        assert!(render(&mut app, 120, 24).contains("press q again to quit"));
        press(&mut app, "q");
        assert!(app.should_quit);
    }

    #[test]
    fn a_single_q_times_out() {
        let mut app = with_review();
        press(&mut app, "q");
        app.now += crate::tui::app::quit::QUIT_WINDOW;
        assert_eq!(app.quit_prompt(), None, "the prompt is gone");
        press(&mut app, "q");
        assert!(!app.should_quit, "a late second press only asks again");
        assert!(app.quit_prompt().is_some());
    }

    #[test]
    fn another_key_calls_the_quit_off_and_does_its_own_job() {
        let mut app = with_review();
        let before = app.open.as_ref().unwrap().selected;
        press(&mut app, "q");
        press(&mut app, "j");
        assert_eq!(app.quit_prompt(), None);
        assert_ne!(app.open.as_ref().unwrap().selected, before, "j still moves");
        press(&mut app, "q");
        assert!(!app.should_quit, "the next q starts over");
    }

    #[test]
    fn ctrl_c_quits_on_its_own_second_press_only() {
        let mut app = with_review();
        app.handle_key(ctrl('c'));
        assert_eq!(app.quit_prompt().as_deref(), Some("press ctrl-c again to quit"));
        press(&mut app, "q");
        assert!(!app.should_quit, "q does not finish a ctrl-c quit");
        app.handle_key(ctrl('c'));
        app.handle_key(ctrl('c'));
        assert!(app.should_quit);
    }

    #[test]
    fn the_prompt_names_unsaved_drafts_and_a_comment_in_the_box() {
        let mut app = with_review();
        on_line(&mut app);
        press(&mut app, "c");
        type_text(&mut app, "nit");
        press(&mut app, "qq");
        assert_eq!(app.quit_prompt().as_deref(), Some("1 draft unsaved · q again to quit"), "the first q closed the pane");
        press(&mut app, "c");
        press(&mut app, "half");
        app.handle_key(ctrl('c'));
        assert_eq!(app.quit_prompt().as_deref(), Some("1 draft unsaved · a comment in the box · ctrl-c again to quit"));
    }

    #[test]
    fn colon_q_quits_at_once_and_one_press_is_back_with_quit_confirm_off() {
        let mut app = with_review();
        press(&mut app, ":q");
        app.handle_key(code(KeyCode::Enter));
        assert!(app.should_quit, ":q is deliberate");
        let mut quick = App::new(Settings { quit_confirm: false, ..settings() });
        press(&mut quick, "q");
        assert!(quick.should_quit);
    }

    #[test]
    fn q_still_closes_a_modal_before_it_asks() {
        let mut app = with_review();
        press(&mut app, "i");
        press(&mut app, "q");
        assert!(app.brief.is_none() && app.quit_prompt().is_none(), "the modal closes, nothing asks");
        let mut app = with_saved_draft();
        press(&mut app, "Pq");
        assert!(app.publish.is_none() && app.quit_prompt().is_none(), "so does the publish list");
    }

    #[test]
    fn zen_shows_the_quit_prompt() {
        let mut app = with_review();
        press(&mut app, "zzq");
        assert!(render(&mut app, 160, 45).contains("press q again to quit"));
    }
}
