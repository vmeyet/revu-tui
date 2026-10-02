//! The service behind the loop: the forge, the cache and the AI, answering actions off the event loop.
use super::{app, blocking, images};
use crate::ai::anthropic::{self, Ask, Claude, Outcome, Stop, Usage};
use crate::ai::triage::{self, Verdict};
use crate::ai::typesafe::{TypeSafe, Unavailable};
use crate::cache::{Cache, Entry, keys};
use crate::diff::fold::FoldState;
use crate::diff::words::InlineRule;
use crate::forge::{DiffFile, Discussion, Draft as HeldDraft, Forge, Mr, MrKey, Queue, Sections, Sha};
use crate::ready::Source as ReadySource;
use crate::review::{Draft, Progress, Review};
use anyhow::{Context as _, Result};
use app::{Ahead, Incoming, Part};
use chrono::{DateTime, Utc};
use futures_util::{StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Duration;

/// Files the outline reads from the forge at once, so a large MR does not burst into its rate limit.
const OUTLINE_READS: usize = 8;

/// What survives between two openings of one MR, in the cache.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct MrState {
    #[serde(default)]
    pub(super) fold: FoldState,
    /// Viewed files by path, with the fingerprint of the change seen.
    #[serde(default)]
    pub(super) viewed_files: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) auto_folded: BTreeSet<String>,
    #[serde(default)]
    pub(super) opened_at: Option<DateTime<Utc>>,
    /// Saved as `split` before side by side replaced the split diff under `D`.
    #[serde(default, alias = "split")]
    pub(super) side_by_side: bool,
    /// Where the cursor rested last: the MR opens there next time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) spot: Option<app::Spot>,
}

#[derive(Clone)]
pub(super) struct Backend {
    pub(super) forge: Forge,
    pub(super) cache: Cache,
    /// The other hosts I am logged in to: every project's queue asks them too.
    pub(super) others: Vec<crate::ctx::Home>,
    pub(super) fold_globs: Vec<String>,
    pub(super) watch_labels: Vec<String>,
    pub(super) rules: crate::forge::rules::Rules,
    /// `[queue.ready] command`: what prints the MRs ready for review.
    pub(super) ready_command: Option<String>,
    pub(super) inline: InlineRule,
    pub(super) open: crate::config::Open,
    /// The checkout `revu` runs in, when its origin is on this forge: `v` opens its real files.
    pub(super) checkout: Option<crate::open::Checkout>,
    /// Jev, when `[ai.typesafe]` is on and a key was found.
    pub(super) jev: Option<TypeSafe>,
    /// Claude, when `[ai.anthropic]` is on and a key was found.
    pub(super) claude: Option<Claude>,
    /// MRs being opened right now: loading ahead waits while there are any.
    pub(super) opening: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    /// The order of the last state save written per MR; also the one lock every write of `state.json` takes.
    pub(super) saved: std::sync::Arc<std::sync::Mutex<HashMap<MrKey, u64>>>,
}

/// What opening an MR fetches: the MR, its diffs, its discussions and my drafts.
pub(super) type Fetched = (Mr, Vec<DiffFile>, Vec<Discussion>, Vec<HeldDraft>);

/// An answer kept in the cache: asking the same thing about the same diff paints it at once, and
/// `a h` lists it. Answers kept before the label was saved have none and are not listed.
#[derive(Serialize, Deserialize)]
struct SavedAnswer {
    text: String,
    model: String,
    usage: Usage,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    asked_at: Option<DateTime<Utc>>,
    #[serde(default)]
    head: Option<Sha>,
    #[serde(default)]
    request: Option<Ask>,
}

impl SavedAnswer {
    fn past(self) -> Option<app::PastAnswer> {
        let outcome = Outcome { stop: Stop::Done, usage: self.usage, model: self.model };
        Some(app::PastAnswer {
            label: self.label?,
            asked_at: self.asked_at?,
            head: self.head?,
            request: self.request?,
            text: self.text,
            outcome,
        })
    }
}

/// How often the cache drops what it keeps for finished MRs.
const PRUNE_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// An MR nobody opened for this long is dropped even when its forge could not say it is finished.
const KEEP_UNTOUCHED: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// One host's cache pruned: the forge asked once per project which kept MRs are finished; a
/// project it cannot answer for keeps its MRs until they are old.
pub(super) async fn prune(forge: &Forge, cache: &Cache) {
    let listed = cache.clone();
    let Ok(kept) = blocking(move || Ok(listed.kept_mrs())).await else { return };
    let mut by_project: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    for mr in &kept {
        by_project.entry(mr.project.clone()).or_default().push(mr.number);
    }
    let mut finished = std::collections::HashSet::new();
    for (project, numbers) in by_project {
        if let Ok(done) = forge.finished(&project, &numbers).await {
            finished.extend(done.into_iter().map(|number| (project.clone(), number)));
        }
    }
    let gone: Vec<(String, u64)> = crate::cache::to_forget(&kept, &finished, std::time::SystemTime::now(), KEEP_UNTOUCHED)
        .into_iter()
        .map(|mr| (mr.project.clone(), mr.number))
        .collect();
    let cache = cache.clone();
    let _ = blocking(move || {
        for (project, number) in &gone {
            let _ = cache.forget_mr(project, *number);
        }
        Ok(())
    })
    .await;
}

/// Counts an MR being opened for as long as it lives, so loading ahead steps aside.
pub(super) struct Opening(std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl Opening {
    pub(super) fn start(count: &std::sync::Arc<std::sync::atomic::AtomicUsize>) -> Self {
        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self(count.clone())
    }
}

impl Drop for Opening {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Backend {
    /// `work` with this backend on tokio's blocking pool: the cache is plain file I/O, and
    /// building a review highlights every hunk.
    pub(super) async fn off<T: Send + 'static>(&self, work: impl FnOnce(&Backend) -> T + Send + 'static) -> Result<T> {
        let me = self.clone();
        tokio::task::spawn_blocking(move || work(&me)).await.context("a background task stopped")
    }

    /// Claude's answer, piece by piece as it streams, or at once from the cache unless `fresh`.
    /// Only a finished answer is kept, so a cut or refused one is asked again next time.
    /// The answers kept for `key` that say what they were, newest first.
    pub(super) fn past_answers(&self, key: &MrKey) -> Vec<app::PastAnswer> {
        let cache = self.cache_of(key);
        let (dir, prefix) = keys::answers(key);
        let mut answers: Vec<app::PastAnswer> =
            cache.keys_in(&dir, prefix).iter().filter_map(|entry| cache.read::<SavedAnswer>(entry)?.past()).collect();
        answers.sort_by_key(|a| std::cmp::Reverse(a.asked_at));
        answers
    }

    #[allow(clippy::too_many_arguments, reason = "the label and head only go into what is kept")]
    pub(super) async fn ask(&self, key: &MrKey, id: u64, request: &Ask, fresh: bool, label: &str, head: &Sha, send: &impl Fn(Incoming)) {
        let part = |part: Part| Incoming::Answer { key: key.clone(), id, part };
        let Some(claude) = &self.claude else {
            send(part(Part::Failed("Claude is off".into())));
            return;
        };
        let cache_key = keys::answer(key, &anthropic::body(claude.model(), request).to_string());
        let (wanted, reading) = (key.clone(), cache_key.clone());
        let saved = self.off(move |b| b.cache_of(&wanted).read::<SavedAnswer>(&reading)).await.ok().flatten();
        if let Some(saved) = saved.filter(|_| !fresh) {
            let outcome = Outcome { stop: Stop::Done, usage: saved.usage, model: saved.model };
            send(part(Part::Done { outcome, cached_text: Some(saved.text) }));
            return;
        }
        let mut text = String::new();
        let streamed = claude
            .stream(request, |event| match event {
                anthropic::Event::Text(more) => {
                    text.push_str(&more);
                    send(part(Part::Text(more)));
                }
                anthropic::Event::Restart => {
                    text.clear();
                    send(part(Part::Restart));
                }
            })
            .await;
        match streamed {
            Ok(outcome) => {
                if outcome.stop == Stop::Done {
                    let saved = SavedAnswer {
                        text,
                        model: outcome.model.clone(),
                        usage: outcome.usage,
                        label: Some(label.to_owned()),
                        asked_at: Some(Utc::now()),
                        head: Some(head.clone()),
                        request: Some(request.clone()),
                    };
                    let wanted = key.clone();
                    let _ = self.off(move |b| b.cache_of(&wanted).write(&cache_key, &saved)).await;
                }
                send(part(Part::Done { outcome, cached_text: None }));
            }
            Err(failure) => send(part(Part::Failed(failure.0))),
        }
    }

    /// Jev's verdict on a queue MR, from the cache while the MR has not moved.
    pub(super) async fn triage(&self, mr: &crate::forge::QueueMr) -> Result<Incoming, Unavailable> {
        let key = mr.key();
        let wanted = key.clone();
        let cached: Option<Verdict> = self.off(move |b| b.cache_of(&wanted).read(&keys::verdict(&wanted))).await.ok().flatten();
        let verdict = if let Some(verdict) = cached.filter(|v| v.fresh_for(mr)) {
            verdict
        } else {
            let jev = self.jev.as_ref().ok_or_else(|| Unavailable("switched off".into()))?;
            let verdict = triage::judge_mr(jev, mr, Utc::now()).await?;
            let (wanted, saved) = (key.clone(), verdict.clone());
            let _ = self.off(move |b| b.cache_of(&wanted).write(&keys::verdict(&wanted), &saved)).await;
            verdict
        };
        Ok(Incoming::Triaged { key, verdict })
    }

    /// Jev's reading of an open MR at `head`, from the cache once read.
    pub(super) async fn read(
        &self,
        key: MrKey,
        head: Sha,
        waits: Option<serde_json::Value>,
        files: Vec<(String, serde_json::Value)>,
    ) -> Result<Incoming, Unavailable> {
        let (wanted, at) = (key.clone(), head.clone());
        let cached = self.off(move |b| b.cache_of(&wanted).read(&keys::reading(&wanted, &at))).await.ok().flatten();
        let reading = if let Some(reading) = cached {
            reading
        } else {
            let jev = self.jev.as_ref().ok_or_else(|| Unavailable("switched off".into()))?;
            let reading = triage::judge_open(jev, waits, files).await?;
            let (wanted, at, saved) = (key.clone(), head.clone(), reading.clone());
            let _ = self.off(move |b| b.cache_of(&wanted).write(&keys::reading(&wanted, &at), &saved)).await;
            reading
        };
        Ok(Incoming::Read { key, head, reading })
    }

    /// Every project's queue also asks the other hosts I am logged in to; one that fails to
    /// answer is left out rather than failing the whole queue. The ready command runs alongside;
    /// when it fails the queue still paints, with its last answer, and the reason comes back apart.
    pub(super) async fn load_queue(&self, scope: Option<String>) -> Result<(Incoming, Option<String>)> {
        let others = async {
            if scope.is_some() {
                return vec![];
            }
            futures_util::future::join_all(self.others.iter().map(crate::ctx::Home::queue))
                .await
                .into_iter()
                .filter_map(Result::ok)
                .collect()
        };
        let (queue, others, said) = tokio::join!(self.forge.queue(scope.as_deref()), others, self.ask_ready());
        let queue = queue?;
        let (wanted, saved) = (scope.clone(), queue.clone());
        let _ = self.off(move |b| b.cache.write_entry(&keys::queue(wanted.as_deref()), &saved)).await;
        let (ready, failure) = match said {
            Ok(Some(output)) => (Some(self.fresh_ready(scope.as_deref(), output, &queue, &others).await), None),
            Ok(None) => (None, None),
            Err(e) => (self.cached_ready(scope.as_deref()), Some(format!("{e:#}"))),
        };
        let answer = self.off(move |b| b.queue_answer(scope, &queue, &others, ready.as_ref(), false)).await?;
        Ok((answer, failure))
    }

    pub(super) fn cached_queue(&self, scope: Option<String>) -> Option<Incoming> {
        let queue: Queue = self.cache.read_entry(&keys::queue(scope.as_deref()))?.value;
        let others: Vec<Queue> = if scope.is_none() { self.others.iter().filter_map(crate::ctx::Home::cached).collect() } else { vec![] };
        let ready = self.cached_ready(scope.as_deref());
        Some(self.queue_answer(scope, &queue, &others, ready.as_ref(), true))
    }

    async fn ask_ready(&self) -> Result<Option<String>> {
        let Some(command) = &self.ready_command else { return Ok(None) };
        crate::ready::output(command).await.map(Some)
    }

    fn cached_ready(&self, scope: Option<&str>) -> Option<ReadySource> {
        self.ready_command.as_ref()?;
        self.cache.read(&keys::ready(scope))
    }

    async fn fresh_ready(&self, scope: Option<&str>, output: String, queue: &Queue, others: &[Queue]) -> ReadySource {
        let queues: Vec<&Queue> = std::iter::once(queue).chain(others).collect();
        let source = crate::ready::resolve(output, &self.forge, &self.others, scope, &queues).await;
        let (wanted, saved) = (scope.map(str::to_owned), source.clone());
        let _ = self.off(move |b| b.cache.write(&keys::ready(wanted.as_deref()), &saved)).await;
        source
    }

    fn queue_answer(&self, scope: Option<String>, queue: &Queue, others: &[Queue], ready: Option<&ReadySource>, cached: bool) -> Incoming {
        let now = Utc::now();
        let parts = std::iter::once(queue).chain(others).map(|q| q.sections_with(&self.watch_labels, &self.rules, now)).collect();
        let sections = Sections::merge(parts);
        let sections = match ready {
            Some(source) => source.apply(sections, self.forge.host(), &self.others, scope.as_deref(), &queue.me, &self.rules),
            None => sections,
        };
        let opened = self.opened_at(&sections);
        Incoming::Queue { scope, me: queue.me.clone(), sections, opened, cached }
    }

    /// The forge an MR lives on: its own host's when the queue merged several.
    /// A picture from the disk cache, else from the forge and then kept; decoded off the loop.
    /// Only a picture that decodes is kept, so a failed one is asked again next time.
    pub(super) async fn image(&self, key: &MrKey, url: &str) -> Option<image::DynamicImage> {
        let (owner, name) = (key.clone(), keys::image(url));
        let cached = self.off(move |b| b.cache_of(&owner).read_bytes(&name)).await.ok().flatten();
        let fresh = cached.is_none();
        let bytes = match cached {
            Some(bytes) => bytes,
            None => self.forge_of(key).image(key, url).await.ok()?,
        };
        let (owner, name) = (key.clone(), keys::image(url));
        self.off(move |b| {
            let picture = images::decode(&bytes)?;
            if fresh {
                let _ = b.cache_of(&owner).write_bytes(&name, &bytes);
            }
            Some(picture)
        })
        .await
        .ok()
        .flatten()
    }

    /// At most once a day, what the cache keeps for merged, closed or long untouched MRs goes:
    /// their diffs, threads and Claude's answers. Safe to run twice; a failure leaves the cache as it is.
    pub(super) async fn prune_daily(&self) {
        let last = blocking(|| Ok(Cache::shared().read_entry::<()>(&keys::pruned()))).await.ok().flatten();
        if last.is_some_and(|entry| entry.age(Utc::now()) < PRUNE_EVERY) {
            return;
        }
        let homes = std::iter::once((&self.forge, &self.cache)).chain(self.others.iter().map(|home| (&home.forge, &home.cache)));
        for (forge, cache) in homes {
            prune(forge, cache).await;
        }
        let _ = blocking(|| Cache::shared().write_entry(&keys::pruned(), &())).await;
    }

    pub(super) fn forge_of(&self, key: &MrKey) -> &Forge {
        self.home_of(key).map_or(&self.forge, |home| &home.forge)
    }

    fn cache_of(&self, key: &MrKey) -> &Cache {
        self.home_of(key).map_or(&self.cache, |home| &home.cache)
    }

    fn home_of(&self, key: &MrKey) -> Option<&crate::ctx::Home> {
        let host = key.host.as_deref()?;
        self.others.iter().find(|home| home.host == host)
    }

    fn opened_at(&self, sections: &Sections) -> HashMap<MrKey, DateTime<Utc>> {
        sections
            .all()
            .filter_map(|mr| {
                let key = mr.key();
                let state: MrState = self.cache_of(&key).read(&keys::state(&key))?;
                Some((key, state.opened_at?))
            })
            .collect()
    }

    fn state(&self, key: &MrKey) -> MrState {
        self.cache_of(key).read(&keys::state(key)).unwrap_or_default()
    }

    pub(super) fn open_cached(&self, key: &MrKey) -> Option<Incoming> {
        let mr: Entry<Mr> = self.cache_of(key).read_entry(&keys::mr(key))?;
        let diffs: Vec<DiffFile> = self.cache_of(key).read(&keys::diffs(key, &mr.value.refs.head))?;
        let discussions: Vec<Discussion> = self.cache_of(key).read(&keys::discussions(key)).unwrap_or_default();
        let drafts: Vec<HeldDraft> = self.cache_of(key).read(&keys::drafts(key)).unwrap_or_default();
        let age = mr.age(Utc::now());
        let review = self.build(key, mr.value, &diffs, discussions, &drafts);
        Some(Incoming::Review { key: key.clone(), review: Box::new(review), cached: Some(age) })
    }

    pub(super) async fn fetch_review(&self, key: MrKey) -> Result<Incoming> {
        let fetched = self.fetch(&key).await?;
        self.reviewed(key, fetched).await
    }

    /// The MR alone while its head is still `head`: no diffs, threads or drafts fetched, nothing highlighted again.
    /// Once the head moved, the review is built again, with the new head's diffs from the cache when loading ahead put them there.
    pub(super) async fn poll_review(&self, key: MrKey, head: &Sha) -> Result<Incoming> {
        let forge = self.forge_of(&key);
        let mr = forge.mr(&key).await?;
        if &mr.refs.head == head {
            let (wanted, kept) = (key.clone(), mr.clone());
            self.off(move |b| {
                let _ = b.cache_of(&wanted).write_entry(&keys::mr(&wanted), &kept);
                b.mark_opened(&wanted);
            })
            .await?;
            return Ok(Incoming::Mr { key, mr: Box::new(mr) });
        }
        let (diffs, discussions, drafts) =
            tokio::try_join!(self.diffs_at(&key, &mr.refs.head), forge.discussions(&key), forge.drafts(&key))?;
        self.reviewed(key, (mr, diffs, discussions, drafts)).await
    }

    /// The diffs of `head`, which never change once pushed: from the cache when it holds them.
    async fn diffs_at(&self, key: &MrKey, head: &Sha) -> Result<Vec<DiffFile>> {
        let (wanted, at) = (key.clone(), head.clone());
        let cached = self.off(move |b| b.cache_of(&wanted).read(&keys::diffs(&wanted, &at))).await.ok().flatten();
        match cached {
            Some(diffs) => Ok(diffs),
            None => self.forge_of(key).diffs(key).await,
        }
    }

    /// What a fetch brought, kept in the cache and built into a review off the loop.
    async fn reviewed(&self, key: MrKey, fetched: Fetched) -> Result<Incoming> {
        self.off(move |b| {
            b.keep(&key, &fetched);
            b.mark_opened(&key);
            let (mr, diffs, discussions, drafts) = fetched;
            let review = b.build(&key, mr, &diffs, discussions, &drafts);
            Incoming::Review { key, review: Box::new(review), cached: None }
        })
        .await
    }

    /// Everything opening an MR needs, from the forge.
    async fn fetch(&self, key: &MrKey) -> Result<Fetched> {
        let forge = self.forge_of(key);
        Ok(tokio::try_join!(forge.mr(key), forge.diffs(key), forge.discussions(key), forge.drafts(key))?)
    }

    /// Writes what a fetch brought where opening reads it; a failed write only costs a later fetch.
    fn keep(&self, key: &MrKey, (mr, diffs, discussions, drafts): &Fetched) {
        let cache = self.cache_of(key);
        let _ = cache.write_entry(&keys::mr(key), mr);
        let _ = cache.write(&keys::diffs(key, &mr.refs.head), diffs);
        let _ = cache.write(&keys::discussions(key), discussions);
        let _ = cache.write(&keys::drafts(key), drafts);
    }

    /// Loads one MR into the cache ahead of time, quietly. It never marks the MR opened, so its
    /// activity dot stays; it steps aside while an MR is being opened or requests run low.
    pub(super) async fn load_ahead(&self, ahead: Ahead) {
        let busy = self.opening.load(std::sync::atomic::Ordering::SeqCst) > 0;
        let low = self.forge_of(&ahead.key).rate().remaining.is_some_and(|left| left < app::prefetch::SPARE);
        let (wanted, seen) = (ahead.key.clone(), ahead.updated_at);
        if busy || low || self.off(move |b| b.is_fresh(&wanted, seen)).await.unwrap_or(true) {
            return;
        }
        if let Ok(fetched) = self.fetch(&ahead.key).await {
            let key = ahead.key;
            let _ = self.off(move |b| b.keep(&key, &fetched)).await;
        }
    }

    /// The cache already holds this MR as the queue last saw it, diff included.
    fn is_fresh(&self, key: &MrKey, seen: DateTime<Utc>) -> bool {
        let cache = self.cache_of(key);
        cache
            .read_entry::<Mr>(&keys::mr(key))
            .is_some_and(|mr| mr.value.updated_at >= seen && cache.has(&keys::diffs(key, &mr.value.refs.head)))
    }

    /// Posts the draft unless the forge already lists it: a retry after a lost answer never doubles a note.
    pub(super) async fn save_draft(&self, key: MrKey, draft: Box<Draft>) -> Result<Incoming> {
        let held = self.forge_of(&key).drafts(&key).await?;
        let saved = match held.into_iter().find(|note| draft.same_as(&Draft::held(note))) {
            Some(note) => note,
            None => self.forge_of(&key).create_draft(&key, &draft.payload()).await?,
        };
        Ok(Incoming::DraftSaved { key, draft, id: saved.id, body: saved.body })
    }

    pub(super) async fn publish(&self, key: MrKey, approve: bool, count: usize) -> Result<Incoming> {
        self.forge_of(&key).publish(&key, approve).await?;
        let wanted = key.clone();
        let _ = self.off(move |b| b.cache_of(&wanted).write(&keys::drafts(&wanted), &Vec::<HeldDraft>::new())).await;
        Ok(Incoming::Published { key, approved: approve, count })
    }

    pub(super) async fn fetch_discussions(&self, key: MrKey) -> Result<Incoming> {
        let discussions = self.forge_of(&key).discussions(&key).await?;
        let (wanted, saved) = (key.clone(), discussions.clone());
        let _ = self.off(move |b| b.cache_of(&wanted).write(&keys::discussions(&wanted), &saved)).await;
        Ok(Incoming::Discussions { key, discussions })
    }

    /// The file ready for the reader's program: the checkout's own when it sits on `sha`,
    /// else a private read-only copy of what the forge serves.
    pub(super) async fn view(&self, key: MrKey, path: &str, sha: &Sha, line: u32, note: Option<String>) -> Result<Incoming> {
        crate::open::safe_path(path)?;
        let checkout = match self.checkout.as_ref().filter(|_| key.host.is_none()) {
            Some(checkout) => checkout.head().await.map(|head| (checkout, head)),
            None => None,
        };
        let source =
            crate::open::Source::pick(&key.project, sha.as_str(), checkout.as_ref().map(|(c, head)| (c.project.as_str(), head.as_str())));
        let (file, dir, note) = if let (crate::open::Source::Checkout, Some((checkout, _))) = (source, checkout) {
            (checkout.root.join(path), None, Some("your checkout · edits are real".to_owned()))
        } else {
            let text = self.file_text(&key, path, sha).await?;
            let name = path.to_owned();
            let (dir, file) = blocking(move || crate::open::write_private(&name, text.as_bytes())).await?;
            (file, Some(std::sync::Arc::new(dir)), note)
        };
        let command = crate::open::command_for(path, &self.open, env("VISUAL").as_deref(), env("EDITOR").as_deref());
        let argv = crate::open::argv(&command, &file.display().to_string(), line, dir.is_some())?;
        let shown = format!("{}:{line}", path.rsplit('/').next().unwrap_or(path));
        Ok(Incoming::ViewReady { key, view: crate::open::View { argv, shown, note, _copy: dir } })
    }

    /// The symbols the files change and the calls between them, a few files read at once.
    pub(super) async fn outline(
        &self,
        key: &MrKey,
        base: &Sha,
        head: &Sha,
        files: Vec<crate::outline::Sides>,
    ) -> Result<crate::outline::Reading> {
        let files: Vec<(String, String, String)> = futures_util::stream::iter(files)
            .map(|sides| async move {
                let path = sides.head.clone().or_else(|| sides.base.clone()).unwrap_or_default();
                let (old, new) = futures_util::try_join!(self.side_text(key, sides.base, base), self.side_text(key, sides.head, head))?;
                Ok::<_, anyhow::Error>((path, old, new))
            })
            .buffered(OUTLINE_READS)
            .try_collect()
            .await?;
        blocking(move || Ok(crate::outline::read(&files))).await
    }

    /// The file at `sha`, empty when it does not exist on that side.
    async fn side_text(&self, key: &MrKey, path: Option<String>, sha: &Sha) -> Result<String> {
        match path {
            Some(path) => self.file_text(key, &path, sha).await,
            None => Ok(String::new()),
        }
    }

    /// A file at a commit never changes, so the cache serves it forever.
    async fn file_text(&self, key: &MrKey, path: &str, sha: &Sha) -> Result<String> {
        let cache_key = keys::file(key, sha, path);
        let (wanted, reading) = (key.clone(), cache_key.clone());
        if let Some(text) = self.off(move |b| b.cache_of(&wanted).read::<String>(&reading)).await.ok().flatten() {
            return Ok(text);
        }
        let short = sha.as_str().get(..8).unwrap_or(sha.as_str());
        let text = self.forge_of(key).file(key, path, sha).await.with_context(|| format!("{path} at {short} · v tries again"))?;
        anyhow::ensure!(
            text.len() <= crate::open::MAX_BYTES,
            "too large to open here ({} MB) · o opens it in the browser",
            text.len() / (1024 * 1024)
        );
        let (wanted, saved) = (key.clone(), text.clone());
        let _ = self.off(move |b| b.cache_of(&wanted).write(&cache_key, &saved)).await;
        Ok(text)
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
            .with_drafts(drafts.iter().map(Draft::held).collect())
    }

    /// Saves what the reader chose unless a newer save of `key` already landed; a `None` spot keeps the place saved before.
    /// Saves run each on its own blocking thread, so they can finish out of order.
    pub(super) fn save_state(&self, key: &MrKey, order: u64, chosen: MrState) -> Result<()> {
        let mut saved = self.saved.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if saved.get(key).is_some_and(|&last| last >= order) {
            return Ok(());
        }
        let before = self.state(key);
        let state = MrState { spot: chosen.spot.or(before.spot), opened_at: before.opened_at, ..chosen };
        self.cache_of(key).write(&keys::state(key), &state)?;
        saved.insert(key.clone(), order);
        Ok(())
    }

    /// Under the lock of the reader's saves, so neither writes over what the other just wrote.
    fn mark_opened(&self, key: &MrKey) {
        let _saving = self.saved.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = MrState { opened_at: Some(Utc::now()), ..self.state(key) };
        let _ = self.cache_of(key).write(&keys::state(key), &state);
    }

    /// Sends where the reader left `key` last time, when a place was saved.
    pub(super) async fn resume(&self, key: &MrKey, send: &impl Fn(Incoming)) {
        let wanted = key.clone();
        if let Ok(Some(spot)) = self.off(move |b| b.state(&wanted).spot).await {
            send(Incoming::Resume { key: key.clone(), spot });
        }
    }

    /// How far I went in each MR of the queue that I started, read from its saved state.
    pub(super) fn progress(&self, sections: &Sections) -> HashMap<MrKey, Progress> {
        sections
            .all()
            .filter_map(|mr| {
                let key = mr.key();
                let state: MrState = self.cache_of(&key).read(&keys::state(&key))?;
                let viewed: BTreeSet<String> = state.viewed_files.into_keys().collect();
                (!viewed.is_empty()).then(|| (key, Progress::new(mr.files as usize, &viewed, &state.auto_folded)))
            })
            .collect()
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

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::ai;
    use crate::forge::gitlab::Client;
    use crate::forge::gitlab::fixture::key;
    use crate::forge::{LineRef, Position, Refs};

    #[test]
    fn kept_answers_list_newest_first_and_skip_those_saved_without_a_label() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend { cache: Cache::in_dir(dir.path()), ..backend_on_nothing() };
        let mr = key();
        let saved = |label: Option<&str>, hours: i64| SavedAnswer {
            text: "text".into(),
            model: "claude-opus-5".into(),
            usage: Usage::default(),
            label: label.map(str::to_owned),
            asked_at: Some(Utc::now() - chrono::Duration::hours(hours)),
            head: Some("b2".into()),
            request: Some(Ask { system: vec![], turns: vec![] }),
        };
        let cache = backend.cache_of(&mr);
        cache.write(&keys::answer(&mr, "old"), &saved(Some("explain · a.rs"), 5)).unwrap();
        cache.write(&keys::answer(&mr, "new"), &saved(Some("summary"), 1)).unwrap();
        cache.write(&keys::answer(&mr, "before"), &serde_json::json!({"text": "t", "model": "m", "usage": Usage::default()})).unwrap();
        let labels: Vec<String> = backend.past_answers(&mr).into_iter().map(|a| a.label).collect();
        assert_eq!(labels, ["summary", "explain · a.rs"]);
    }

    #[tokio::test]
    async fn pruning_drops_the_merged_mrs_keeps_the_open_ones_and_runs_twice_without_harm() {
        use wiremock::matchers::{method, path};
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(method("GET"))
            .and(path("/api/v4/projects/acme%2Fwidgets/merge_requests"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"iid": 40, "state": "merged"}, {"iid": 42, "state": "opened"}
            ])))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::in_dir(dir.path());
        cache.write("mr/acme+widgets/40/ai/answer.1.json", &1).unwrap();
        cache.write("mr/acme+widgets/42/mr.json", &2).unwrap();
        let forge = backend_on(&server).forge;
        prune(&forge, &cache).await;
        prune(&forge, &cache).await;
        let left: Vec<u64> = cache.kept_mrs().into_iter().map(|k| k.number).collect();
        assert_eq!(left, [42]);
    }

    #[tokio::test]
    async fn off_runs_the_work_with_the_backend_and_hands_back_its_result() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend { cache: Cache::in_dir(dir.path()), ..backend_on_nothing() };
        backend.cache.write(&keys::queue(None), &7_u32).unwrap();
        assert_eq!(backend.off(|b| b.cache.read::<u32>(&keys::queue(None))).await.unwrap(), Some(7));
    }

    use crate::diff::fold::Fold;

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

    #[test]
    fn state_round_trips_and_tolerates_an_empty_file() {
        let state: MrState = serde_json::from_str("{}").unwrap();
        assert_eq!(state, MrState::default());
        let before: MrState = serde_json::from_str(r#"{"split": true}"#).unwrap();
        assert!(before.side_by_side, "the split choice saved before reads as side by side");
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::in_dir(dir.path());
        let backend = Backend {
            others: vec![],
            forge: test_forge(),
            cache,
            fold_globs: vec![],
            watch_labels: vec![],
            rules: crate::forge::rules::Rules::off(),
            ready_command: None,
            inline: InlineRule::default(),
            open: crate::config::Open::default(),
            checkout: None,
            jev: None,
            claude: None,
            opening: std::sync::Arc::default(),
            saved: std::sync::Arc::default(),
        };
        let spot = app::Spot { path: "a.rs".into(), old: None, new: Some(3) };
        let viewed = BTreeMap::from([("a.rs".to_owned(), "f1".to_owned())]);
        let chosen = MrState { viewed_files: viewed.clone(), side_by_side: true, ..MrState::default() };
        backend.save_state(&key(), 1, MrState { spot: Some(spot.clone()), ..chosen.clone() }).unwrap();
        backend.save_state(&key(), 2, chosen).unwrap();
        let state = backend.state(&key());
        assert_eq!((state.viewed_files, state.side_by_side), (viewed, true), "the side by side choice is remembered per MR");
        assert_eq!(state.spot, Some(spot), "a save without a place keeps the one saved before");
    }

    #[test]
    fn a_state_save_finishing_after_a_newer_one_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend { cache: Cache::in_dir(dir.path()), ..backend_on_nothing() };
        backend.save_state(&key(), 2, MrState { side_by_side: true, ..MrState::default() }).unwrap();
        backend.save_state(&key(), 1, MrState::default()).unwrap();
        backend.mark_opened(&key());
        backend.save_state(&key(), 2, MrState::default()).unwrap();
        let state = backend.state(&key());
        assert!(state.side_by_side, "the older save and a repeat of the newest one are dropped");
        assert!(state.opened_at.is_some(), "stamping the MR opened keeps what the reader chose");
    }

    #[tokio::test]
    async fn jev_is_asked_once_per_mr_state_and_answers_from_the_cache_after() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend {
            others: vec![],
            forge: test_forge(),
            cache: Cache::in_dir(dir.path()),
            fold_globs: vec![],
            watch_labels: vec![],
            rules: crate::forge::rules::Rules::off(),
            ready_command: None,
            inline: InlineRule::default(),
            open: crate::config::Open::default(),
            checkout: None,
            jev: None,
            claude: None,
            opening: std::sync::Arc::default(),
            saved: std::sync::Arc::default(),
        };
        let mr = crate::forge::gitlab::fixture::queue(include_str!("../forge/gitlab/fixtures/queue.json")).review_requested[0].clone();
        let verdict = Verdict { urgency: 2.8, size: triage::Size::Large, seen: mr.updated_at };
        backend.cache.write(&keys::verdict(&mr.key()), &verdict).unwrap();
        let Ok(Incoming::Triaged { verdict: cached, .. }) = backend.triage(&mr).await else { panic!("the cache answers") };
        assert_eq!(cached, verdict);
        let moved = crate::forge::QueueMr { updated_at: mr.updated_at + chrono::TimeDelta::minutes(5), ..mr };
        assert!(backend.triage(&moved).await.is_err(), "a moved MR needs Jev, which is off here");
        let reading = triage::Reading { waits_on_me: true, risks: BTreeMap::new() };
        backend.cache.write(&keys::reading(&key(), &"abc".into()), &reading).unwrap();
        let Ok(Incoming::Read { reading: read, .. }) = backend.read(key(), "abc".into(), None, vec![]).await else { panic!("cached") };
        assert!(read.waits_on_me);
    }

    #[tokio::test]
    async fn an_answer_streams_once_then_comes_from_the_cache_until_asked_fresh() {
        use wiremock::matchers::method;
        let server = wiremock::MockServer::start().await;
        let stream = [
            r#"{"type":"message_start","message":{"model":"claude-opus-5","usage":{"input_tokens":10,"cache_read_input_tokens":900}}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Looks fine."}}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3}}"#,
        ]
        .iter()
        .map(|data| format!("data: {data}\n\n"))
        .collect::<String>();
        wiremock::Mock::given(method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(stream))
            .expect(2)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend {
            others: vec![],
            forge: test_forge(),
            cache: Cache::in_dir(dir.path()),
            fold_globs: vec![],
            watch_labels: vec![],
            rules: crate::forge::rules::Rules::off(),
            ready_command: None,
            inline: InlineRule::default(),
            open: crate::config::Open::default(),
            checkout: None,
            jev: None,
            claude: Some(Claude::with_base(&server.uri(), ai::Secret::new("sk-ant-test"), "claude-opus-5")),
            opening: std::sync::Arc::default(),
            saved: std::sync::Arc::default(),
        };
        let request = Ask { system: vec![], turns: vec![anthropic::Turn { role: anthropic::Role::User, text: "ok?".into() }] };
        let heard = std::sync::Mutex::new(vec![]);
        let listen = |incoming: Incoming| heard.lock().unwrap().push(incoming);
        backend.ask(&key(), 1, &request, false, "explain", &"b2".into(), &listen).await;
        backend.ask(&key(), 2, &request, false, "explain", &"b2".into(), &listen).await;
        backend.ask(&key(), 3, &request, true, "explain", &"b2".into(), &listen).await;
        let heard = heard.into_inner().unwrap();
        let cached: Vec<_> = heard
            .iter()
            .filter_map(|i| match i {
                Incoming::Answer { id, part: Part::Done { cached_text, .. }, .. } => Some((*id, cached_text.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(cached, vec![(1, None), (2, Some("Looks fine.".into())), (3, None)], "the second answer never reaches the API");
    }

    fn test_forge() -> Forge {
        Forge::GitLab(Client::new(&crate::auth::Credentials { host: "gitlab.com".into(), token: "glpat-xxxx".into() }).unwrap())
    }

    fn held_note(id: u64, note: &str, new_line: u32) -> serde_json::Value {
        serde_json::json!({
            "id": id, "author_id": 1, "merge_request_id": 42, "note": note,
            "position": {"base_sha": "a", "head_sha": "b", "start_sha": "a", "position_type": "text",
                         "old_path": "src/a.rs", "new_path": "src/a.rs", "old_line": null, "new_line": new_line}
        })
    }

    /// One MR on the mock GitLab, changed at `updated`; its five requests (MR, approvals, diffs, discussions,
    /// drafts) may come `times[i]` times.
    async fn mount_mr(server: &wiremock::MockServer, updated: &str, times: [u64; 5]) {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, ResponseTemplate};
        let base = "/api/v4/projects/acme%2Fwidgets/merge_requests/42";
        let mr = serde_json::json!({
            "id": 1042, "iid": 42, "project_id": 7, "title": "feat: charge cards", "description": "", "state": "opened", "draft": false,
            "author": {"id": 5, "username": "omar", "name": "Omar"}, "source_branch": "feat/checkout", "target_branch": "main",
            "web_url": "https://gitlab.com/acme/widgets/-/merge_requests/42", "updated_at": updated, "sha": "bbbb",
            "diff_refs": {"base_sha": "aaaa", "head_sha": "bbbb", "start_sha": "aaaa"}
        });
        let answers = [
            (base.to_owned(), mr),
            (format!("{base}/approvals"), serde_json::json!({"approved": false, "approvals_left": 1, "approved_by": []})),
            (format!("{base}/diffs"), serde_json::json!([{"diff": "@@ -1 +1 @@\n-a\n+b\n", "old_path": "a.rs", "new_path": "a.rs"}])),
            (format!("{base}/discussions"), serde_json::json!([])),
            (format!("{base}/draft_notes"), serde_json::json!([])),
        ];
        for ((route, body), times) in answers.into_iter().zip(times) {
            Mock::given(method("GET"))
                .and(path(route))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .expect(times)
                .mount(server)
                .await;
        }
    }

    fn ahead(updated: &str) -> Ahead {
        Ahead { key: key(), updated_at: updated.parse().unwrap() }
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
    async fn loading_ahead_steps_aside_while_an_mr_opens() {
        let server = wiremock::MockServer::start().await;
        mount_mr(&server, "2026-09-22T09:00:00Z", [0; 5]).await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend { cache: Cache::in_dir(dir.path()), ..backend_on(&server) };
        let _opening = Opening::start(&backend.opening);
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

    fn backend_on(server: &wiremock::MockServer) -> Backend {
        let creds = crate::auth::Credentials { host: "gitlab.com".into(), token: "glpat-xxxx".into() };
        let forge = Forge::GitLab(Client::with_base(&creds, &format!("{}/api/v4/", server.uri())).unwrap());
        let dir = tempfile::tempdir().unwrap();
        Backend {
            others: vec![],
            forge,
            cache: Cache::in_dir(dir.path()),
            fold_globs: vec![],
            watch_labels: vec![],
            rules: crate::forge::rules::Rules::off(),
            ready_command: None,
            inline: InlineRule::default(),
            open: crate::config::Open::default(),
            checkout: None,
            jev: None,
            claude: None,
            opening: std::sync::Arc::default(),
            saved: std::sync::Arc::default(),
        }
    }

    #[test]
    fn an_mr_from_another_host_reaches_that_host_and_its_cache() {
        let dir = tempfile::tempdir().unwrap();
        let github =
            crate::forge::github::Client::new(&crate::auth::Credentials { host: "github.com".into(), token: "ghp_xxxx".into() }).unwrap();
        let other =
            crate::ctx::Home { host: "github.com".into(), forge: Forge::GitHub(github), cache: Cache::in_dir(dir.path().join("gh")) };
        let queue = crate::forge::gitlab::fixture::queue(include_str!("../forge/gitlab/fixtures/queue.json"));
        other.cache.write_entry(&keys::queue(None), &queue.clone().on_host("github.com")).unwrap();
        let backend = Backend { others: vec![other], cache: Cache::in_dir(dir.path().join("gl")), ..backend_on_nothing() };
        backend.cache.write_entry(&keys::queue(None), &queue).unwrap();
        let there = MrKey { host: Some("github.com".into()), ..key() };
        assert_eq!(
            (backend.forge_of(&there).kind(), backend.forge_of(&key()).kind()),
            (crate::forge::Kind::GitHub, crate::forge::Kind::GitLab)
        );
        let Some(Incoming::Queue { sections, .. }) = backend.cached_queue(None) else { panic!("both caches answer") };
        assert_eq!(
            sections.to_review.iter().filter(|mr| mr.host.as_deref() == Some("github.com")).count(),
            queue.sections(&[]).to_review.len()
        );
        assert!(backend.cached_queue(Some("acme/widgets".into())).is_none(), "a scoped queue keeps to its own host");
    }

    #[test]
    fn the_last_ready_answer_paints_ready_from_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend {
            cache: Cache::in_dir(dir.path()),
            ready_command: Some("never-run".into()),
            rules: crate::forge::rules::Rules::default(),
            ..backend_on_nothing()
        };
        let queue = crate::forge::gitlab::fixture::queue_in(include_str!("../forge/gitlab/fixtures/queue_scoped.json"), "acme/widgets");
        backend.cache.write_entry(&keys::queue(Some("acme/widgets")), &queue).unwrap();
        let output = format!("ready: https://{}/acme/widgets/-/merge_requests/51", backend.forge.host());
        backend.cache.write(&keys::ready(Some("acme/widgets")), &ReadySource { output, outside: vec![] }).unwrap();
        let Some(Incoming::Queue { sections, .. }) = backend.cached_queue(Some("acme/widgets".into())) else { panic!("the cache answers") };
        assert_eq!(sections.ready.iter().map(|mr| mr.number).collect::<Vec<_>>(), [51]);
        assert!(sections.open.iter().all(|mr| mr.number != 51));
        let without = Backend { ready_command: None, ..backend };
        let Some(Incoming::Queue { sections, .. }) = without.cached_queue(Some("acme/widgets".into())) else { panic!("the cache answers") };
        assert!(sections.ready.is_empty(), "no command, no Ready, whatever the cache kept");
    }

    fn backend_on_nothing() -> Backend {
        Backend {
            others: vec![],
            forge: test_forge(),
            cache: Cache::in_dir(std::path::Path::new("/nonexistent")),
            fold_globs: vec![],
            watch_labels: vec![],
            rules: crate::forge::rules::Rules::off(),
            ready_command: None,
            inline: InlineRule::default(),
            open: crate::config::Open::default(),
            checkout: None,
            jev: None,
            claude: None,
            opening: std::sync::Arc::default(),
            saved: std::sync::Arc::default(),
        }
    }

    fn draft_at(new_line: u32, body: &str) -> Draft {
        let refs = Refs { base: "a".into(), start: "a".into(), head: "b".into() };
        let line = LineRef { old: None, new: Some(new_line) };
        Draft::on(Position { refs, old_path: "src/a.rs".into(), new_path: "src/a.rs".into(), line, start: None }, body)
    }

    #[tokio::test]
    async fn a_viewed_file_is_fetched_once_copied_read_only_and_handed_to_the_program() {
        use std::os::unix::fs::PermissionsExt;
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/acme%2Fwidgets/repository/files/src%2Fpay%2Fcharge.rs/raw"))
            .and(query_param("ref", "bbbb"))
            .respond_with(ResponseTemplate::new(200).set_body_string("fn charge() {}\n"))
            .expect(1)
            .mount(&server)
            .await;
        let backend = Backend { open: crate::config::Open { default: Some("nvim".into()), files: BTreeMap::new() }, ..backend_on(&server) };
        for _ in 0..2 {
            let Incoming::ViewReady { view, .. } = backend.view(key(), "src/pay/charge.rs", &"bbbb".into(), 12, None).await.unwrap() else {
                panic!("not ready")
            };
            let file = std::path::Path::new(&view.argv[3]);
            assert_eq!(view.argv[..3], ["nvim", "-R", "+12"], "a copy opens read-only");
            assert_eq!(std::fs::read_to_string(file).unwrap(), "fn charge() {}\n");
            assert_eq!(std::fs::metadata(file).unwrap().permissions().mode() & 0o777, 0o400);
            assert_eq!(view.shown, "charge.rs:12");
        }
        let refused = backend.view(key(), "../../etc/passwd", &"bbbb".into(), 1, None).await.unwrap_err();
        assert!(refused.to_string().contains("does not trust"), "{refused}");
    }

    #[tokio::test]
    async fn a_draft_the_forge_already_holds_is_not_posted_twice() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/acme%2Fwidgets/merge_requests/42/draft_notes"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([held_note(5, "nit", 12)])))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v4/projects/acme%2Fwidgets/merge_requests/42/draft_notes"))
            .respond_with(ResponseTemplate::new(201).set_body_json(held_note(6, "other", 13)))
            .expect(1)
            .mount(&server)
            .await;
        let backend = backend_on(&server);
        let same = Box::new(draft_at(12, "nit"));
        assert_eq!(
            backend.save_draft(key(), same.clone()).await.unwrap(),
            Incoming::DraftSaved { key: key(), draft: same, id: 5, body: "nit".into() }
        );
        let fresh = Box::new(draft_at(13, "other"));
        assert_eq!(
            backend.save_draft(key(), fresh.clone()).await.unwrap(),
            Incoming::DraftSaved { key: key(), draft: fresh, id: 6, body: "other".into() }
        );
    }
}
