//! The service behind the loop: the forge, the cache and the AI, answering actions off the event loop.
mod ai;
mod drafts;
mod files;
mod prune;
mod queue;
mod reviews;
mod saves;

pub(super) use saves::MrState;

use super::app::Incoming;
use super::lanes::Lanes;
use crate::ai::anthropic::Claude;
use crate::ai::typesafe::TypeSafe;
use crate::blocking;
use crate::cache::Cache;
use crate::diff::words::InlineRule;
use crate::forge::{Forge, MrKey};
use crate::log::OrWarn as _;
use anyhow::{Context as _, Result};
use std::collections::HashMap;

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
    /// The agent whose sessions `a g` opens, when `[agent]` is on.
    pub(super) agent: Option<crate::agent::Agent>,
    /// MRs being opened right now: loading ahead waits while there are any.
    pub(super) opening: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    /// The order of the last state save written per MR; also the one lock every write of `state.json` takes.
    pub(super) saved: std::sync::Arc<std::sync::Mutex<HashMap<MrKey, u64>>>,
    /// The order of the last change written per cache entry and target: pins and the queue view.
    pub(super) changed: std::sync::Arc<std::sync::Mutex<HashMap<String, u64>>>,
    /// Draft writes and publishing of one MR wait for each other: on GitHub each reads the pending review, then writes it.
    pub(super) draft_lanes: Lanes,
}

fn warn_stopped(error: &anyhow::Error) {
    tracing::warn!("{error:#}");
}

impl Backend {
    /// `work` with this backend on tokio's blocking pool: the cache is plain file I/O, and
    /// building a review highlights every hunk.
    pub(super) async fn run_blocking<T: Send + 'static>(&self, work: impl FnOnce(&Backend) -> T + Send + 'static) -> Result<T> {
        let me = self.clone();
        blocking::run(move || Ok(work(&me))).await
    }

    /// `run_blocking` for a cache read: a task that stopped is logged and reads as a miss.
    pub(super) async fn read_cache<T: Default + Send + 'static>(&self, work: impl FnOnce(&Backend) -> T + Send + 'static) -> T {
        self.run_blocking(work).await.inspect_err(warn_stopped).unwrap_or_default()
    }

    /// A cache write on the blocking pool whose failure only costs a later fetch: logged as `what`, never handed back.
    async fn write_cache(&self, what: &str, work: impl FnOnce(&Backend) -> Result<()> + Send + 'static) {
        let me = self.clone();
        blocking::run(move || work(&me)).await.or_warn(what);
    }

    /// The forge an MR lives on: its own host's when the queue merged several.
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

    /// The agent session of `mr`, found or started, then on show in tmux or ready for the terminal.
    pub(super) async fn session(&self, mr: crate::agent::Mr, choice: crate::agent::Choice) -> Result<Incoming> {
        use crate::agent::Outcome;
        let agent = self.agent.clone().context("agent sessions are off")?;
        let cache = self.cache_of(&mr.key).clone();
        blocking::run(move || {
            Ok(match agent.open(&mr, &cache, choice)? {
                Outcome::Pick(sessions) => Incoming::SessionPick { mr: Box::new(mr), sessions },
                Outcome::Shown(shown) => Incoming::Done(shown),
                Outcome::Handoff(handoff) => Incoming::SessionReady(handoff),
            })
        })
        .await
    }
}

#[cfg(test)]
mod cache_shapes;
#[cfg(test)]
mod test_support;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::cache::keys;
    use test_support::backend_on_nothing;

    #[tokio::test]
    async fn run_blocking_runs_the_work_with_the_backend_and_hands_back_its_result() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend { cache: Cache::in_dir(dir.path()), ..backend_on_nothing() };
        backend.cache.write(&keys::queue(None), &7_u32).unwrap();
        assert_eq!(backend.run_blocking(|b| b.cache.read::<u32>(&keys::queue(None))).await.unwrap(), Some(7));
    }

    #[tokio::test]
    async fn a_cache_write_that_fails_is_logged_with_where_it_went() {
        crate::log::capture::start();
        let dir = tempfile::tempdir().unwrap();
        let taken = dir.path().join("taken");
        std::fs::write(&taken, "a file where the cache folder goes").unwrap();
        let backend = Backend { cache: Cache::in_dir(&taken), ..backend_on_nothing() };
        backend.write_cache("caching the queue", |b| b.cache.write(&keys::queue(None), &7_u32)).await;
        let text = crate::log::capture::text();
        assert!(text.contains("WARN") && text.contains("caching the queue failed: writing ") && text.contains("taken/"), "{text}");
    }
}
