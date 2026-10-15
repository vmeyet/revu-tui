//! How many requests the forge still allows, and whether a request is waiting out a rate limit.
//! Shared by every clone of a client, so the TUI reads what the background tasks saw.
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const UNKNOWN: u64 = u64::MAX;

#[derive(Clone, Debug)]
pub(super) struct Budget {
    remaining: Arc<AtomicU64>,
    /// Unix seconds the forge asked to wait until; 0, or a time gone by, when it asks none.
    waiting_until: Arc<AtomicU64>,
}

impl Default for Budget {
    fn default() -> Self {
        Self { remaining: Arc::new(AtomicU64::new(UNKNOWN)), waiting_until: Arc::new(AtomicU64::new(0)) }
    }
}

/// What the TUI shows and paces itself on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RateLimit {
    /// Requests left in the current window, once an answer said.
    pub(crate) remaining: Option<u64>,
    /// How long until the forge takes requests again, while it asked to wait.
    pub(crate) wait: Option<Duration>,
}

impl RateLimit {
    /// Below this many requests left, polling slows down so the window lasts.
    const LOW: u64 = 100;

    pub(crate) fn is_low(self) -> bool {
        self.remaining.is_some_and(|left| left < Self::LOW)
    }
}

impl Budget {
    /// Keeps the count an answer's header gave, when it gave one.
    pub(crate) fn note(&self, remaining: Option<u64>) {
        if let Some(left) = remaining {
            self.remaining.store(left, Ordering::Relaxed);
        }
    }

    /// The later of two asked waits holds: a short one never hides a long one.
    pub(super) fn waiting_for(&self, wait: Duration) {
        self.waiting_until.fetch_max(unix_now() + wait.as_secs(), Ordering::Relaxed);
    }

    pub(crate) fn now(&self) -> RateLimit {
        let remaining = Some(self.remaining.load(Ordering::Relaxed)).filter(|n| *n != UNKNOWN);
        let (until, now) = (self.waiting_until.load(Ordering::Relaxed), unix_now());
        let wait = (until > now).then(|| Duration::from_secs(until - now));
        RateLimit { remaining, wait }
    }
}

pub(super) fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// A header's number, as both forges send their rate-limit counts.
pub(super) fn header_number(headers: &reqwest::header::HeaderMap, name: &str) -> Option<u64> {
    headers.get(name).and_then(|h| h.to_str().ok()?.trim().parse().ok())
}

#[cfg(test)]
mod tests {
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
        clone.waiting_for(Duration::from_secs(5));
        assert!(budget.now().wait.is_some_and(|w| (29..=30).contains(&w.as_secs())), "the longer wait holds");
        let other = Budget::default();
        other.waiting_for(Duration::ZERO);
        assert_eq!(other.now().wait, None, "a wait gone by is none");
    }
}
