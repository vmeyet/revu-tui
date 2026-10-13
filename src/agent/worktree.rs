//! The folder a new session works in: a git worktree on the MR's own branch, never a new branch.
use super::{Call, Run};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

/// Where revu adds worktrees, under the checkout's root.
const WORKTREES: &str = ".claude/worktrees";

/// The worktree of `repo` that has `branch` checked out, the main checkout included; without one,
/// the branch is fetched from origin and a worktree is added for it.
pub(super) fn on_branch(run: &dyn Run, repo: &Path, branch: &str) -> Result<PathBuf> {
    if branch.starts_with('-') || branch.contains("..") {
        bail!("revu does not trust the branch name {}", branch.escape_debug());
    }
    let listed = git(run, repo, &["worktree", "list", "--porcelain"])?;
    if let Some(folder) = checked_out(&listed, branch) {
        return Ok(folder);
    }
    let refspec = format!("refs/heads/{branch}:refs/remotes/origin/{branch}");
    git(run, repo, &["fetch", "origin", &refspec])
        .with_context(|| format!("no session started: the branch `{branch}` is not on origin, as with an MR from a fork"))?;
    let folder = repo.join(WORKTREES).join(branch.replace('/', "+"));
    git(run, repo, &["worktree", "add", &folder.to_string_lossy(), branch])?;
    Ok(folder)
}

/// The main checkout of the repository `dir` is in, which git lists first: started in a worktree,
/// revu still adds the next one beside it and not inside it. `dir` itself when git does not say.
pub(super) fn main_checkout(run: &dyn Run, dir: &Path) -> PathBuf {
    let listed = git(run, dir, &["worktree", "list", "--porcelain"]).unwrap_or_default();
    listed.lines().find_map(|line| line.strip_prefix("worktree ")).map_or_else(|| dir.to_owned(), PathBuf::from)
}

fn git(run: &dyn Run, repo: &Path, args: &[&str]) -> Result<String> {
    run.output(&Call::new(&[&["git"], args].concat()).in_dir(repo))
}

/// The folder of the worktree whose `branch` line names `branch`, in what `git worktree list --porcelain` printed.
fn checked_out(listed: &str, branch: &str) -> Option<PathBuf> {
    let wanted = format!("branch refs/heads/{branch}");
    let worktree = listed.split("\n\n").find(|worktree| worktree.lines().any(|line| line == wanted))?;
    worktree.lines().find_map(|line| line.strip_prefix("worktree ")).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::super::tests::{REPO, Script};
    use super::*;

    const LISTED: &str = "worktree /code/widgets\nHEAD aaaa\nbranch refs/heads/main\n\nworktree /code/widgets/.claude/worktrees/fix+cache\nHEAD bbbb\nbranch refs/heads/fix/cache\n\nworktree /code/widgets/.claude/worktrees/old\nHEAD cccc\ndetached\n";

    #[test]
    fn a_branch_already_checked_out_reuses_its_folder_the_main_checkout_included() {
        let script = Script::default().says("git worktree list", LISTED);
        assert_eq!(on_branch(&script, Path::new(REPO), "fix/cache").unwrap(), PathBuf::from("/code/widgets/.claude/worktrees/fix+cache"));
        assert_eq!(on_branch(&script, Path::new(REPO), "main").unwrap(), PathBuf::from(REPO));
        assert!(script.lines().iter().all(|line| line == "git worktree list --porcelain"), "nothing fetched, nothing added");
    }

    #[test]
    fn a_branch_checked_out_nowhere_is_fetched_and_gets_a_worktree_named_after_it() {
        let script = Script::default().says("git worktree list", LISTED);
        let folder = on_branch(&script, Path::new(REPO), "feat/checkout").unwrap();
        assert_eq!(folder, PathBuf::from("/code/widgets/.claude/worktrees/feat+checkout"));
        assert_eq!(
            script.lines(),
            [
                "git worktree list --porcelain",
                "git fetch origin refs/heads/feat/checkout:refs/remotes/origin/feat/checkout",
                "git worktree add /code/widgets/.claude/worktrees/feat+checkout feat/checkout",
            ]
        );
        assert!(script.calls().iter().all(|call| call.dir.as_deref() == Some(Path::new(REPO))), "git runs in the checkout");
    }

    #[test]
    fn a_branch_origin_does_not_hold_starts_nothing_and_says_it_may_be_a_fork() {
        let script = Script::default().says("git worktree list", LISTED).fails("git fetch", "`git fetch` failed: couldn't find remote ref");
        let err = format!("{:#}", on_branch(&script, Path::new(REPO), "feat/checkout").unwrap_err());
        assert!(err.contains("feat/checkout") && err.contains("fork") && err.contains("couldn't find remote ref"), "{err}");
        assert!(!script.lines().iter().any(|line| line.starts_with("git worktree add")), "no worktree is added");
    }

    #[test]
    fn a_worktree_gives_the_main_checkout_and_a_folder_git_does_not_know_gives_itself() {
        let script = Script::default().says("git worktree list", LISTED);
        let worktree = Path::new("/code/widgets/.claude/worktrees/fix+cache");
        assert_eq!(main_checkout(&script, worktree), PathBuf::from(REPO));
        assert_eq!(script.calls()[0].dir.as_deref(), Some(worktree));
        let unknown = Script::default().fails("git worktree list", "not a git repository");
        assert_eq!(main_checkout(&unknown, Path::new("/tmp/elsewhere")), PathBuf::from("/tmp/elsewhere"));
    }

    #[test]
    fn a_branch_name_that_reads_as_a_flag_or_climbs_out_is_refused_before_git_runs() {
        let script = Script::default();
        for branch in ["--upload-pack=x", "a/../../b"] {
            assert!(on_branch(&script, Path::new(REPO), branch).is_err(), "{branch}");
        }
        assert_eq!(script.lines(), [] as [String; 0]);
    }
}
