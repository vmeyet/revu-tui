//! Where a newer `revu` comes from, and whether the running one is already it.
use crate::cache::Cache;
use crate::version;
use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

/// Where `revu update` installs from.
pub const REPO: &str = "https://github.com/vmeyet/revu-tui";

/// cargo's build folder, kept in the cache between updates so only revu recompiles.
/// Hosts are the other names at the cache root and always hold a dot, so this one cannot collide.
const BUILD_FOLDER: &str = "cargo_target";

/// The newest commit of the repo.
pub fn latest() -> Result<String> {
    let listing = git(&["ls-remote", REPO, "HEAD"])?;
    listing.split_whitespace().next().map(str::to_owned).with_context(|| format!("{REPO} has no HEAD"))
}

/// Rebuilds and installs from the repo with cargo, reusing the dependencies built last time.
pub fn install() -> Result<()> {
    let build = Cache::shared().folder(BUILD_FOLDER)?;
    forget_revu(&build)?;
    let status = Command::new("cargo")
        .args(["install", "--git", REPO, "--force", "--target-dir"])
        .arg(&build)
        .status()
        .context("running cargo install")?;
    if !status.success() {
        bail!("cargo install failed");
    }
    Ok(())
}

/// cargo ties a `--git` build to the repo URL, never to the commit, so a kept build folder would
/// look fresh and reinstall the old binary with its old commit hash. Dropping revu's own
/// fingerprints recompiles revu and reruns `build.rs`; the dependencies stay built.
fn forget_revu(build: &Path) -> Result<()> {
    let Ok(entries) = std::fs::read_dir(build.join("release").join(".fingerprint")) else { return Ok(()) };
    let prefix = concat!(env!("CARGO_PKG_NAME"), "-");
    for entry in entries {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with(prefix) {
            std::fs::remove_dir_all(entry.path()).with_context(|| format!("removing {}", entry.path().display()))?;
        }
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
    fn forgetting_revu_drops_only_its_own_fingerprints() {
        let build = tempfile::tempdir().unwrap();
        let fingerprints = build.path().join("release").join(".fingerprint");
        for name in ["revu-3483aceb8f1a5a28", "revu-e613dd567bc7d97f", "serde-0123456789abcdef", "revulsion-0000000000000000"] {
            std::fs::create_dir_all(fingerprints.join(name)).unwrap();
        }
        forget_revu(build.path()).unwrap();
        let mut left: Vec<String> =
            std::fs::read_dir(&fingerprints).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["revulsion-0000000000000000", "serde-0123456789abcdef"]);
    }

    #[test]
    fn forgetting_revu_in_a_folder_never_built_is_fine() {
        let build = tempfile::tempdir().unwrap();
        forget_revu(&build.path().join("missing")).unwrap();
        forget_revu(build.path()).unwrap();
    }

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
