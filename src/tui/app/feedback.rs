use super::App;
use std::time::{Duration, Instant};

const TOAST_LIFE: Duration = Duration::from_secs(4);
/// In zen a toast shows only this long, then leaves the screen quiet.
const ZEN_TOAST: Duration = Duration::from_secs(2);
/// How long before its end a toast stops showing in zen.
const ZEN_TOAST_GONE: Duration = Duration::from_secs(TOAST_LIFE.as_secs() - ZEN_TOAST.as_secs());

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

    /// In its first two seconds: how long zen shows it.
    pub fn fresh(&self, now: Instant) -> bool {
        now + ZEN_TOAST_GONE < self.until
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
