//! Cache files as revu has written them: a type that no longer reads one makes the cache drop it,
//! and with it the reader's pins, viewed files and drafts.
#![allow(clippy::unwrap_used)]
use super::MrState;
use super::ai::SavedAnswer;
use crate::cache::Entry;
use crate::forge::{Discussion, Draft as HeldDraft, Mr, MrKey, Queue};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::BTreeSet;

/// `json` reads as `T` and writes back the same JSON.
fn reads_back<T: DeserializeOwned + Serialize>(json: &str) {
    let read: T = serde_json::from_str(json).unwrap();
    let expected: serde_json::Value = serde_json::from_str(json).unwrap();
    assert_eq!(serde_json::to_value(read).unwrap(), expected);
}

#[test]
fn a_cached_mr_reads_back_unchanged() {
    reads_back::<Entry<Mr>>(include_str!("fixtures/mr.json"));
}

#[test]
fn cached_threads_read_back_unchanged() {
    reads_back::<Vec<Discussion>>(include_str!("fixtures/discussions.json"));
}

#[test]
fn cached_drafts_read_back_unchanged() {
    reads_back::<Vec<HeldDraft>>(include_str!("fixtures/drafts.json"));
}

#[test]
fn pins_read_back_unchanged() {
    reads_back::<BTreeSet<MrKey>>(include_str!("fixtures/pins.json"));
}

#[test]
fn a_saved_state_reads_back_unchanged() {
    reads_back::<MrState>(include_str!("fixtures/state.json"));
}

#[test]
fn a_cached_queue_reads_back_unchanged() {
    reads_back::<Entry<Queue>>(include_str!("fixtures/queue.json"));
}

#[test]
fn a_kept_answer_reads_back_unchanged() {
    reads_back::<SavedAnswer>(include_str!("fixtures/answer.json"));
}
