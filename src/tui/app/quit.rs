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
