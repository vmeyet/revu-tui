//! What the palette lists of the other space: the worktrees of the checkout from the forge, the forge's cached queue
//! from the worktrees. Neither reads a token nor asks the network, and one that cannot be read lists nothing.
use crate::auth::{self, Env, SecurityCli};
use crate::blocking;
use crate::cache::Cache;
use crate::config::Config;
use crate::ctx::{ForgeQueue, Launch};
use crate::forge::{Kind, ProjectPath, Queue, local};
use crate::tui::app::{OtherRows, Space};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::OnceCell;

#[derive(Clone)]
pub(in crate::tui) enum OtherSpace {
    /// A forge session in a checkout lists its worktrees, from `git worktree list` alone.
    Worktrees(PathBuf),
    /// A local session lists the queue `revu` would show, as the cache last held it.
    Forge {
        launch: Launch,
        /// Shared by every clone of the backend, so the host and project are found once a session, on the first ask.
        found: Arc<OnceCell<Option<CachedQueue>>>,
    },
}

/// Where the forge's queue sits in the cache, and the forge it lives on.
#[derive(Clone)]
pub(in crate::tui) struct CachedQueue {
    cache: Cache,
    project: Option<ProjectPath>,
    kind: Kind,
}

impl OtherSpace {
    /// The other space of a session on `space`, run in `checkout`: a forge session outside one has none.
    pub(in crate::tui) fn for_session(space: Space, checkout: Option<PathBuf>, launch: &Launch) -> Option<Self> {
        if space == Space::Local {
            return Some(Self::Forge { launch: launch.clone(), found: Arc::default() });
        }
        checkout.map(Self::Worktrees)
    }

    /// The rows found, `None` when the other space cannot be read: the reason is logged, the palette lists this space only.
    pub(in crate::tui) async fn rows(&self) -> Option<OtherRows> {
        let found = match self {
            Self::Worktrees(dir) => local::worktrees(dir).await.map(|queue| Some(rows_of(Kind::Local, &queue))),
            Self::Forge { launch, found } => {
                let cached = found.get_or_init(|| find_cached_queue(launch.clone())).await.clone()?;
                blocking::run(move || Ok(cached.rows())).await
            }
        };
        found.inspect_err(|e| tracing::warn!("listing the other space in the palette: {e:#}")).ok().flatten()
    }
}

/// Where `revu` would read its queue, picked as it picks it but asking the keychain only whether it holds a token.
async fn find_cached_queue(launch: Launch) -> Option<CachedQueue> {
    let found = blocking::run(move || {
        let config = Config::load()?;
        let ForgeQueue { host, kind, project } = launch.forge_queue(&Env::from_process(), &SecurityCli::new(auth::SERVICE), &config)?;
        Ok(CachedQueue { cache: Cache::for_host(&host)?, project, kind })
    });
    found.await.inspect_err(|e| tracing::warn!("finding the forge's queue for the palette: {e:#}")).ok()
}

impl CachedQueue {
    /// `None` when the cache holds no queue there.
    fn rows(&self) -> Option<OtherRows> {
        Queue::cached(&self.cache, self.project.as_ref()).map(|queue| rows_of(self.kind, &queue))
    }
}

fn rows_of(kind: Kind, queue: &Queue) -> OtherRows {
    OtherRows { kind, me: queue.me.clone(), mrs: queue.rows() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_forge_session_outside_a_checkout_has_no_other_space() {
        let launch = Launch { host: None, all: false, target: None };
        assert!(OtherSpace::for_session(Space::Forge, None, &launch).is_none());
    }

    #[tokio::test]
    async fn the_worktrees_of_the_checkout_come_back_as_local_rows() {
        let repo = local::tests::Repo::new();
        let rows = OtherSpace::Worktrees(repo.path()).rows().await.unwrap();
        assert_eq!(rows.kind, Kind::Local);
        assert!(!rows.mrs.is_empty() && rows.mrs.iter().all(|mr| mr.key().is_local()), "{rows:?}");
    }

    #[tokio::test]
    async fn a_folder_that_stopped_being_a_checkout_lists_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(OtherSpace::Worktrees(dir.path().to_path_buf()).rows().await, None);
    }

    fn cached_in(dir: &std::path::Path, project: Option<&str>) -> CachedQueue {
        CachedQueue { cache: Cache::in_dir(dir), project: project.and_then(ProjectPath::parse), kind: Kind::GitHub }
    }

    #[test]
    fn a_forge_never_cached_lists_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(cached_in(dir.path(), None).rows(), None);
    }

    #[test]
    fn the_cached_queue_of_the_project_comes_back_on_its_forge_with_its_identity() {
        let dir = tempfile::tempdir().unwrap();
        let cached = cached_in(dir.path(), Some("acme/widgets"));
        let queue = crate::forge::gitlab::fixture::queue(include_str!("../../forge/gitlab/fixtures/queue.json"));
        cached.cache.write_entry(&crate::cache::keys::queue(cached.project.as_ref()), &queue).unwrap();
        assert_eq!(cached.rows(), Some(OtherRows { kind: Kind::GitHub, me: queue.me.clone(), mrs: queue.rows() }));
        assert_eq!(cached_in(dir.path(), None).rows(), None, "another scope's queue is not this one");
    }
}
