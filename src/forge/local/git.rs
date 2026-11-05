//! Every `git` call of the local backend, on `tokio::process` so nothing blocks the loop.
use super::Target;
use super::split::DIFF_FLAGS;
use crate::forge::{ProjectPath, Sha, Size};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// How many untracked files are read at once.
const UNTRACKED_AT_ONCE: usize = 8;
/// Git's name for a commit that does not exist yet: a worktree on an unborn branch.
const NO_COMMIT: &str = "0000000000000000000000000000000000000000";

/// One entry of `git worktree list` that can be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Worktree {
    pub(super) path: PathBuf,
    pub(super) head: Sha,
    /// `None` on a detached `HEAD`.
    branch: Option<String>,
}

impl Worktree {
    /// Its branch, or the short sha of a detached `HEAD`.
    pub(super) fn label(&self) -> String {
        self.branch.clone().unwrap_or_else(|| crate::version::short(self.head.as_str()).to_owned())
    }

    /// `local/<folder>`, its folder made safe for a project path.
    pub(super) fn project(&self) -> Result<ProjectPath> {
        let safe = |c: char| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '-' };
        let folder: String = folder_of(&self.path).chars().map(safe).collect();
        ProjectPath::parse(&format!("local/{folder}")).context("this worktree's folder cannot name a project")
    }

    /// A stable number from its path: the same worktree finds its `state.json` again on every run.
    pub(super) fn number(&self) -> u64 {
        let digest = Sha256::digest(self.path.as_os_str().as_encoded_bytes());
        digest.iter().take(8).fold(0, |number, byte| (number << 8) | u64::from(*byte))
    }
}

fn folder_of(path: &Path) -> String {
    path.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

/// What `git worktree list` says: the repo's name, and the worktrees to read, bare and prunable ones left out.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Listing {
    pub(super) repo: String,
    pub(super) worktrees: Vec<Worktree>,
}

pub(super) async fn listing(dir: &Path) -> Result<Listing> {
    Ok(parse_listing(&run(dir, &["worktree", "list", "--porcelain", "-z"]).await?))
}

pub(super) fn parse_listing(output: &str) -> Listing {
    let records: Vec<Vec<&str>> = output.split("\0\0").map(|record| record.split('\0').collect()).collect();
    let main = records.first().and_then(|record| record.first()).and_then(|line| line.strip_prefix("worktree "));
    let repo = main.map(|path| folder_of(Path::new(path)).trim_end_matches(".git").to_owned()).unwrap_or_default();
    Listing { repo, worktrees: records.iter().filter_map(|record| parse_worktree(record)).collect() }
}

fn parse_worktree(fields: &[&str]) -> Option<Worktree> {
    let value = |name: &str| fields.iter().find_map(|field| field.strip_prefix(name)?.strip_prefix(' '));
    let has = |name: &str| fields.iter().any(|field| field.split(' ').next() == Some(name));
    if has("bare") || has("prunable") || value("HEAD") == Some(NO_COMMIT) {
        return None;
    }
    let branch = value("branch").map(|branch| branch.strip_prefix("refs/heads/").unwrap_or(branch).to_owned());
    Some(Worktree { path: value("worktree")?.into(), head: Sha::parse(value("HEAD")?)?, branch })
}

pub(super) async fn toplevel(dir: &Path) -> Result<PathBuf> {
    Ok(run(dir, &["rev-parse", "--show-toplevel"]).await?.trim().into())
}

/// What `target` names, refused when that is no commit.
pub(super) async fn commit_of(dir: &Path, target: &Target) -> Result<String> {
    let commit = format!("{target}^{{commit}}");
    run(dir, &["rev-parse", "--verify", "--end-of-options", &commit]).await.with_context(|| format!("{target} is not a commit here"))
}

/// Where `target` and `HEAD` split; fails when they share no history.
pub(super) async fn merge_base(dir: &Path, target: &Target) -> Result<Sha> {
    let tip = commit_of(dir, target).await?;
    let base =
        run(dir, &["merge-base", tip.trim(), "HEAD"]).await.with_context(|| format!("{target} shares no history with this branch"))?;
    parse_sha(&base)
}

fn parse_sha(text: &str) -> Result<Sha> {
    Sha::parse(text.trim()).with_context(|| format!("git answered `{}` for a commit", text.trim()))
}

pub(super) async fn is_commit(dir: &Path, sha: &Sha) -> bool {
    run(dir, &["cat-file", "-e", &format!("{sha}^{{commit}}")]).await.is_ok()
}

pub(super) async fn show(dir: &Path, sha: &Sha, path: &str) -> Result<String> {
    run(dir, &["show", "--no-textconv", &format!("{sha}:{path}")]).await
}

pub(super) async fn commit_time(dir: &Path) -> Result<DateTime<Utc>> {
    let time = run(dir, &["log", "-1", "--no-show-signature", "--format=%cI"]).await?;
    time.trim().parse().with_context(|| format!("git answered `{}` for a commit time", time.trim()))
}

/// The files holding uncommitted work: staged, unstaged or untracked.
pub(super) async fn uncommitted_files(dir: &Path) -> Result<Vec<String>> {
    let status = run(dir, &["status", "--porcelain", "--no-renames", "--untracked-files=all", "-z"]).await?;
    Ok(status.split('\0').filter_map(|entry| entry.get(3..)).map(str::to_owned).collect())
}

/// When any of `files` was last saved; one that is gone, as a deleted file is, does not count.
pub(super) async fn last_saved(dir: &Path, files: &[String]) -> Option<DateTime<Utc>> {
    let (dir, files) = (dir.to_owned(), files.to_vec());
    let newest = move || Ok(files.iter().filter_map(|file| std::fs::metadata(dir.join(file)).ok()?.modified().ok()).max());
    crate::blocking::run(newest)
        .await
        .inspect_err(|e| tracing::warn!("reading when files were saved: {e:#}"))
        .ok()
        .flatten()
        .map(DateTime::from)
}

/// How many commits lead from `base` to `head`.
pub(super) async fn commits_between(dir: &Path, base: &Sha, head: &Sha) -> Result<u32> {
    let count = run(dir, &["rev-list", "--count", &format!("{base}..{head}")]).await?;
    count.trim().parse().with_context(|| format!("git answered `{}` for a count", count.trim()))
}

/// What changed from `base` to `head`, in lines and files; a binary file counts as a file only.
pub(super) async fn size_between(dir: &Path, base: &Sha, head: &Sha) -> Result<Size> {
    let args: Vec<&str> = ["diff", "--numstat"].into_iter().chain(DIFF_FLAGS).chain([base.as_str(), head.as_str()]).collect();
    Ok(parse_numstat(&run(dir, &args).await?))
}

fn parse_numstat(numstat: &str) -> Size {
    let count = |word: Option<&str>| word.and_then(|word| word.parse::<u32>().ok()).unwrap_or(0);
    numstat.lines().map(|line| line.split('\t')).fold(Size { additions: 0, deletions: 0, files: 0 }, |size, mut words| Size {
        additions: size.additions + count(words.next()),
        deletions: size.deletions + count(words.next()),
        files: size.files + 1,
    })
}

/// `git diff` from `base`: to `head`, or to the files on disk without one.
pub(super) async fn patch(dir: &Path, base: &Sha, head: Option<&Sha>) -> Result<String> {
    let args: Vec<&str> =
        ["diff", "--raw", "-p", "-z"].into_iter().chain(DIFF_FLAGS).chain([base.as_str()]).chain(head.map(Sha::as_str)).collect();
    run(dir, &args).await
}

/// One file git does not track, as a patch that adds it.
pub(super) struct Untracked {
    pub(super) path: String,
    pub(super) patch: String,
}

pub(super) async fn untracked(dir: &Path) -> Result<Vec<Untracked>> {
    let listed = run(dir, &["ls-files", "--others", "--exclude-standard", "-z"]).await?;
    // A nested repository is listed as a folder, which no patch can add.
    let paths: Vec<&str> = listed.split('\0').filter(|path| !path.is_empty() && !path.ends_with('/')).collect();
    let mut files = Vec::with_capacity(paths.len());
    for batch in paths.chunks(UNTRACKED_AT_ONCE) {
        files.extend(futures_util::future::try_join_all(batch.iter().map(|path| untracked_patch(dir, path))).await?);
    }
    Ok(files)
}

/// `--no-index` exits 1 when it finds a difference, which is what it is asked for here.
async fn untracked_patch(dir: &Path, path: &str) -> Result<Untracked> {
    let output =
        git(dir).args(["diff", "--no-index"]).args(DIFF_FLAGS).args(["--", "/dev/null", path]).output().await.context("running git")?;
    if !matches!(output.status.code(), Some(0 | 1)) {
        bail!("{}", first_line(&output.stderr));
    }
    Ok(Untracked { path: path.to_owned(), patch: String::from_utf8_lossy(&output.stdout).into_owned() })
}

/// What stands for `HEAD` once uncommitted work is shown: moves whenever the diff or an untracked file does.
pub(super) fn uncommitted_head(patch: &str, untracked: &[Untracked]) -> Result<Sha> {
    let digest =
        untracked.iter().fold(Sha256::new().chain_update(patch), |digest, file| digest.chain_update(&file.path).chain_update(&file.patch));
    parse_sha(&format!("{:x}", digest.finalize()))
}

async fn run(dir: &Path, args: &[&str]) -> Result<String> {
    let output = git(dir).args(args).output().await.context("running git")?;
    if !output.status.success() {
        bail!("{}", first_line(&output.stderr));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn first_line(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr).lines().next().unwrap_or("git failed").to_owned()
}

/// `GIT_OPTIONAL_LOCKS=0`: a read every 2s must not take the index lock from the reader's own git.
fn git(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir).env("GIT_OPTIONAL_LOCKS", "0");
    command
}
