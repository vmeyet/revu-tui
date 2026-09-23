//! The seam between the app and the forge that hosts the code: one neutral model, one enum that
//! sends each call to the backend of the host.
mod budget;
pub mod checks;
pub mod github;
pub mod gitlab;
pub mod image;
mod model;
mod queue;
pub mod rules;

pub use budget::RateLimit;
pub use model::{
    Applicable, Approvals, DiffFile, Discussion, Draft, LineRef, Mr, MrKey, NewDraft, Note, Pipeline, Position, Refs, Side, Suggestion,
    User,
};
pub use queue::{Queue, QueueMr, ReviewState, ReviewerState, Sections};

use crate::auth::Credentials;
use crate::config::Config;
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Which forge a host runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    GitLab,
    GitHub,
}

impl Kind {
    /// `github.com` is GitHub, anything else GitLab, unless `[hosts."<host>"] forge` says otherwise.
    pub fn for_host(host: &str, config: &Config) -> Self {
        let named = config.hosts.get(host).and_then(|h| h.forge);
        named.unwrap_or(if host == "github.com" { Kind::GitHub } else { Kind::GitLab })
    }

    /// What goes between a project and a number: `group/project!42`, `owner/repo#42`.
    pub fn sigil(self) -> char {
        match self {
            Kind::GitLab => '!',
            Kind::GitHub => '#',
        }
    }

    /// The web page of one diff line: the MR's page, scrolled to the line.
    pub fn line_url(self, web_url: &str, path: &str, line: LineRef) -> String {
        match self {
            Kind::GitLab => gitlab::line_url(web_url, path, line),
            Kind::GitHub => github::line_url(web_url, path, line),
        }
    }
}

/// A connected forge. Every method takes and returns the neutral model.
/// The hosts a merged queue draws from: the one `revu` started with, and the others. Tells which
/// forge an MR's host runs, and the short tag its row carries once there is more than one host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hosts {
    pub main: (String, Kind),
    pub others: Vec<(String, Kind)>,
}

impl Hosts {
    #[cfg(test)]
    pub fn one(host: &str, kind: Kind) -> Self {
        Self { main: (host.to_owned(), kind), others: vec![] }
    }

    pub fn kind_of(&self, key: &MrKey) -> Kind {
        key.host.as_deref().and_then(|h| self.others.iter().find(|(host, _)| host == h)).map_or(self.main.1, |(_, kind)| *kind)
    }

    /// `gitlab`, `github`: the first name of the host `key` lives on. Callers show it only when
    /// the rows on screen come from several hosts (`Sections::mixes_hosts`).
    pub fn tag(&self, key: &MrKey) -> Option<String> {
        let host = key.host.as_deref().unwrap_or(&self.main.0);
        host.split('.').next().map(str::to_owned)
    }
}

#[derive(Clone, Debug)]
pub enum Forge {
    GitLab(gitlab::Client),
    GitHub(github::Client),
}

impl Forge {
    /// The backend for `kind`, speaking to `credentials.host` only.
    pub fn connect(kind: Kind, credentials: &Credentials) -> Result<Self> {
        match kind {
            Kind::GitLab => Ok(Forge::GitLab(gitlab::Client::new(credentials)?)),
            Kind::GitHub => Ok(Forge::GitHub(github::Client::new(credentials)?)),
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            Forge::GitLab(_) => Kind::GitLab,
            Forge::GitHub(_) => Kind::GitHub,
        }
    }

    pub fn host(&self) -> &str {
        match self {
            Forge::GitLab(client) => client.host(),
            Forge::GitHub(client) => client.host(),
        }
    }

    pub async fn me(&self) -> Result<User> {
        match self {
            Forge::GitLab(client) => client.me().await,
            Forge::GitHub(client) => client.me().await,
        }
    }

    /// Every MR waiting on me; with `project`, only that project's, plus all its other open MRs.
    pub async fn queue(&self, project: Option<&str>) -> Result<Queue> {
        match self {
            Forge::GitLab(client) => client.queue(project).await,
            Forge::GitHub(client) => client.queue(project).await,
        }
    }

    /// The path of a project the user named by its numeric id (`123!42`).
    pub async fn project_path(&self, id: u64) -> Result<String> {
        match self {
            Forge::GitLab(client) => client.project_path(id).await,
            Forge::GitHub(client) => client.project_path(id).await,
        }
    }

    /// The open MR whose source is `branch`, if any.
    pub async fn mr_for_branch(&self, project: &str, branch: &str) -> Result<Option<u64>> {
        match self {
            Forge::GitLab(client) => client.mr_for_branch(project, branch).await,
            Forge::GitHub(client) => client.mr_for_branch(project, branch).await,
        }
    }

    pub async fn mr(&self, key: &MrKey) -> Result<Mr> {
        match self {
            Forge::GitLab(client) => client.mr(key).await,
            Forge::GitHub(client) => client.mr(key).await,
        }
    }

    /// Requests left and any rate-limit wait, as the last answers said.
    pub fn rate(&self) -> RateLimit {
        match self {
            Forge::GitLab(client) => client.rate(),
            Forge::GitHub(client) => client.rate(),
        }
    }

    /// The whole file `path` at commit `sha`, for the context around a hunk.
    pub async fn file(&self, key: &MrKey, path: &str, sha: &str) -> Result<String> {
        match self {
            Forge::GitLab(client) => client.file(&key.project, path, sha).await,
            Forge::GitHub(client) => client.file(&key.project, path, sha).await,
        }
    }

    /// The bytes of a picture a note of `key` points at, `url` as the note wrote it. Links off
    /// the forge are refused, and a slow one gives up after [`image::TIMEOUT`].
    pub async fn image(&self, key: &MrKey, url: &str) -> Result<Vec<u8>> {
        let fetch = async {
            match self {
                Forge::GitLab(client) => client.image(&key.project, url).await,
                Forge::GitHub(client) => client.image(url).await,
            }
        };
        tokio::time::timeout(image::TIMEOUT, fetch).await.map_err(|_| anyhow::anyhow!("timed out"))?
    }

    /// Commits `suggestion` on the MR's source branch `branch`.
    pub async fn apply(&self, key: &MrKey, branch: &str, suggestion: &Suggestion) -> Result<()> {
        match self {
            Forge::GitLab(client) => client.apply(suggestion).await,
            Forge::GitHub(client) => client.apply(key, branch, suggestion).await,
        }
    }

    /// The CI run of the MR's head commit `head`; `None` when nothing ran on it.
    pub async fn checks(&self, key: &MrKey, head: &str) -> Result<Option<checks::Checks>> {
        match self {
            Forge::GitLab(client) => client.checks(key).await,
            Forge::GitHub(client) => client.checks(key, head).await,
        }
    }

    pub async fn diffs(&self, key: &MrKey) -> Result<Vec<DiffFile>> {
        match self {
            Forge::GitLab(client) => client.diffs(key).await,
            Forge::GitHub(client) => client.diffs(key).await,
        }
    }

    pub async fn discussions(&self, key: &MrKey) -> Result<Vec<Discussion>> {
        match self {
            Forge::GitLab(client) => client.discussions(key).await,
            Forge::GitHub(client) => client.discussions(key).await,
        }
    }

    /// My unpublished notes on the MR.
    pub async fn drafts(&self, key: &MrKey) -> Result<Vec<Draft>> {
        match self {
            Forge::GitLab(client) => client.drafts(key).await,
            Forge::GitHub(client) => client.drafts(key).await,
        }
    }

    pub async fn create_draft(&self, key: &MrKey, draft: &NewDraft) -> Result<Draft> {
        match self {
            Forge::GitLab(client) => client.create_draft(key, draft).await,
            Forge::GitHub(client) => client.create_draft(key, draft).await,
        }
    }

    /// Replaces the whole draft, position included.
    pub async fn update_draft(&self, key: &MrKey, id: u64, draft: &NewDraft) -> Result<Draft> {
        match self {
            Forge::GitLab(client) => client.update_draft(key, id, draft).await,
            Forge::GitHub(client) => client.update_draft(key, id, draft).await,
        }
    }

    pub async fn delete_draft(&self, key: &MrKey, id: u64) -> Result<()> {
        match self {
            Forge::GitLab(client) => client.delete_draft(key, id).await,
            Forge::GitHub(client) => client.delete_draft(key, id).await,
        }
    }

    /// Every draft of mine goes public as one review, approving the MR with it when asked.
    pub async fn publish(&self, key: &MrKey, approve: bool) -> Result<()> {
        match self {
            Forge::GitLab(client) => client.publish(key, approve).await,
            Forge::GitHub(client) => client.publish(key, approve).await,
        }
    }

    pub async fn resolve(&self, key: &MrKey, discussion: &str, resolved: bool) -> Result<()> {
        match self {
            Forge::GitLab(client) => client.resolve(key, discussion, resolved).await,
            Forge::GitHub(client) => client.resolve(discussion, resolved).await,
        }
    }

    /// Approves the MR, or takes my approval back.
    pub async fn approve(&self, key: &MrKey, approve: bool) -> Result<()> {
        match self {
            Forge::GitLab(client) => client.approve(key, approve).await,
            Forge::GitHub(client) => client.approve(key, approve).await,
        }
    }

    /// A public new thread, on a line when `position` is given.
    pub async fn comment(&self, key: &MrKey, body: &str, position: Option<&Position>) -> Result<Discussion> {
        match self {
            Forge::GitLab(client) => client.comment(key, body, position).await,
            Forge::GitHub(client) => client.comment(key, body, position).await,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::config::Host;

    #[test]
    fn the_host_names_the_forge_unless_the_config_says_otherwise() {
        let config = Config::default();
        assert_eq!(Kind::for_host("github.com", &config), Kind::GitHub);
        assert_eq!(Kind::for_host("gitlab.com", &config), Kind::GitLab);
        assert_eq!(Kind::for_host("git.acme.dev", &config), Kind::GitLab);
        let config =
            Config { hosts: [("git.acme.dev".into(), Host { forge: Some(Kind::GitHub), ..Host::default() })].into(), ..Config::default() };
        assert_eq!(Kind::for_host("git.acme.dev", &config), Kind::GitHub);
    }

    #[test]
    fn each_kind_connects_its_own_backend() {
        let credentials = Credentials { host: "github.com".into(), token: "ghp_xxxx".into() };
        assert_eq!(Forge::connect(Kind::GitHub, &credentials).unwrap().kind(), Kind::GitHub);
        assert_eq!(Forge::connect(Kind::GitLab, &Credentials { host: "gitlab.com".into(), ..credentials }).unwrap().kind(), Kind::GitLab);
    }

    #[test]
    fn sigils_follow_the_forge() {
        assert_eq!((Kind::GitLab.sigil(), Kind::GitHub.sigil()), ('!', '#'));
    }

    #[test]
    fn hosts_say_which_forge_an_mr_lives_on_and_name_its_host() {
        let alone = Hosts::one("gitlab.com", Kind::GitLab);
        let key = MrKey::new("acme/widgets", 42);
        assert_eq!((alone.kind_of(&key), alone.tag(&key).as_deref()), (Kind::GitLab, Some("gitlab")));
        let both = Hosts { others: vec![("github.com".into(), Kind::GitHub)], ..alone };
        let there = MrKey { host: Some("github.com".into()), ..key.clone() };
        assert_eq!((both.kind_of(&there), both.tag(&there).as_deref()), (Kind::GitHub, Some("github")));
        assert_eq!((both.kind_of(&key), both.tag(&key).as_deref()), (Kind::GitLab, Some("gitlab")));
    }
}
