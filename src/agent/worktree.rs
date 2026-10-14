//! The folder a new session works in: a git worktree on the MR's own branch, never a new branch.
use super::{Call, Run};
use anyhow::{Result, anyhow, bail};
use std::path::{Path, PathBuf};

/// Where revu adds worktrees, under the checkout's root.
const WORKTREES: &str = ".claude/worktrees";
/// What git says when the remote has no such branch, as for a branch that lives on a fork.
const NO_SUCH_BRANCH: &str = "couldn't find remote ref";

/// A folder on the MR's branch, and what the session should know about that branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Folder {
    pub(super) dir: PathBuf,
    /// Said in the toast: a branch revu found already there is used as it is, even behind origin.
    pub(super) notice: Option<String>,
}

/// The worktree of `repo` on `branch`. Origin's branch is fetched first. A worktree that has the
/// branch checked out, the main checkout included, is used as it is; so is a branch the checkout
/// holds, in a new worktree. revu never moves either: it says when they are behind origin or have
/// left it. With neither, the new worktree starts from origin's branch.
pub(super) fn on_branch(run: &dyn Run, repo: &Path, branch: &str) -> Result<Folder> {
    if branch.starts_with('-') || branch.contains("..") {
        bail!("revu does not trust the branch name {}", branch.escape_debug());
    }
    let listed = git(run, repo, &["worktree", "list", "--porcelain"])?;
    let fetched = run.output(&git_call(repo, &["fetch", "origin", &format!("+refs/heads/{branch}:refs/remotes/origin/{branch}")]).slow());
    if let Some(dir) = checked_out(&listed, branch) {
        return Ok(Folder { dir, notice: fetched.is_ok().then(|| notice(run, repo, branch)).flatten() });
    }
    let dir = repo.join(WORKTREES).join(branch.replace('/', "+"));
    let place = dir.to_string_lossy();
    let is_here = git(run, repo, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")]).is_ok();
    if is_here {
        run.output(&git_call(repo, &["worktree", "add", &place, branch]).slow())?;
        return Ok(Folder { dir: dir.clone(), notice: fetched.is_ok().then(|| notice(run, repo, branch)).flatten() });
    }
    fetched.map_err(|why| not_fetched(branch, &why))?;
    run.output(&git_call(repo, &["worktree", "add", "-b", branch, &place, &format!("origin/{branch}")]).slow())?;
    Ok(Folder { dir, notice: None })
}

/// Why no session starts: the remote has no such branch, or whatever else git said.
fn not_fetched(branch: &str, why: &anyhow::Error) -> anyhow::Error {
    let why = format!("{why:#}");
    if why.contains(NO_SUCH_BRANCH) {
        return anyhow!("no session started: the branch `{branch}` is not on origin, as with an MR from a fork");
    }
    anyhow!("no session started: the branch `{branch}` could not be fetched from origin: {why}")
}

/// How the local `branch` stands against origin's, when it is not level with it or ahead of it.
fn notice(run: &dyn Run, repo: &Path, branch: &str) -> Option<String> {
    let counted = git(run, repo, &["rev-list", "--left-right", "--count", &format!("{branch}...origin/{branch}")]).ok()?;
    let mut counts = counted.split_whitespace().map(str::parse::<u32>);
    let (ahead, behind) = (counts.next()?.ok()?, counts.next()?.ok()?);
    let commits = if behind == 1 { "commit" } else { "commits" };
    match (ahead, behind) {
        (_, 0) => None,
        (0, _) => Some(format!("its branch is {behind} {commits} behind origin: pull before you trust it")),
        (_, _) => Some("its branch and origin's have gone separate ways: revu left it as it is".to_owned()),
    }
}

/// The main checkout of the repository `dir` is in, which git lists first: started in a worktree,
/// revu still adds the next one beside it and not inside it. `dir` itself when git does not say.
pub(super) fn main_checkout(run: &dyn Run, dir: &Path) -> PathBuf {
    let listed = git(run, dir, &["worktree", "list", "--porcelain"]).unwrap_or_default();
    listed.lines().find_map(|line| line.strip_prefix("worktree ")).map_or_else(|| dir.to_owned(), PathBuf::from)
}

fn git_call(repo: &Path, args: &[&str]) -> Call {
    Call::new(&[&["git"], args].concat()).in_dir(repo)
}

fn git(run: &dyn Run, repo: &Path, args: &[&str]) -> Result<String> {
    run.output(&git_call(repo, args))
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
    const LIST: &str = "git worktree list --porcelain";
    const FETCH: &str = "git fetch origin +refs/heads/feat/checkout:refs/remotes/origin/feat/checkout";
    const NEW: &str = "/code/widgets/.claude/worktrees/feat+checkout";

    fn listed() -> Script {
        Script::default().says("git worktree list", LISTED)
    }

    fn only_on_origin() -> Script {
        listed().fails("git rev-parse", "`git rev-parse` failed: no message")
    }

    #[test]
    fn a_branch_already_checked_out_reuses_its_folder_the_main_checkout_included() {
        let script = listed().says("git rev-list", "0\t0\n");
        let folder = on_branch(&script, Path::new(REPO), "fix/cache").unwrap();
        assert_eq!(folder, Folder { dir: "/code/widgets/.claude/worktrees/fix+cache".into(), notice: None });
        assert_eq!(on_branch(&script, Path::new(REPO), "main").unwrap().dir, PathBuf::from(REPO));
        let lines = script.lines();
        assert_eq!(
            lines[..3],
            [
                LIST,
                "git fetch origin +refs/heads/fix/cache:refs/remotes/origin/fix/cache",
                "git rev-list --left-right --count fix/cache...origin/fix/cache"
            ]
        );
        assert!(!lines.iter().any(|line| line.starts_with("git worktree add")), "nothing is added");
    }

    #[test]
    fn a_branch_found_behind_origin_or_apart_from_it_is_left_alone_and_the_toast_says_so() {
        let stands = |counts: &str| on_branch(&listed().says("git rev-list", counts), Path::new(REPO), "fix/cache").unwrap().notice;
        assert_eq!(stands("0\t3\n").as_deref(), Some("its branch is 3 commits behind origin: pull before you trust it"));
        assert_eq!(stands("0\t1\n").as_deref(), Some("its branch is 1 commit behind origin: pull before you trust it"));
        assert_eq!(stands("2\t5\n").as_deref(), Some("its branch and origin's have gone separate ways: revu left it as it is"));
        assert_eq!(stands("2\t0\n"), None, "ahead of origin is the reader's own work");
        assert_eq!(stands("nonsense"), None);
        let unreachable = listed().fails("git fetch", "`git fetch` failed: fatal: unable to access");
        let folder = on_branch(&unreachable, Path::new(REPO), "fix/cache").unwrap();
        assert_eq!(folder.notice, None, "origin did not answer: nothing to compare with");
        assert!(!unreachable.lines().iter().any(|line| line.starts_with("git rev-list")));
    }

    #[test]
    fn a_branch_the_checkout_holds_gets_a_worktree_as_it_is_never_moved() {
        let script = listed().says("git rev-list", "0\t4\n");
        let folder = on_branch(&script, Path::new(REPO), "feat/checkout").unwrap();
        assert_eq!(folder.dir, PathBuf::from(NEW));
        assert!(folder.notice.unwrap().contains("4 commits behind origin"));
        let add = "git worktree add /code/widgets/.claude/worktrees/feat+checkout feat/checkout";
        let count = "git rev-list --left-right --count feat/checkout...origin/feat/checkout";
        assert_eq!(script.lines(), [LIST, FETCH, "git rev-parse --verify --quiet refs/heads/feat/checkout", add, count]);
    }

    #[test]
    fn a_branch_only_origin_holds_is_fetched_over_a_rewritten_one_and_the_worktree_starts_from_it() {
        let script = only_on_origin();
        let folder = on_branch(&script, Path::new(REPO), "feat/checkout").unwrap();
        assert_eq!(folder, Folder { dir: NEW.into(), notice: None });
        let add = "git worktree add -b feat/checkout /code/widgets/.claude/worktrees/feat+checkout origin/feat/checkout";
        assert_eq!(script.lines(), [LIST, FETCH, "git rev-parse --verify --quiet refs/heads/feat/checkout", add]);
        assert!(FETCH.contains(" +refs/heads/"), "the + lets a force-pushed branch replace the one fetched before");
        assert!(script.calls().iter().all(|call| call.dir.as_deref() == Some(Path::new(REPO))), "git runs in the checkout");
    }

    #[test]
    fn a_branch_origin_does_not_have_starts_nothing_and_says_it_may_be_a_fork() {
        let script = only_on_origin().fails("git fetch", "`git fetch` failed: fatal: couldn't find remote ref refs/heads/feat/checkout");
        let err = format!("{:#}", on_branch(&script, Path::new(REPO), "feat/checkout").unwrap_err());
        assert_eq!(err, "no session started: the branch `feat/checkout` is not on origin, as with an MR from a fork");
        assert!(!script.lines().iter().any(|line| line.starts_with("git worktree add")), "no worktree is added");
    }

    #[test]
    fn any_other_fetch_failure_gives_gits_own_reason_and_no_guess() {
        let rejected = "`git fetch` failed: ! [rejected] feat/checkout -> origin/feat/checkout  (non-fast-forward)";
        let script = only_on_origin().fails("git fetch", rejected);
        let err = format!("{:#}", on_branch(&script, Path::new(REPO), "feat/checkout").unwrap_err());
        assert_eq!(err, format!("no session started: the branch `feat/checkout` could not be fetched from origin: {rejected}"));
        assert!(!err.contains("fork"));
    }

    #[test]
    fn the_fetch_and_the_new_worktree_get_the_long_limit_and_a_stuck_fetch_starts_nothing() {
        let script = only_on_origin();
        on_branch(&script, Path::new(REPO), "feat/checkout").unwrap();
        let limits: Vec<u64> = script.calls().iter().map(|call| call.limit.as_secs()).collect();
        assert_eq!(limits, [15, 120, 15, 120], "{:?}", script.lines());
        let stuck = only_on_origin().fails("git fetch", "`git fetch` took longer than 120s: revu stopped it");
        let err = format!("{:#}", on_branch(&stuck, Path::new(REPO), "feat/checkout").unwrap_err());
        assert!(err.starts_with("no session started") && err.ends_with("revu stopped it"), "{err}");
    }

    #[test]
    fn a_worktree_gives_the_main_checkout_and_a_folder_git_does_not_know_gives_itself() {
        let script = listed();
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
