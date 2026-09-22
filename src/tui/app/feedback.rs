use super::App;
use std::time::{Duration, Instant};

const TOAST_LIFE: Duration = Duration::from_secs(4);

/// Something that just happened, shown over the status line until it ages out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub text: String,
    pub danger: bool,
    until: Instant,
}

impl Toast {
    pub fn live(&self, now: Instant) -> bool {
        now < self.until
    }
}

impl App {
    pub(super) fn toast(&mut self, text: impl Into<String>) {
        self.toast = Some(Toast { text: text.into(), danger: false, until: self.now + TOAST_LIFE });
    }

    pub(super) fn warn(&mut self, text: impl Into<String>) {
        self.toast = Some(Toast { text: text.into(), danger: true, until: self.now + TOAST_LIFE });
    }

    pub fn live_toast(&self) -> Option<&Toast> {
        self.toast.as_ref().filter(|t| t.live(self.now))
    }
}
