//! How many requests the forge still allows, and whether a request is waiting out a rate limit.
//! Shared by every clone of a client, so the TUI reads what the background tasks saw.
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const UNKNOWN: u64 = u64::MAX;

#[derive(Clone, Debug)]
pub struct Budget {
    remaining: Arc<AtomicU64>,
    /// Unix seconds a rate-limited request sleeps until; 0 when none waits.
    waiting_until: Arc<AtomicU64>,
}

impl Default for Budget {
    fn default() -> Self {
        Self { remaining: Arc::new(AtomicU64::new(UNKNOWN)), waiting_until: Arc::new(AtomicU64::new(0)) }
    }
}

/// What the TUI shows and paces itself on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RateLimit {
    /// Requests left in the current window, once an answer said.
    pub remaining: Option<u64>,
    /// How long until a waiting request goes again.
    pub wait: Option<Duration>,
}

impl RateLimit {
    /// Below this many requests left, polling slows down so the window lasts.
    pub const LOW: u64 = 100;

    pub fn is_low(self) -> bool {
        self.remaining.is_some_and(|left| left < Self::LOW)
    }
}

impl Budget {
    /// Keeps the count an answer's header gave, when it gave one.
    pub fn note(&self, remaining: Option<u64>) {
        if let Some(left) = remaining {
            self.remaining.store(left, Ordering::Relaxed);
        }
    }

    pub fn waiting_for(&self, wait: Duration) {
        self.waiting_until.store(unix_now() + wait.as_secs(), Ordering::Relaxed);
    }

    pub fn done_waiting(&self) {
        self.waiting_until.store(0, Ordering::Relaxed);
    }

    pub fn now(&self) -> RateLimit {
        let remaining = Some(self.remaining.load(Ordering::Relaxed)).filter(|n| *n != UNKNOWN);
        let until = self.waiting_until.load(Ordering::Relaxed);
        let wait = (until > 0).then(|| Duration::from_secs(until.saturating_sub(unix_now())));
        RateLimit { remaining, wait }
    }
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// A header's number, as both forges send their rate-limit counts.
pub fn header_number(headers: &reqwest::header::HeaderMap, name: &str) -> Option<u64> {
    headers.get(name).and_then(|h| h.to_str().ok()?.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn clones_share_what_one_saw_and_a_wait_counts_down() {
        let budget = Budget::default();
        assert_eq!(budget.now(), RateLimit::default());
        let clone = budget.clone();
        clone.note(Some(42));
        clone.note(None);
        assert_eq!(budget.now().remaining, Some(42));
        assert!(budget.now().is_low());
        clone.waiting_for(Duration::from_secs(30));
        assert!(budget.now().wait.is_some_and(|w| (29..=30).contains(&w.as_secs())));
        clone.done_waiting();
        assert_eq!(budget.now().wait, None);
    }
}
