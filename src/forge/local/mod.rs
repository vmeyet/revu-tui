//! The local backend: the branch of a worktree read with `git`, with no host and no token. It answers in the neutral
//! model like a forge, a worktree standing for an MR; what only a forge can do fails.
mod git;
mod split;
#[cfg(test)]
pub(crate) mod tests;

use super::{Approvals, DiffFile, Fetched, MergePlan, Mr, MrKey, MrState, Queue, QueueMr, Refs, Sha, Size, User, WorktreeRow};
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

/// What `B` and `I` ask for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Change {
    /// Compare against this, as the user typed it.
    Target(String),
    /// Show uncommitted work, or hide it.
    Uncommitted,
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
        let listing = self.listing().await?;
        let here =
            listing.worktrees.iter().find(|worktree| worktree.path == self.here).context("this worktree is not in `git worktree list`")?;
        key_of(here)
    }

    /// What the next answers show; a target that names no commit changes nothing.
    pub(crate) async fn change(&self, change: Change) -> Result<()> {
        match change {
            Change::Target(target) => self.set_target(&target).await,
            Change::Uncommitted => {
                self.toggle_uncommitted();
                Ok(())
            }
        }
    }

    async fn set_target(&self, target: &str) -> Result<()> {
        let target = checked(&self.here, target).await?;
        self.choice.write().unwrap_or_else(PoisonError::into_inner).target = target;
        Ok(())
    }

    /// Uncommitted work goes in the diffs, or back out.
    fn toggle_uncommitted(&self) {
        let mut choice = self.choice.write().unwrap_or_else(PoisonError::into_inner);
        choice.uncommitted = !choice.uncommitted;
    }

    fn choice(&self) -> Choice {
        self.choice.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub(crate) async fn mr(&self, key: &MrKey) -> Result<Mr> {
        let git::Listing { repo, worktrees } = self.listing().await?;
        read_mr(&repo, find(&worktrees, key)?, &self.choice()).await
    }

    /// The worktree as `mr` names it, opening empty for `reason`.
    pub(crate) async fn mr_saying(&self, key: &MrKey, reason: String) -> Result<Mr> {
        let git::Listing { repo, worktrees } = self.listing().await?;
        Ok(Mr { no_diff: Some(reason), ..bare_mr(&repo, find(&worktrees, key)?, &self.choice().target)? })
    }

    /// One row per worktree, in three lists: the one revu ran in, those ahead of the target, the rest.
    pub(crate) async fn queue(&self) -> Result<Fetched> {
        let git::Listing { repo, worktrees } = self.listing().await?;
        let committed = Choice { uncommitted: false, ..self.choice() };
        let rows = futures_util::future::try_join_all(worktrees.iter().map(|worktree| row(&repo, worktree, &committed))).await?;
        let here_number = worktrees.iter().find(|worktree| worktree.path == self.here).map(Worktree::number);
        let (here, others): (Vec<_>, Vec<_>) = rows.into_iter().partition(|row| Some(row.number) == here_number);
        let (ahead, rest): (Vec<_>, Vec<_>) = others.into_iter().partition(|row| row.worktree.as_ref().is_some_and(|w| w.commits > 0));
        let queue = Queue { me: repo, project: None, review_requested: ahead, authored: here, assigned: vec![], open: rest };
        Ok(Fetched { queue, missing: None })
    }

    pub(crate) async fn diffs(&self, key: &MrKey) -> Result<Vec<DiffFile>> {
        let worktree = self.worktree(key).await?;
        let Choice { target, uncommitted } = self.choice();
        // `Mr::no_diff` says why there is no base.
        let base = if worktree.unreadable().await.is_some() { None } else { git::merge_base(&worktree.path, &target).await? };
        let Some(base) = base else { return Ok(vec![]) };
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

    /// Where the worktree holds the file on disk, unless `sha` is an older commit, which only git holds; with uncommitted
    /// work shown, even `HEAD` is older than the disk.
    pub(crate) async fn real_file(&self, key: &MrKey, path: &str, sha: &Sha) -> Result<Option<PathBuf>> {
        safe_path(path)?;
        let worktree = self.worktree(key).await?;
        let is_older = (self.choice().uncommitted || *sha != worktree.head) && git::is_commit(&worktree.path, sha).await;
        Ok((!is_older).then(|| worktree.path.join(path)))
    }

    async fn worktree(&self, key: &MrKey) -> Result<Worktree> {
        find(&self.listing().await?.worktrees, key).cloned()
    }

    /// What `git worktree list` says, without the git folder a submodule's list names, the folder revu ran in always among it.
    async fn listing(&self) -> Result<git::Listing> {
        let git::Listing { repo, worktrees } = git::listing(&self.here).await?;
        let git_folder = git::common_dir(&self.here).await?;
        let mut worktrees: Vec<Worktree> = worktrees.into_iter().filter(|worktree| worktree.path != git_folder).collect();
        if !worktrees.iter().any(|worktree| worktree.path == self.here) {
            worktrees.push(git::worktree_at(&self.here).await?);
        }
        Ok(git::Listing { repo, worktrees })
    }
}

/// What fails on a local diff: the keys that reach it are hidden, so only a stray call gets here.
pub(crate) fn forge_only<T>() -> Result<T> {
    bail!("a local diff has no forge behind it")
}

fn key_of(worktree: &Worktree) -> Result<MrKey> {
    Ok(MrKey { host: Some(HOST.to_owned()), ..MrKey::new(worktree.project()?, worktree.number()) })
}

fn find<'a>(worktrees: &'a [Worktree], key: &MrKey) -> Result<&'a Worktree> {
    worktrees.iter().find(|worktree| worktree.number() == key.number).with_context(|| format!("{} is no longer a worktree", key.project))
}

/// The worktree as a review reads it; one with nothing to read opens empty and says why.
async fn read_mr(repo: &str, worktree: &Worktree, choice: &Choice) -> Result<Mr> {
    match worktree.unreadable().await {
        Some(reason) => Ok(Mr { no_diff: Some(reason.to_owned()), ..bare_mr(repo, worktree, &choice.target)? }),
        None => mr_of(repo, worktree, choice).await,
    }
}

/// One with no merge base opens empty and says why.
async fn mr_of(repo: &str, worktree: &Worktree, choice: &Choice) -> Result<Mr> {
    let Choice { target, uncommitted } = choice;
    let base = git::merge_base(&worktree.path, target).await?;
    let no_diff = base.is_none().then(|| format!("{target} shares no history with this branch"));
    let head = match &base {
        Some(base) if *uncommitted => shown_head(worktree, base).await?,
        _ => worktree.head.clone(),
    };
    let base = base.unwrap_or_else(|| worktree.head.clone());
    let committed = git::commit_time(&worktree.path).await?;
    let files = git::uncommitted_files(&worktree.path).await;
    let files = files.inspect_err(|e| tracing::warn!("counting uncommitted files failed: {e:#}")).unwrap_or_default();
    let saved = git::last_saved(&worktree.path, &files).await;
    Ok(Mr {
        updated_at: saved.map_or(committed, |saved| saved.max(committed)),
        refs: Refs { start: base.clone(), base, head },
        uncommitted: if *uncommitted { 0 } else { u32::try_from(files.len()).unwrap_or(u32::MAX) },
        no_diff,
        ..bare_mr(repo, worktree, target)?
    })
}

/// What needs no git to say: its names, at its `HEAD`, as nothing ahead of anything.
fn bare_mr(repo: &str, worktree: &Worktree, target: &Target) -> Result<Mr> {
    let key = key_of(worktree)?;
    let head = worktree.head.clone();
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
        updated_at: chrono::DateTime::UNIX_EPOCH,
        refs: Refs { base: head.clone(), start: head.clone(), head },
        pipeline: None,
        changes_count: None,
        conflicts: false,
        reviewers: vec![],
        labels: vec![],
        approvals: Approvals::default(),
        merge: MergePlan::default(),
        mine: false,
        uncommitted: 0,
        no_diff: None,
    })
}

/// The worktree as the screen lists it: its commits against the target, whatever `I` shows; whatever goes wrong
/// reading one, it keeps its row and says why.
async fn row(repo: &str, worktree: &Worktree, committed: &Choice) -> Result<QueueMr> {
    let mr = match read_mr(repo, worktree, committed).await {
        Ok(mr) => mr,
        Err(failed) => Mr { no_diff: Some(failed.to_string()), ..bare_mr(repo, worktree, &committed.target)? },
    };
    let Refs { base, head, .. } = &mr.refs;
    let (commits, size) = if mr.no_diff.is_some() {
        (0, Size { additions: 0, deletions: 0, files: 0 })
    } else {
        tokio::try_join!(git::commits_between(&worktree.path, base, head), git::size_between(&worktree.path, base, head))?
    };
    let row = QueueMr::from_mr(&mr, Some(HOST.to_owned())).context("a worktree is always open")?;
    Ok(QueueMr {
        title: mr.source_branch.clone(),
        size: Some(size),
        worktree: Some(WorktreeRow { commits, uncommitted: mr.uncommitted, no_diff: mr.no_diff.clone() }),
        ..row
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
