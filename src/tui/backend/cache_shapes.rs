//! Cache files as revu has written them: a type that no longer reads one makes the cache drop it,
//! and with it the reader's pins, viewed files and drafts.
use super::MrState;
use crate::cache::Entry;
use crate::forge::{Discussion, Draft as HeldDraft, MergeVerdict, Mr, MrKey, Queue};
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
fn an_mr_cached_before_revu_read_the_merge_verdict_still_reads_and_is_not_ready() {
    let mut older: serde_json::Value = serde_json::from_str(include_str!("fixtures/mr.json")).unwrap();
    older["value"].as_object_mut().unwrap().remove("merge_verdict");
    let read: Entry<Mr> = serde_json::from_value(older).unwrap();
    assert_eq!(read.value.merge_verdict, MergeVerdict::Unknown);
}

#[test]
fn cached_threads_read_back_unchanged() {
    reads_back::<Vec<Discussion>>(include_str!("fixtures/discussions.json"));
}

#[test]
fn threads_cached_before_a_note_said_its_kind_still_read() {
    let mut older: serde_json::Value = serde_json::from_str(include_str!("fixtures/discussions.json")).unwrap();
    older[0]["notes"][0].as_object_mut().unwrap().remove("kind");
    let read: Vec<Discussion> = serde_json::from_value(older).unwrap();
    assert_eq!(read[0].notes[0].kind, None);
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
