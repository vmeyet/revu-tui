//! Where a newer `revu` comes from, and whether the running one is already it.
use crate::version;
use anyhow::{Context, Result, bail};
use std::process::Command;

/// Where `revu update` installs from.
pub const REPO: &str = "https://github.com/vmeyet/revu-tui";

/// The newest commit of the repo.
pub fn latest() -> Result<String> {
    let listing = git(&["ls-remote", REPO, "HEAD"])?;
    listing.split_whitespace().next().map(str::to_owned).with_context(|| format!("{REPO} has no HEAD"))
}

/// Rebuilds and installs from the repo with cargo.
pub fn install() -> Result<()> {
    let status = Command::new("cargo").args(["install", "--git", REPO, "--force"]).status().context("running cargo install")?;
    if !status.success() {
        bail!("cargo install failed");
    }
    Ok(())
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

fn git(args: &[&str]) -> Result<String> {
    let output = Command::new("git").args(args).env("GIT_TERMINAL_PROMPT", "0").output().context("running git")?;
    if !output.status.success() {
        bail!("git {}: {}", args.join(" "), String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
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
}
