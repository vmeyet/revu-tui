//! What the reader keeps between runs: each MR's `state.json`, the pins and the queue view.
use super::Homes;
use crate::cache::keys;
use crate::diff::fold::FoldState;
use crate::forge::{MrKey, ProjectPath, Sections};
use crate::log::OrWarn as _;
use crate::review::Progress;
use crate::tui::app::{self, PinChange, QueueView};
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex, PoisonError};

/// What survives between two openings of one MR, in the cache.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(in crate::tui) struct MrState {
    #[serde(default)]
    pub(in crate::tui) fold: FoldState,
    /// Viewed files by path, with the fingerprint of the change seen.
    #[serde(default)]
    pub(in crate::tui) viewed_files: BTreeMap<String, String>,
    #[serde(default)]
    pub(in crate::tui) auto_folded: BTreeSet<String>,
    #[serde(default)]
    pub(in crate::tui) opened_at: Option<DateTime<Utc>>,
    /// Saved as `split` before side by side replaced the split diff under `D`.
    #[serde(default, alias = "split")]
    pub(in crate::tui) side_by_side: bool,
    /// Where the cursor rested last: the MR opens there next time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::tui) spot: Option<app::Spot>,
}

/// Writes of what the reader keeps, in the order the reader made them: each runs on its own blocking thread,
/// so they can finish out of order.
#[derive(Clone)]
pub(in crate::tui) struct Saves {
    homes: Arc<Homes>,
    /// The order of the last state save written per MR; also the one lock every write of `state.json` takes,
    /// shared by every task so a save and the opened stamp never write over each other.
    saved: Arc<Mutex<HashMap<MrKey, u64>>>,
    /// The order of the last change written per cache entry and target, pins and the queue view, shared by every task.
    changed: Arc<Mutex<HashMap<String, u64>>>,
}

impl Saves {
    pub(super) fn new(homes: Arc<Homes>) -> Self {
        Self { homes, saved: Arc::default(), changed: Arc::default() }
    }

    pub(super) fn state(&self, key: &MrKey) -> MrState {
        self.homes.cache_of(key).read(&keys::state(key)).unwrap_or_default()
    }

    pub(in crate::tui) fn pins(&self, scope: Option<&ProjectPath>) -> BTreeSet<MrKey> {
        self.homes.cache.read(&keys::pins(scope)).unwrap_or_default()
    }

    pub(in crate::tui) fn queue_view(&self, scope: Option<&ProjectPath>) -> Option<QueueView> {
        self.homes.cache.read(&keys::queue_view(scope))
    }

    /// Saves what the reader chose unless a newer save of `key` already landed; a `None` spot keeps the place saved before.
    pub(in crate::tui) fn save_state(&self, key: &MrKey, order: u64, chosen: MrState) -> Result<()> {
        let mut saved = self.saved.lock().unwrap_or_else(PoisonError::into_inner);
        if saved.get(key).is_some_and(|&last| last >= order) {
            return Ok(());
        }
        let before = self.state(key);
        let state = MrState { spot: chosen.spot.or(before.spot), opened_at: before.opened_at, ..chosen };
        self.homes.cache_of(key).write(&keys::state(key), &state)?;
        saved.insert(key.clone(), order);
        drop(saved);
        Ok(())
    }

    /// Under the lock of the reader's saves, so neither writes over what the other just wrote.
    pub(super) fn mark_opened(&self, key: &MrKey) {
        let _saving = self.saved.lock().unwrap_or_else(PoisonError::into_inner);
        let state = MrState { opened_at: Some(Utc::now()), ..self.state(key) };
        self.homes.cache_of(key).write(&keys::state(key), &state).or_warn("marking the MR opened");
    }

    pub(in crate::tui) fn change_pin(&self, scope: Option<&ProjectPath>, key: &MrKey, change: PinChange, order: u64) -> Result<()> {
        let change = |pins: BTreeSet<MrKey>| app::with_member(pins, key.clone(), change == PinChange::Pin);
        self.change_entry(&keys::pins(scope), &format!("{key:?}"), order, change)
    }

    pub(in crate::tui) fn change_queue_view(&self, scope: Option<&ProjectPath>, change: app::ViewChange, order: u64) -> Result<()> {
        self.change_entry(&keys::queue_view(scope), &change.target(), order, |view| change.apply(view))
    }

    /// Changes `entry` as the cache holds it, so another revu's changes stay, unless a newer change of
    /// `target` already landed.
    fn change_entry<T>(&self, entry: &str, target: &str, order: u64, change: impl FnOnce(T) -> T) -> Result<()>
    where
        T: Serialize + serde::de::DeserializeOwned + Default,
    {
        let mut changed = self.changed.lock().unwrap_or_else(PoisonError::into_inner);
        let id = format!("{entry} {target}");
        if changed.get(&id).is_some_and(|&last| last >= order) {
            return Ok(());
        }
        self.homes.cache.update(entry, change)?;
        changed.insert(id, order);
        drop(changed);
        Ok(())
    }

    pub(super) fn opened_at(&self, sections: &Sections) -> HashMap<MrKey, DateTime<Utc>> {
        sections
            .all()
            .filter_map(|mr| {
                let key = mr.key();
                let state: MrState = self.homes.cache_of(&key).read(&keys::state(&key))?;
                Some((key, state.opened_at?))
            })
            .collect()
    }

    /// How far I went in each MR of the queue that I started, read from its saved state.
    pub(in crate::tui) fn progress(&self, sections: &Sections) -> HashMap<MrKey, Progress> {
        sections
            .all()
            .filter_map(|mr| {
                let files = mr.size?.files as usize;
                let key = mr.key();
                let state: MrState = self.homes.cache_of(&key).read(&keys::state(&key))?;
                let viewed: BTreeSet<String> = state.viewed_files.into_keys().collect();
                (!viewed.is_empty()).then(|| (key, Progress::new(files, &viewed, &state.auto_folded)))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::gitlab::fixture::key;
    use crate::tui::backend::test_support::homes_in;

    #[test]
    fn state_round_trips_and_tolerates_an_empty_file() {
        let state: MrState = serde_json::from_str("{}").unwrap();
        assert_eq!(state, MrState::default());
        let before: MrState = serde_json::from_str(r#"{"split": true}"#).unwrap();
        assert!(before.side_by_side, "the split choice saved before reads as side by side");
        let dir = tempfile::tempdir().unwrap();
        let saves = Saves::new(homes_in(dir.path()).into());
        let spot = app::Spot { path: "a.rs".into(), old: None, new: Some(3) };
        let viewed = BTreeMap::from([("a.rs".to_owned(), "f1".to_owned())]);
        let chosen = MrState { viewed_files: viewed.clone(), side_by_side: true, ..MrState::default() };
        saves.save_state(&key(), 1, MrState { spot: Some(spot.clone()), ..chosen.clone() }).unwrap();
        saves.save_state(&key(), 2, chosen).unwrap();
        let state = saves.state(&key());
        assert_eq!((state.viewed_files, state.side_by_side), (viewed, true), "the side by side choice is remembered per MR");
        assert_eq!(state.spot, Some(spot), "a save without a place keeps the one saved before");
    }

    #[test]
    fn a_state_save_finishing_after_a_newer_one_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let saves = Saves::new(homes_in(dir.path()).into());
        saves.save_state(&key(), 2, MrState { side_by_side: true, ..MrState::default() }).unwrap();
        saves.save_state(&key(), 1, MrState::default()).unwrap();
        saves.mark_opened(&key());
        saves.save_state(&key(), 2, MrState::default()).unwrap();
        let state = saves.state(&key());
        assert!(state.side_by_side, "the older save and a repeat of the newest one are dropped");
        assert!(state.opened_at.is_some(), "stamping the MR opened keeps what the reader chose");
    }

    #[test]
    fn state_saves_and_opened_stamps_running_at_once_lose_no_write() {
        const ROUNDS: u64 = 100;
        let dir = tempfile::tempdir().unwrap();
        let saves = Saves::new(homes_in(dir.path()).into());
        let start = std::sync::Barrier::new(2);
        let saving = std::sync::atomic::AtomicBool::new(true);
        std::thread::scope(|threads| {
            threads.spawn(|| {
                start.wait();
                for order in 1..=ROUNDS {
                    let viewed = BTreeMap::from([("a.rs".to_owned(), order.to_string())]);
                    saves.save_state(&key(), order, MrState { viewed_files: viewed, ..MrState::default() }).unwrap();
                }
                saving.store(false, std::sync::atomic::Ordering::SeqCst);
            });
            threads.spawn(|| {
                start.wait();
                saves.mark_opened(&key());
                while saving.load(std::sync::atomic::Ordering::SeqCst) {
                    saves.mark_opened(&key());
                }
            });
        });
        let state = saves.state(&key());
        assert_eq!(state.viewed_files["a.rs"], ROUNDS.to_string(), "no stamp wrote an older state over the last save");
        assert!(state.opened_at.is_some(), "no save wrote over the stamp");
    }

    #[test]
    fn two_pin_changes_finishing_out_of_order_keep_the_later_one_made() {
        let dir = tempfile::tempdir().unwrap();
        let saves = Saves::new(homes_in(dir.path()).into());
        let (infra, widgets) = (MrKey::new("acme/infra", 35), key());
        saves.change_pin(None, &infra, PinChange::Unpin, 2).unwrap();
        saves.change_pin(None, &widgets, PinChange::Pin, 3).unwrap();
        saves.change_pin(None, &infra, PinChange::Pin, 1).unwrap();
        assert_eq!(saves.pins(None), [widgets].into(), "the pin of 35, made before its unpin, lands after it and changes nothing");
    }

    #[test]
    fn two_revus_sharing_a_cache_keep_each_others_pins_and_queue_view() {
        let dir = tempfile::tempdir().unwrap();
        let desktop = Saves::new(homes_in(dir.path()).into());
        let phone = Saves::new(homes_in(dir.path()).into());
        let (infra, widgets) = (MrKey::new("acme/infra", 35), key());
        desktop.change_pin(None, &infra, PinChange::Pin, 1).unwrap();
        phone.change_pin(None, &widgets, PinChange::Pin, 1).unwrap();
        assert_eq!(desktop.pins(None), [infra.clone(), widgets.clone()].into());
        desktop.change_pin(None, &infra, PinChange::Unpin, 2).unwrap();
        assert_eq!(phone.pins(None), [widgets].into());
        desktop.change_queue_view(None, app::ViewChange::Section { name: "OPEN".into(), closed: true }, 3).unwrap();
        phone.change_queue_view(None, app::ViewChange::Section { name: "DONE".into(), closed: false }, 2).unwrap();
        let view = desktop.queue_view(None).unwrap();
        assert!(view.closed_sections.contains("OPEN"));
        assert!(!view.closed_sections.contains("DONE") && view.closed_sections.contains("DRAFTS"), "{view:?}");
    }
}
