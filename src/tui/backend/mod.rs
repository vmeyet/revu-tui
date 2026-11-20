//! The service behind the loop: the forge, the cache and the AI, answering actions off the event loop.
//! Each part holds what one concern needs; `Backend` puts them together for the action runner and opens agent sessions.
mod ai;
mod drafts;
mod files;
mod other_space;
mod prune;
mod queue;
mod reviews;
mod saves;

pub(super) use other_space::OtherSpace;
pub(super) use saves::MrState;

use super::app::Incoming;
use super::lanes::Lanes;
use crate::ai::typesafe::TypeSafe;
use crate::auth::HostOrigin;
use crate::blocking;
use crate::cache::Cache;
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
    pub(super) open: crate::config::Open,
    pub(super) checkout: Option<crate::open::Checkout>,
    pub(super) jev: Option<TypeSafe>,
    pub(super) agent: Option<crate::agent::Agent>,
    pub(super) host_origin: HostOrigin,
    pub(super) notifier: crate::notify::Notifier,
    pub(super) other_space: Option<OtherSpace>,
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
    /// The agent whose sessions `a` opens, when `[agent]` is on.
    pub(super) agent: Option<crate::agent::Agent>,
    /// How the host revu started with was chosen.
    host_origin: HostOrigin,
    /// What shows the notification for an MR to review, and brings revu back on a click.
    pub(super) notifier: crate::notify::Notifier,
    /// Where the palette finds the MRs of the other space.
    pub(super) other_space: Option<OtherSpace>,
    /// Two opens of one MR would each find no session and start one; shared by every clone so they wait.
    session_lanes: Lanes,
}

impl Backend {
    pub(super) fn new(homes: Homes, setup: Setup) -> Self {
        let homes = Arc::new(homes);
        let saves = Saves::new(Arc::clone(&homes));
        Self {
            queues: Queues {
                homes: Arc::clone(&homes),
                saves: saves.clone(),
                watch_labels: setup.watch_labels,
                rules: setup.rules,
                ready_command: setup.ready_command,
            },
            reviews: Reviews { homes: Arc::clone(&homes), saves: saves.clone(), fold_globs: setup.fold_globs, opening: Arc::default() },
            drafts: Drafts { homes: Arc::clone(&homes), lanes: Lanes::default() },
            ai: Ai { homes: Arc::clone(&homes), jev: setup.jev },
            files: Files { homes: Arc::clone(&homes), open: setup.open, checkout: setup.checkout },
            agent: setup.agent,
            host_origin: setup.host_origin,
            notifier: setup.notifier,
            other_space: setup.other_space,
            session_lanes: Lanes::default(),
            saves,
            homes,
        }
    }

    /// Why `key` did not open, in the words a command says it. An MR of another host is on the host its row
    /// names, so it gets no hint to ask another.
    pub(super) fn with_host_hint(&self, key: &MrKey, err: anyhow::Error) -> anyhow::Error {
        let origin = if self.homes.home_of(key).is_some() { HostOrigin::Named } else { self.host_origin };
        crate::ctx::with_host_hint(origin, err)
    }

    /// The agent session of `subject`, found or started, then on show in tmux or ready for the terminal.
    pub(super) async fn session(&self, subject: crate::agent::Subject, choice: crate::agent::Choice) -> Result<Incoming> {
        use crate::agent::Outcome;
        let agent = self.agent.clone().context("agent sessions are off")?;
        let cache = self.homes.cache_of(subject.key()).clone();
        let _lane = self.session_lanes.enter(subject.key()).await;
        blocking::run(move || {
            Ok(match agent.open(&subject, &cache, choice)? {
                Outcome::Pick(sessions) => Incoming::SessionPick { subject: Box::new(subject), sessions },
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
    use super::*;
    use crate::cache::keys;
    use crate::tui::app::{Action, Failure};
    use test_support::{homes_in, homes_on};

    fn backend(homes: Homes, host_origin: HostOrigin) -> Backend {
        let setup = Setup {
            fold_globs: vec![],
            watch_labels: vec![],
            rules: crate::forge::rules::Rules::default(),
            ready_command: None,
            open: crate::config::Open::default(),
            checkout: None,
            jev: None,
            agent: None,
            host_origin,
            notifier: crate::notify::Notifier::default(),
            other_space: None,
        };
        Backend::new(homes, setup)
    }

    /// What the app is told when it opens `key` on `backend`.
    async fn open_failure(backend: &Backend, key: MrKey) -> String {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        crate::tui::actions::spawn(Action::Open(key.clone()), backend, tx);
        match rx.recv().await {
            Some(Incoming::Failed { what: Failure::Open { key: failed }, message }) if failed == key => message,
            other => panic!("the open should fail: {other:?}"),
        }
    }

    #[tokio::test]
    async fn opening_an_mr_the_host_of_the_checkout_does_not_have_says_the_host_and_suggests_host() {
        let (server, dir) = (wiremock::MockServer::start().await, tempfile::tempdir().unwrap());
        let backend = backend(homes_on(&server, dir.path()), HostOrigin::Checkout);
        assert_eq!(
            open_failure(&backend, MrKey::new("acme/widgets", 42)).await,
            "acme/widgets!42 was not found on 127.0.0.1 (the host of this checkout): pass --host to ask another forge"
        );
    }

    #[tokio::test]
    async fn opening_an_mr_another_host_does_not_have_does_not_suggest_host() {
        let (server, dir) = (wiremock::MockServer::start().await, tempfile::tempdir().unwrap());
        let Homes { forge, cache, .. } = homes_on(&server, &dir.path().join("other"));
        let other = crate::ctx::Home { host: "gl.acme.test".into(), forge, cache };
        let backend = backend(Homes { others: vec![other], ..homes_in(&dir.path().join("main")) }, HostOrigin::Checkout);
        let key = MrKey { host: Some("gl.acme.test".into()), ..MrKey::new("acme/widgets", 42) };
        assert_eq!(open_failure(&backend, key).await, "acme/widgets!42 was not found on 127.0.0.1, or this token cannot see it");
    }

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
