//! The service behind the loop: the forge, the cache and the AI, answering actions off the event loop.
//! Each part holds what one concern needs; `Backend` puts them together for the action runner and opens agent sessions.
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
use ai::Ai;
use anyhow::{Context as _, Result};
use drafts::Drafts;
use files::Files;
use queue::Queues;
use reviews::Reviews;
use saves::Saves;
use std::sync::Arc;

/// The forge and the cache of the host revu runs against, and of every other host I am logged in to.
/// Every part holds the same one behind an `Arc`: it is read only, and a part is cloned for each action.
#[derive(Clone)]
pub(super) struct Homes {
    pub(super) forge: Forge,
    pub(super) cache: Cache,
    /// The other hosts I am logged in to: every project's queue asks them too.
    pub(super) others: Vec<crate::ctx::Home>,
}

impl Homes {
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
}

/// What startup read from the config and found on the machine.
pub(super) struct Setup {
    pub(super) fold_globs: Vec<String>,
    pub(super) watch_labels: Vec<String>,
    pub(super) rules: crate::forge::rules::Rules,
    pub(super) ready_command: Option<String>,
    pub(super) inline: InlineRule,
    pub(super) open: crate::config::Open,
    pub(super) checkout: Option<crate::open::Checkout>,
    pub(super) jev: Option<TypeSafe>,
    pub(super) claude: Option<Claude>,
    pub(super) agent: Option<crate::agent::Agent>,
}

#[derive(Clone)]
pub(super) struct Backend {
    pub(super) homes: Arc<Homes>,
    pub(super) saves: Saves,
    pub(super) queues: Queues,
    pub(super) reviews: Reviews,
    pub(super) drafts: Drafts,
    pub(super) ai: Ai,
    pub(super) files: Files,
    /// The agent whose sessions `a g` opens, when `[agent]` is on.
    pub(super) agent: Option<crate::agent::Agent>,
}

impl Backend {
    pub(super) fn new(homes: Homes, setup: Setup) -> Self {
        let homes = Arc::new(homes);
        let saves = Saves::new(homes.clone());
        Self {
            queues: Queues {
                homes: homes.clone(),
                saves: saves.clone(),
                watch_labels: setup.watch_labels,
                rules: setup.rules,
                ready_command: setup.ready_command,
            },
            reviews: Reviews {
                homes: homes.clone(),
                saves: saves.clone(),
                fold_globs: setup.fold_globs,
                inline: setup.inline,
                opening: Arc::default(),
            },
            drafts: Drafts { homes: homes.clone(), lanes: Lanes::default() },
            ai: Ai { homes: homes.clone(), jev: setup.jev, claude: setup.claude },
            files: Files { homes: homes.clone(), open: setup.open, checkout: setup.checkout },
            agent: setup.agent,
            saves,
            homes,
        }
    }

    /// The agent session of `mr`, found or started, then on show in tmux or ready for the terminal.
    pub(super) async fn session(&self, mr: crate::agent::Mr, choice: crate::agent::Choice) -> Result<Incoming> {
        use crate::agent::Outcome;
        let agent = self.agent.clone().context("agent sessions are off")?;
        let cache = self.homes.cache_of(&mr.key).clone();
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

/// `work` with its own copy of `part` on tokio's blocking pool: the cache is plain file I/O, and
/// building a review highlights every hunk.
pub(super) async fn run_blocking<P, T>(part: &P, work: impl FnOnce(&P) -> T + Send + 'static) -> Result<T>
where
    P: Clone + Send + 'static,
    T: Send + 'static,
{
    let part = part.clone();
    blocking::run(move || Ok(work(&part))).await
}

/// `run_blocking` for a cache read: a task that stopped is logged and reads as a miss.
pub(super) async fn read_cache<P, T>(part: &P, work: impl FnOnce(&P) -> T + Send + 'static) -> T
where
    P: Clone + Send + 'static,
    T: Default + Send + 'static,
{
    run_blocking(part, work).await.inspect_err(warn_stopped).unwrap_or_default()
}

/// A cache write on the blocking pool whose failure only costs a later fetch: logged as `what`, never handed back.
async fn write_cache<P>(what: &str, part: &P, work: impl FnOnce(&P) -> Result<()> + Send + 'static)
where
    P: Clone + Send + 'static,
{
    run_blocking(part, work).await.and_then(|written| written).or_warn(what);
}

fn warn_stopped(error: &anyhow::Error) {
    tracing::warn!("{error:#}");
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
    use test_support::homes_in;

    #[tokio::test]
    async fn run_blocking_runs_the_work_with_the_part_and_hands_back_its_result() {
        let dir = tempfile::tempdir().unwrap();
        let homes = homes_in(dir.path());
        homes.cache.write(&keys::queue(None), &7_u32).unwrap();
        assert_eq!(run_blocking(&homes, |homes| homes.cache.read::<u32>(&keys::queue(None))).await.unwrap(), Some(7));
    }

    #[tokio::test]
    async fn a_cache_write_that_fails_is_logged_with_where_it_went() {
        crate::log::capture::start();
        let dir = tempfile::tempdir().unwrap();
        let taken = dir.path().join("taken");
        std::fs::write(&taken, "a file where the cache folder goes").unwrap();
        write_cache("caching the queue", &homes_in(&taken), |homes| homes.cache.write(&keys::queue(None), &7_u32)).await;
        let text = crate::log::capture::text();
        assert!(text.contains("WARN") && text.contains("caching the queue failed: writing ") && text.contains("taken/"), "{text}");
    }
}
