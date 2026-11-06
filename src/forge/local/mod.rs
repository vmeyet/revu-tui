//! The local backend: the branch of a worktree read with `git`, with no host and no token. It answers in the neutral
//! model like a forge, a worktree standing for an MR; what only a forge can do fails.
mod git;
mod split;
#[cfg(test)]
pub(crate) mod tests;

use super::{Approvals, DiffFile, Fetched, MergePlan, Mr, MrKey, MrState, Queue, Refs, Sha, User};
use crate::open::safe_path;
use anyhow::{Context, Result, bail};
use git::Worktree;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};

/// What `MrKey.host` holds for a worktree, and the name of the cache folder.
pub(crate) const HOST: &str = "local";
/// What a diff compares against when nothing names a target.
const DEFAULT_TARGETS: [&str; 3] = ["origin/HEAD", "main", "master"];

/// What a diff compares against, as the user named it; never reads as a flag.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Target(String);

impl Target {
    fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        if text.is_empty() || text.starts_with('-') {
            bail!("`{text}` is not a branch or a commit");
        }
        Ok(Self(text.to_owned()))
    }
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What the reader chose for every worktree, for this session.
#[derive(Clone, Debug)]
struct Choice {
    target: Target,
    uncommitted: bool,
}

/// One repository's worktrees, found from the one revu ran in.
#[derive(Clone, Debug)]
pub(crate) struct Client {
    here: PathBuf,
    /// Shared by every clone, since `Forge` methods take only a key; only `Action::ChangeLocal` writes it.
    choice: Arc<RwLock<Choice>>,
}

impl Client {
    /// The repository `dir` is in, comparing against `target`, else the first of `DEFAULT_TARGETS` that exists.
    pub(crate) async fn open(dir: &Path, target: Option<&str>) -> Result<Self> {
        let here = git::toplevel(dir).await?;
        let target = match target {
            Some(text) => checked(&here, text).await?,
            None => default_target(&here).await?,
        };
        Ok(Self { here, choice: Arc::new(RwLock::new(Choice { target, uncommitted: false })) })
    }

    /// The key of the worktree revu ran in.
    pub(crate) async fn here_key(&self) -> Result<MrKey> {
        let listing = git::listing(&self.here).await?;
        let here =
            listing.worktrees.iter().find(|worktree| worktree.path == self.here).context("this worktree is not in `git worktree list`")?;
        key_of(here)
    }

    /// From now on, diffs compare against `target`; one that names no commit changes nothing.
    pub(crate) async fn set_target(&self, target: &str) -> Result<()> {
        let target = checked(&self.here, target).await?;
        self.choice.write().unwrap_or_else(PoisonError::into_inner).target = target;
        Ok(())
    }

    /// Uncommitted work goes in the diffs, or back out.
    pub(crate) fn toggle_uncommitted(&self) {
        let mut choice = self.choice.write().unwrap_or_else(PoisonError::into_inner);
        choice.uncommitted = !choice.uncommitted;
    }

    fn choice(&self) -> Choice {
        self.choice.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub(crate) async fn mr(&self, key: &MrKey) -> Result<Mr> {
        let git::Listing { repo, worktrees } = git::listing(&self.here).await?;
        mr_of(&repo, find(&worktrees, key)?, &self.choice()).await
    }

    pub(crate) async fn diffs(&self, key: &MrKey) -> Result<Vec<DiffFile>> {
        let worktree = self.worktree(key).await?;
        let Choice { target, uncommitted } = self.choice();
        // `Mr::no_diff` says why there is no base.
        let Ok(base) = git::merge_base(&worktree.path, &target).await else { return Ok(vec![]) };
        if !uncommitted {
            return Ok(split::files(&git::patch(&worktree.path, &base, Some(&worktree.head)).await?));
        }
        let (patch, untracked) = uncommitted_work(&worktree, &base).await?;
        let added = untracked.iter().map(|file| split::new_file(&file.path, &file.patch));
        Ok(split::files(&patch).into_iter().chain(added).collect())
    }

    /// The file at commit `sha`; a `sha` that is no commit is the uncommitted head, which only the disk holds.
    pub(crate) async fn file(&self, key: &MrKey, path: &str, sha: &Sha) -> Result<String> {
        safe_path(path)?;
        let worktree = self.worktree(key).await?;
        if git::is_commit(&worktree.path, sha).await {
            return git::show(&worktree.path, sha, path).await;
        }
        let bytes = tokio::fs::read(worktree.path.join(path)).await.with_context(|| format!("reading {path}"))?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn worktree(&self, key: &MrKey) -> Result<Worktree> {
        find(&git::listing(&self.here).await?.worktrees, key).cloned()
    }
}

/// No rows yet: the worktree screen fills them.
pub(super) fn queue() -> Fetched {
    let queue = Queue { me: String::new(), project: None, review_requested: vec![], authored: vec![], assigned: vec![], open: vec![] };
    Fetched { queue, missing: None }
}

/// What fails on a local diff: the keys that reach it are hidden, so only a stray call gets here.
pub(super) fn forge_only<T>() -> Result<T> {
    bail!("a local diff has no forge behind it")
}

fn key_of(worktree: &Worktree) -> Result<MrKey> {
    Ok(MrKey { host: Some(HOST.to_owned()), ..MrKey::new(worktree.project()?, worktree.number()) })
}

fn find<'a>(worktrees: &'a [Worktree], key: &MrKey) -> Result<&'a Worktree> {
    worktrees.iter().find(|worktree| worktree.number() == key.number).with_context(|| format!("{} is no longer a worktree", key.project))
}

/// The worktree as a review reads it; one with no merge base opens empty and says why.
async fn mr_of(repo: &str, worktree: &Worktree, choice: &Choice) -> Result<Mr> {
    let key = key_of(worktree)?;
    let Choice { target, uncommitted } = choice;
    let base = git::merge_base(&worktree.path, target).await;
    let no_diff = base.as_ref().err().map(ToString::to_string);
    let head = match &base {
        Ok(base) if *uncommitted => shown_head(worktree, base).await?,
        _ => worktree.head.clone(),
    };
    let base = base.unwrap_or_else(|_| worktree.head.clone());
    let (updated_at, files) = tokio::try_join!(git::commit_time(&worktree.path), git::uncommitted_files(&worktree.path))?;
    let label = worktree.label();
    Ok(Mr {
        project: key.project,
        number: key.number,
        title: format!("{repo} ~ {label}"),
        description: String::new(),
        state: MrState::Open,
        draft: false,
        author: User { id: 0, username: String::new(), name: String::new() },
        source_branch: label,
        target_branch: target.to_string(),
        web_url: String::new(),
        updated_at,
        refs: Refs { start: base.clone(), base, head },
        pipeline: None,
        changes_count: None,
        conflicts: false,
        reviewers: vec![],
        labels: vec![],
        approvals: Approvals::default(),
        merge: MergePlan::default(),
        mine: false,
        uncommitted: if *uncommitted { 0 } else { files },
        no_diff,
    })
}

async fn uncommitted_work(worktree: &Worktree, base: &Sha) -> Result<(String, Vec<git::Untracked>)> {
    tokio::try_join!(git::patch(&worktree.path, base, None), git::untracked(&worktree.path))
}

async fn shown_head(worktree: &Worktree, base: &Sha) -> Result<Sha> {
    let (patch, untracked) = uncommitted_work(worktree, base).await?;
    git::uncommitted_head(&patch, &untracked)
}

async fn checked(dir: &Path, text: &str) -> Result<Target> {
    let target = Target::parse(text)?;
    git::commit_of(dir, &target).await?;
    Ok(target)
}

async fn default_target(dir: &Path) -> Result<Target> {
    for name in DEFAULT_TARGETS {
        if let Ok(target) = checked(dir, name).await {
            return Ok(target);
        }
    }
    bail!("no origin/HEAD, main or master here: name the target, `revu local <target>`")
}
