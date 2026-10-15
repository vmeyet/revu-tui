//! Opening an MR: painted from the cache, fetched again, built into a `Review`, and loaded ahead of time.
use super::{Backend, warn_stopped};
use crate::cache::{Entry, keys};
use crate::diff::fold::FoldState;
use crate::forge::{DiffFile, Discussion, Draft as HeldDraft, Mr, MrKey, Sha};
use crate::log::OrWarn as _;
use crate::review::{Draft, Review};
use crate::tui::app::{self, Ahead, Incoming};
use anyhow::Result;
use chrono::{DateTime, Utc};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// What opening an MR fetches.
struct Fetched {
    mr: Mr,
    diffs: Vec<DiffFile>,
    discussions: Vec<Discussion>,
    /// The drafts the forge holds for me.
    drafts: Vec<HeldDraft>,
}

/// Counts an MR being opened for as long as it lives, so loading ahead steps aside.
pub(in crate::tui) struct Opening(Arc<AtomicUsize>);

impl Drop for Opening {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Backend {
    /// Counts an MR as being opened until the returned guard drops.
    pub(in crate::tui) fn start_opening(&self) -> Opening {
        self.opening.fetch_add(1, Ordering::SeqCst);
        Opening(self.opening.clone())
    }

    pub(in crate::tui) fn open_cached(&self, key: &MrKey) -> Option<Incoming> {
        let cache = self.cache_of(key);
        let mr: Entry<Mr> = cache.read_entry(&keys::mr(key))?;
        let diffs: Vec<DiffFile> = cache.read(&keys::diffs(key, &mr.value.refs.head))?;
        let discussions: Vec<Discussion> = cache.read(&keys::discussions(key)).unwrap_or_default();
        let drafts: Vec<HeldDraft> = cache.read(&keys::drafts(key)).unwrap_or_default();
        let age = mr.age(Utc::now());
        let review = self.build(key, mr.value, &diffs, discussions, &drafts);
        Some(Incoming::Review { key: key.clone(), review: Box::new(review), cached: Some(age) })
    }

    pub(in crate::tui) async fn fetch_review(&self, key: MrKey) -> Result<Incoming> {
        let fetched = self.fetch(&key).await?;
        self.reviewed(key, fetched).await
    }

    /// The MR alone while its head is still `head`: no diffs, threads or drafts fetched, nothing highlighted again.
    /// Once the head moved, the review is built again, with the new head's diffs from the cache when loading ahead put them there.
    pub(in crate::tui) async fn poll_review(&self, key: MrKey, head: &Sha) -> Result<Incoming> {
        let forge = self.forge_of(&key);
        let mr = forge.mr(&key).await?;
        if &mr.refs.head == head {
            let (wanted, kept) = (key.clone(), mr.clone());
            self.run_blocking(move |b| {
                b.cache_of(&wanted).write_entry(&keys::mr(&wanted), &kept).or_warn("caching the MR");
                b.mark_opened(&wanted);
            })
            .await?;
            return Ok(Incoming::Mr { key, mr: Box::new(mr) });
        }
        let (diffs, discussions, drafts) =
            tokio::try_join!(self.diffs_at(&key, &mr.refs.head), forge.discussions(&key), forge.drafts(&key))?;
        self.reviewed(key, Fetched { mr, diffs, discussions, drafts }).await
    }

    /// Opening over a painted cache: the MR, its threads and drafts read at once. While the head is still the painted
    /// `head`, no diff is read and the painted one stays; once it moved, the review is built again.
    pub(in crate::tui) async fn reopen(&self, key: MrKey, head: &Sha) -> Result<Incoming> {
        let forge = self.forge_of(&key);
        let (mr, discussions, drafts) = tokio::try_join!(forge.mr(&key), forge.discussions(&key), forge.drafts(&key))?;
        if &mr.refs.head != head {
            let diffs = self.diffs_at(&key, &mr.refs.head).await?;
            return self.reviewed(key, Fetched { mr, diffs, discussions, drafts }).await;
        }
        self.run_blocking(move |b| {
            b.keep_threads(&key, &mr, &discussions, &drafts);
            b.mark_opened(&key);
            let drafts = drafts.iter().map(Draft::from_held).collect();
            Incoming::Threads { key, mr: Box::new(mr), discussions, drafts }
        })
        .await
    }

    pub(in crate::tui) async fn fetch_discussions(&self, key: MrKey) -> Result<Incoming> {
        let discussions = self.forge_of(&key).discussions(&key).await?;
        let (wanted, saved) = (key.clone(), discussions.clone());
        self.write_cache("caching the threads", move |b| b.cache_of(&wanted).write(&keys::discussions(&wanted), &saved)).await;
        Ok(Incoming::Discussions { key, discussions })
    }

    /// Sends where the reader left `key` last time, when a place was saved.
    pub(in crate::tui) async fn resume(&self, key: &MrKey, send: &impl Fn(Incoming)) {
        let wanted = key.clone();
        if let Ok(Some(spot)) = self.run_blocking(move |b| b.state(&wanted).spot).await {
            send(Incoming::Resume { key: key.clone(), spot });
        }
    }

    /// The diffs of `head`, which never change once pushed: from the cache when it holds them.
    async fn diffs_at(&self, key: &MrKey, head: &Sha) -> Result<Vec<DiffFile>> {
        let (wanted, at) = (key.clone(), head.clone());
        let cached = self.read_cache(move |b| b.cache_of(&wanted).read(&keys::diffs(&wanted, &at))).await;
        match cached {
            Some(diffs) => Ok(diffs),
            None => self.forge_of(key).diffs(key).await,
        }
    }

    /// What a fetch brought, kept in the cache and built into a review off the loop.
    async fn reviewed(&self, key: MrKey, fetched: Fetched) -> Result<Incoming> {
        self.run_blocking(move |b| {
            b.keep(&key, &fetched);
            b.mark_opened(&key);
            let Fetched { mr, diffs, discussions, drafts } = fetched;
            let review = b.build(&key, mr, &diffs, discussions, &drafts);
            Incoming::Review { key, review: Box::new(review), cached: None }
        })
        .await
    }

    /// Everything opening an MR needs, from the forge.
    async fn fetch(&self, key: &MrKey) -> Result<Fetched> {
        let forge = self.forge_of(key);
        let (mr, diffs, discussions, drafts) =
            tokio::try_join!(forge.mr(key), forge.diffs(key), forge.discussions(key), forge.drafts(key))?;
        Ok(Fetched { mr, diffs, discussions, drafts })
    }

    /// Writes what a fetch brought where opening reads it; a failed write only costs a later fetch.
    fn keep(&self, key: &MrKey, fetched: &Fetched) {
        self.cache_of(key).write(&keys::diffs(key, &fetched.mr.refs.head), &fetched.diffs).or_warn("caching the diffs");
        self.keep_threads(key, &fetched.mr, &fetched.discussions, &fetched.drafts);
    }

    /// Writes what changes at a head the cache already holds the diffs of.
    fn keep_threads(&self, key: &MrKey, mr: &Mr, discussions: &[Discussion], drafts: &[HeldDraft]) {
        let cache = self.cache_of(key);
        cache.write_entry(&keys::mr(key), mr).or_warn("caching the MR");
        cache.write(&keys::discussions(key), discussions).or_warn("caching the threads");
        cache.write(&keys::drafts(key), drafts).or_warn("caching the drafts");
    }

    /// Loads one MR into the cache ahead of time, quietly. It never marks the MR opened, so its
    /// activity dot stays; it steps aside while an MR is being opened or requests run low.
    pub(in crate::tui) async fn load_ahead(&self, ahead: Ahead) {
        let busy = self.opening.load(Ordering::SeqCst) > 0;
        let low = self.forge_of(&ahead.key).rate().remaining.is_some_and(|left| left < app::prefetch::SPARE);
        let (wanted, seen) = (ahead.key.clone(), ahead.updated_at);
        if busy || low || self.run_blocking(move |b| b.is_fresh(&wanted, seen)).await.inspect_err(warn_stopped).unwrap_or(true) {
            return;
        }
        if let Ok(fetched) = self.fetch(&ahead.key).await {
            let key = ahead.key;
            self.run_blocking(move |b| b.keep(&key, &fetched)).await.or_warn("loading ahead");
        }
    }

    /// The cache already holds this MR as the queue last saw it, diff included.
    fn is_fresh(&self, key: &MrKey, seen: DateTime<Utc>) -> bool {
        let cache = self.cache_of(key);
        cache
            .read_entry::<Mr>(&keys::mr(key))
            .is_some_and(|mr| mr.value.updated_at >= seen && cache.has(&keys::diffs(key, &mr.value.refs.head)))
    }

    fn build(&self, key: &MrKey, mr: Mr, diffs: &[DiffFile], discussions: Vec<Discussion>, drafts: &[HeldDraft]) -> Review {
        let state = self.state(key);
        let review = Review::new(mr, diffs, discussions, &self.fold_globs);
        let fold = merged_fold(review.fold.clone(), state.fold);
        let viewed = review.still_viewed(&state.viewed_files);
        review
            .with_fold(fold)
            .with_viewed(viewed)
            .with_inline(self.inline)
            .with_side_by_side(state.side_by_side)
            .with_drafts(drafts.iter().map(Draft::from_held).collect())
    }
}

/// What the reader folded wins over the defaults, file by file.
fn merged_fold(initial: FoldState, saved: FoldState) -> FoldState {
    let mut files = initial.files;
    files.extend(saved.files);
    let mut hunks = initial.hunks;
    hunks.extend(saved.hunks);
    FoldState { files, hunks }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::cache::Cache;
    use crate::diff::fold::Fold;
    use crate::forge::gitlab::fixture::key;
    use crate::tui::backend::test_support::{backend_on, mount_mr};

    fn ahead(updated: &str) -> Ahead {
        Ahead { key: key(), updated_at: updated.parse().unwrap() }
    }

    #[test]
    fn saved_folds_win_over_the_defaults() {
        let mut initial = FoldState::default();
        initial.files.insert("Cargo.lock".into(), Fold::Closed);
        let mut saved = FoldState::default();
        saved.files.insert("src/a.rs".into(), Fold::Closed);
        let merged = merged_fold(initial, saved);
        assert!(!merged.file_is_open("Cargo.lock"));
        assert!(!merged.file_is_open("src/a.rs"));
    }

    #[tokio::test]
    async fn loading_ahead_fills_the_cache_opening_reads_without_marking_the_mr_opened() {
        let server = wiremock::MockServer::start().await;
        mount_mr(&server, "2026-09-22T09:00:00Z", [1; 5]).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend { cache: Cache::in_dir(dir.path()), ..backend_on(&server) };
        backend.load_ahead(ahead("2026-09-22T09:00:00Z")).await;
        let Some(Incoming::Review { cached, .. }) = backend.open_cached(&key()) else { panic!("opening paints from the cache") };
        assert!(cached.is_some());
        assert_eq!(backend.state(&key()).opened_at, None, "loading ahead never hides the activity dot");
    }

    #[tokio::test]
    async fn a_cache_as_new_as_the_queue_is_not_fetched_again_and_a_newer_change_is() {
        let server = wiremock::MockServer::start().await;
        mount_mr(&server, "2026-09-22T09:00:00Z", [2; 5]).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend { cache: Cache::in_dir(dir.path()), ..backend_on(&server) };
        backend.load_ahead(ahead("2026-09-22T09:00:00Z")).await;
        backend.load_ahead(ahead("2026-09-22T09:00:00Z")).await;
        backend.load_ahead(ahead("2026-09-22T10:00:00Z")).await;
    }

    #[tokio::test]
    async fn a_poll_reads_only_the_mr_while_the_head_stays_and_a_new_heads_diffs_come_from_the_cache() {
        let server = wiremock::MockServer::start().await;
        mount_mr(&server, "2026-09-22T09:00:00Z", [2, 2, 0, 1, 1]).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend { cache: Cache::in_dir(dir.path()), ..backend_on(&server) };
        let Ok(Incoming::Mr { mr, .. }) = backend.poll_review(key(), &"bbbb".into()).await else { panic!("the head did not move") };
        assert_eq!(mr.refs.head.as_str(), "bbbb");
        assert!(backend.state(&key()).opened_at.is_some());
        backend.cache.write(&keys::diffs(&key(), &"bbbb".into()), &Vec::<DiffFile>::new()).unwrap();
        let Ok(Incoming::Review { review, cached: None, .. }) = backend.poll_review(key(), &"aaaa".into()).await else {
            panic!("the head moved")
        };
        assert!(review.files.is_empty(), "the diffs came from the cache, not the forge");
    }

    #[tokio::test]
    async fn reopening_at_the_painted_head_reads_no_diff_and_a_moved_head_builds_the_review_again() {
        let server = wiremock::MockServer::start().await;
        mount_mr(&server, "2026-09-22T09:00:00Z", [2, 2, 0, 2, 2]).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend { cache: Cache::in_dir(dir.path()), ..backend_on(&server) };
        let Ok(Incoming::Threads { mr, .. }) = backend.reopen(key(), &"bbbb".into()).await else { panic!("the head did not move") };
        assert_eq!(mr.refs.head.as_str(), "bbbb");
        assert!(backend.state(&key()).opened_at.is_some());
        backend.cache.write(&keys::diffs(&key(), &"bbbb".into()), &Vec::<DiffFile>::new()).unwrap();
        let Ok(Incoming::Review { review, cached: None, .. }) = backend.reopen(key(), &"aaaa".into()).await else {
            panic!("the head moved")
        };
        assert!(review.files.is_empty(), "the diffs came from the cache, not the forge");
    }

    #[tokio::test]
    async fn loading_ahead_steps_aside_while_an_mr_opens() {
        let server = wiremock::MockServer::start().await;
        mount_mr(&server, "2026-09-22T09:00:00Z", [0; 5]).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend { cache: Cache::in_dir(dir.path()), ..backend_on(&server) };
        let _opening = backend.start_opening();
        backend.load_ahead(ahead("2026-09-22T09:00:00Z")).await;
    }

    #[tokio::test]
    async fn loading_ahead_stops_when_requests_run_low() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, ResponseTemplate};
        let server = wiremock::MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/user"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("ratelimit-remaining", "150")
                    .set_body_json(serde_json::json!({"id": 1, "username": "nina", "name": "Nina"})),
            )
            .mount(&server)
            .await;
        mount_mr(&server, "2026-09-22T09:00:00Z", [0; 5]).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend { cache: Cache::in_dir(dir.path()), ..backend_on(&server) };
        backend.forge.me().await.unwrap();
        backend.load_ahead(ahead("2026-09-22T09:00:00Z")).await;
    }
}
