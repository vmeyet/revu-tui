//! Where a newer `mr` comes from, and whether the running one is already it.
use crate::version;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The public repository, once there is one; until then the checkout the binary was built from.
pub const REPO: Option<&str> = None;

/// What `mr update` rebuilds from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Remote(&'static str),
    Checkout(PathBuf),
}

impl Source {
    /// The public repo wins; otherwise the build directory, when it is a checkout of its own
    /// (a `cargo install --git` build lives in cargo's cache, which is no place to pull into).
    pub fn find() -> Result<Self> {
        if let Some(repo) = REPO {
            return Ok(Source::Remote(repo));
        }
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        if !dir.join(".git").exists() {
            bail!("no source to update from: {} is not a git checkout", dir.display());
        }
        Ok(Source::Checkout(dir.to_owned()))
    }

    /// The newest commit there, after a fast-forward pull when the checkout tracks a remote.
    pub fn latest(&self) -> Result<String> {
        match self {
            Source::Remote(repo) => {
                let listing = git(None, &["ls-remote", repo, "HEAD"])?;
                listing.split_whitespace().next().map(str::to_owned).with_context(|| format!("{repo} has no HEAD"))
            }
            Source::Checkout(dir) => {
                if git(Some(dir), &["rev-parse", "--abbrev-ref", "@{upstream}"]).is_ok() {
                    git(Some(dir), &["pull", "--ff-only", "--quiet"])?;
                }
                git(Some(dir), &["rev-parse", "HEAD"])
            }
        }
    }

    /// Uncommitted changes a checkout build would carry, so the user hears about them first.
    pub fn dirty(&self) -> bool {
        match self {
            Source::Remote(_) => false,
            Source::Checkout(dir) => git(Some(dir), &["status", "--porcelain"]).is_ok_and(|s| !s.is_empty()),
        }
    }

    pub fn install(&self) -> Result<()> {
        let mut cargo = Command::new("cargo");
        match self {
            Source::Remote(repo) => cargo.args(["install", "--git", repo, "--force"]),
            Source::Checkout(dir) => cargo.arg("install").arg("--path").arg(dir).arg("--force"),
        };
        if !cargo.status().context("running cargo install")?.success() {
            bail!("cargo install failed");
        }
        Ok(())
    }
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::Remote(repo) => f.write_str(repo),
            Source::Checkout(dir) => write!(f, "{}", dir.display()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Install,
    UpToDate,
}

/// Only a commit we know we already run spares the rebuild; anything unanswered installs.
pub fn decide(installed: &str, latest: Option<&str>, force: bool) -> Decision {
    match latest {
        Some(latest) if !force && installed != version::UNKNOWN && latest == installed => Decision::UpToDate,
        _ => Decision::Install,
    }
}

fn git(dir: Option<&Path>, args: &[&str]) -> Result<String> {
    let mut command = Command::new("git");
    if let Some(dir) = dir {
        command.arg("-C").arg(dir);
    }
    let output = command.args(args).env("GIT_TERMINAL_PROMPT", "0").output().context("running git")?;
    if !output.status.success() {
        bail!("git {}: {}", args.join(" "), String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const INSTALLED: &str = "9731436a0e7c4d1b2f3a4b5c6d7e8f9a0b1c2d3e";
    const NEWER: &str = "635cf1b0000000000000000000000000000000ff";

    #[test]
    fn same_commit_needs_no_install() {
        assert_eq!(decide(INSTALLED, Some(INSTALLED), false), Decision::UpToDate);
    }

    #[test]
    fn a_newer_commit_installs() {
        assert_eq!(decide(INSTALLED, Some(NEWER), false), Decision::Install);
    }

    #[test]
    fn force_installs_over_the_same_commit() {
        assert_eq!(decide(INSTALLED, Some(INSTALLED), true), Decision::Install);
    }

    #[test]
    fn a_failed_check_installs() {
        assert_eq!(decide(INSTALLED, None, false), Decision::Install);
    }

    #[test]
    fn an_unknown_installed_commit_installs() {
        assert_eq!(decide(version::UNKNOWN, Some(version::UNKNOWN), false), Decision::Install);
    }

    #[test]
    fn this_checkout_is_a_source() {
        assert_eq!(Source::find().unwrap(), Source::Checkout(PathBuf::from(env!("CARGO_MANIFEST_DIR"))));
    }
}
