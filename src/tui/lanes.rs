//! One lane per MR: whatever enters it runs one at a time.
use crate::forge::MrKey;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

#[derive(Clone, Debug, Default)]
pub(super) struct Lanes(Arc<Mutex<HashMap<MrKey, Arc<AsyncMutex<()>>>>>);

impl Lanes {
    /// Waits until the lane of `key` is free; it stays taken until the guard drops.
    pub(super) async fn enter(&self, key: &MrKey) -> OwnedMutexGuard<()> {
        self.lane_of(key).lock_owned().await
    }

    fn lane_of(&self, key: &MrKey) -> Arc<AsyncMutex<()>> {
        let mut lanes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(lanes.entry(key.clone()).or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    async fn most_at_once(keys: [MrKey; 3]) -> usize {
        let lanes = Lanes::default();
        let (running, most) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let (lanes, running, most) = (&lanes, &running, &most);
        let run = |key: MrKey| async move {
            let _lane = lanes.enter(&key).await;
            let now = running.fetch_add(1, Ordering::SeqCst) + 1;
            most.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            running.fetch_sub(1, Ordering::SeqCst);
        };
        let [a, b, c] = keys;
        tokio::join!(run(a), run(b), run(c));
        most.load(Ordering::SeqCst)
    }

    #[tokio::test(start_paused = true)]
    async fn one_mr_runs_one_at_a_time_and_two_mrs_run_side_by_side() {
        let one = MrKey::new("acme/widgets", 42);
        assert_eq!(most_at_once([one.clone(), one.clone(), one]).await, 1);
        assert_eq!(most_at_once([42, 43, 44].map(|n| MrKey::new("acme/widgets", n))).await, 3);
    }
}
