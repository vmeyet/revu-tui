use super::*;
use crate::forge::FileKind;
use std::process::Command;

/// A throwaway repository on `main` with one commit, and a `work` branch checked out.
pub(crate) struct Repo(tempfile::TempDir);

impl Repo {
    pub(crate) fn new() -> Self {
        let repo = Self(tempfile::tempdir().unwrap());
        repo.git(&["init", "-q", "-b", "main"]);
        repo.write("a.txt", "one\ntwo\n");
        repo.commit("start");
        repo.git(&["switch", "-q", "-c", "work"]);
        repo
    }

    pub(crate) fn path(&self) -> PathBuf {
        self.0.path().canonicalize().unwrap()
    }

    fn command(&self, args: &[&str]) -> std::process::Output {
        let identity = ["-c", "user.name=nina", "-c", "user.email=nina@acme.test"];
        Command::new("git")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .arg("-C")
            .arg(self.0.path())
            .args(identity)
            .args(args)
            .output()
            .unwrap()
    }

    pub(crate) fn git(&self, args: &[&str]) -> String {
        let output = self.command(args);
        assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8(output.stdout).unwrap()
    }

    pub(crate) fn write(&self, path: &str, text: &str) {
        std::fs::write(self.0.path().join(path), text).unwrap();
    }

    pub(crate) fn commit(&self, message: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", message]);
    }

    /// What `split` makes of `git diff` with `revisions`, as the backend runs it.
    fn parsed(&self, revisions: &[&str]) -> Vec<DiffFile> {
        let args: Vec<&str> = ["diff", "--raw", "-p", "-z"].into_iter().chain(split::DIFF_FLAGS).chain(revisions.iter().copied()).collect();
        split::files(&self.git(&args))
    }

    pub(crate) async fn open(&self) -> (Client, MrKey) {
        let client = Client::open(self.0.path(), None).await.unwrap();
        let key = client.here_key().await.unwrap();
        (client, key)
    }

    pub(crate) async fn open_on(&self, target: &str) -> (Client, MrKey) {
        let client = Client::open(self.0.path(), Some(target)).await.unwrap();
        let key = client.here_key().await.unwrap();
        (client, key)
    }

    async fn diffs(&self) -> Vec<DiffFile> {
        let (client, key) = self.open().await;
        client.diffs(&key).await.unwrap()
    }
}

#[test]
fn a_rename_keeps_both_paths_and_its_edit() {
    let repo = Repo::new();
    repo.write("a.txt", "one\ntwo\nthree\n");
    repo.git(&["mv", "a.txt", "b.txt"]);
    repo.commit("rename");
    let [file] = repo.parsed(&["main", "HEAD"]).try_into().unwrap();
    assert_eq!((file.old_path.as_str(), file.new_path.as_str(), file.change), ("a.txt", "b.txt", FileKind::Renamed));
    assert!(file.diff.starts_with("@@ ") && file.diff.contains("+three"), "{}", file.diff);
}

#[test]
fn a_space_in_a_path_stays_in_the_path() {
    let repo = Repo::new();
    repo.write("my notes.txt", "hi\n");
    repo.commit("add");
    let [file] = repo.parsed(&["main", "HEAD"]).try_into().unwrap();
    assert_eq!((file.new_path.as_str(), file.change), ("my notes.txt", FileKind::Added));
}

#[test]
fn a_binary_file_has_no_body_and_the_file_after_it_keeps_its_own() {
    let repo = Repo::new();
    std::fs::write(repo.path().join("logo.png"), [0_u8, 1, 2]).unwrap();
    repo.write("z.txt", "after\n");
    repo.commit("add");
    let files = repo.parsed(&["main", "HEAD"]);
    assert_eq!((files[0].new_path.as_str(), files[0].diff.as_str(), files[0].change), ("logo.png", "", FileKind::Added));
    assert!(files[1].diff.contains("+after"), "{}", files[1].diff);
}

#[test]
fn a_mode_change_alone_is_a_mode_change() {
    use std::os::unix::fs::PermissionsExt;
    let repo = Repo::new();
    std::fs::set_permissions(repo.path().join("a.txt"), std::fs::Permissions::from_mode(0o755)).unwrap();
    repo.commit("chmod");
    let [file] = repo.parsed(&["main", "HEAD"]).try_into().unwrap();
    assert_eq!((file.diff.as_str(), file.change), ("", FileKind::Mode));
}

#[test]
fn a_file_turned_symlink_is_one_file_and_the_next_keeps_its_own_body() {
    let repo = Repo::new();
    std::fs::remove_file(repo.path().join("a.txt")).unwrap();
    std::os::unix::fs::symlink("z.txt", repo.path().join("a.txt")).unwrap();
    repo.write("z.txt", "zz\n");
    repo.commit("link");
    let files = repo.parsed(&["main", "HEAD"]);
    assert_eq!(files.iter().map(|file| file.new_path.as_str()).collect::<Vec<_>>(), ["a.txt", "z.txt"]);
    assert!(files[0].diff.contains("-one") && files[0].diff.contains("+z.txt"), "{}", files[0].diff);
    assert!(files[1].diff.contains("+zz") && !files[1].diff.contains("-one"), "{}", files[1].diff);
}

#[test]
fn a_conflicted_file_breaks_no_other_file() {
    let repo = Repo::new();
    repo.write("a.txt", "one\nmine\n");
    repo.commit("mine");
    repo.git(&["switch", "-q", "main"]);
    repo.write("a.txt", "one\ntheirs\n");
    repo.write("b.txt", "b\n");
    repo.commit("theirs");
    repo.git(&["switch", "-q", "work"]);
    assert!(!repo.command(&["merge", "-q", "main"]).status.success());
    let files = repo.parsed(&["HEAD"]);
    assert!(files.iter().any(|file| file.new_path == "b.txt" && file.diff.contains("+b\n")), "{files:#?}");
    assert!(files.iter().all(|file| !file.diff.contains("Unmerged")), "{files:#?}");
}

#[tokio::test]
async fn untracked_work_shows_only_once_asked_and_is_counted_until_then() {
    let repo = Repo::new();
    repo.write("new file.txt", "fresh\n");
    let (client, key) = repo.open().await;
    assert_eq!(client.diffs(&key).await.unwrap(), vec![]);
    assert_eq!(client.mr(&key).await.unwrap().uncommitted, 1);
    client.toggle_uncommitted();
    let [file] = client.diffs(&key).await.unwrap().try_into().unwrap();
    assert_eq!((file.new_path.as_str(), file.change), ("new file.txt", FileKind::Added));
    assert!(file.diff.contains("+fresh"), "{}", file.diff);
    assert_eq!(client.mr(&key).await.unwrap().uncommitted, 0, "nothing is hidden now");
}

#[tokio::test]
async fn staged_and_unstaged_work_show_once_asked() {
    let repo = Repo::new();
    repo.write("a.txt", "one\nunstaged\n");
    repo.write("s.txt", "staged\n");
    repo.git(&["add", "s.txt"]);
    let (client, key) = repo.open().await;
    assert_eq!(client.mr(&key).await.unwrap().uncommitted, 2);
    client.toggle_uncommitted();
    let paths: Vec<String> = client.diffs(&key).await.unwrap().into_iter().map(|file| file.new_path).collect();
    assert_eq!(paths, ["a.txt", "s.txt"]);
}

#[tokio::test]
async fn the_diff_starts_at_the_merge_base_after_the_target_moved() {
    let repo = Repo::new();
    repo.write("mine.txt", "mine\n");
    repo.commit("mine");
    repo.git(&["switch", "-q", "main"]);
    repo.write("theirs.txt", "theirs\n");
    repo.commit("theirs");
    repo.git(&["switch", "-q", "work"]);
    let [file] = repo.diffs().await.try_into().unwrap();
    assert_eq!(file.new_path, "mine.txt");
}

#[tokio::test]
async fn a_bad_target_changes_nothing() {
    let repo = Repo::new();
    let (client, key) = repo.open().await;
    let refused = client.set_target("nope").await.unwrap_err();
    assert!(refused.to_string().contains("nope"), "{refused}");
    assert_eq!(client.mr(&key).await.unwrap().target_branch, "main");
}

#[tokio::test]
async fn a_target_that_reads_as_a_flag_is_refused_before_git_sees_it() {
    let repo = Repo::new();
    let (client, _) = repo.open().await;
    for flag in ["--upload-pack=x", "-n", ""] {
        assert!(client.set_target(flag).await.unwrap_err().to_string().contains("is not a branch"), "{flag:?}");
    }
    assert!(Client::open(repo.0.path(), Some("--all")).await.is_err());
}

#[tokio::test]
async fn the_default_target_needs_a_branch_to_name() {
    let repo = Repo::new();
    repo.git(&["branch", "-q", "-m", "main", "trunk"]);
    let refused = Client::open(repo.0.path(), None).await.unwrap_err();
    assert!(refused.to_string().contains("revu local <target>"), "{refused}");
    assert!(Client::open(repo.0.path(), Some("trunk")).await.is_ok());
}

#[tokio::test]
async fn a_worktree_with_no_merge_base_opens_empty_and_says_why() {
    let repo = Repo::new();
    repo.git(&["switch", "-q", "--orphan", "island"]);
    repo.write("island.txt", "x\n");
    repo.commit("island");
    let (client, key) = repo.open_on("main").await;
    let mr = client.mr(&key).await.unwrap();
    assert_eq!(mr.no_diff.as_deref(), Some("main shares no history with this branch"));
    assert_eq!(client.diffs(&key).await.unwrap(), vec![]);
}

#[tokio::test]
async fn a_worktree_sharing_history_gives_no_reason() {
    let repo = Repo::new();
    let (client, key) = repo.open().await;
    assert_eq!(client.mr(&key).await.unwrap().no_diff, None);
}

#[tokio::test]
async fn the_head_is_the_real_one_until_uncommitted_work_shows() {
    let repo = Repo::new();
    let (client, key) = repo.open().await;
    let real = repo.git(&["rev-parse", "HEAD"]);
    let head = || async { client.mr(&key).await.unwrap().refs.head };
    assert_eq!(head().await.as_str(), real.trim());
    client.toggle_uncommitted();
    let empty = head().await;
    assert_ne!(empty.as_str(), real.trim());
    assert_eq!(head().await, empty, "nothing moved");
    std::fs::write(repo.path().join("logo.png"), [0_u8, 1, 2]).unwrap();
    let added = head().await;
    assert_ne!(added, empty);
    std::fs::write(repo.path().join("logo.png"), [0_u8, 9, 9]).unwrap();
    assert_ne!(head().await, added, "an untracked binary moves it too");
}

#[tokio::test]
async fn a_file_comes_from_its_commit_or_from_the_disk_at_the_uncommitted_head() {
    let repo = Repo::new();
    let (client, key) = repo.open().await;
    let head = client.mr(&key).await.unwrap().refs.head;
    repo.write("a.txt", "on disk\n");
    assert_eq!(client.file(&key, "a.txt", &head).await.unwrap(), "one\ntwo\n");
    client.toggle_uncommitted();
    let shown = client.mr(&key).await.unwrap().refs.head;
    assert_eq!(client.file(&key, "a.txt", &shown).await.unwrap(), "on disk\n");
    assert!(client.file(&key, "../secret", &shown).await.unwrap_err().to_string().contains("does not trust"));
}

#[tokio::test]
async fn the_real_file_is_the_worktrees_at_any_head_but_not_at_the_base() {
    let repo = Repo::new();
    let linked = repo.path().join("linked");
    repo.git(&["worktree", "add", "-q", "-b", "other", linked.to_str().unwrap()]);
    std::fs::write(linked.join("b.txt"), "b\n").unwrap();
    let (client, _) = repo.open().await;
    let there = Client::open(&linked, None).await.unwrap().here_key().await.unwrap();
    let Refs { base, head, .. } = client.mr(&there).await.unwrap().refs;
    let parent = repo.git(&["rev-parse", "HEAD"]);
    assert_eq!(head.as_str(), parent.trim());
    repo.git(&["-C", linked.to_str().unwrap(), "commit", "-q", "--allow-empty", "-m", "ahead"]);
    let moved = client.mr(&there).await.unwrap().refs.head;
    assert_eq!(client.real_file(&there, "b.txt", &moved).await.unwrap(), Some(linked.join("b.txt")));
    assert_eq!(client.real_file(&there, "a.txt", &base).await.unwrap(), None);
    client.toggle_uncommitted();
    let shown = client.mr(&there).await.unwrap().refs.head;
    assert_eq!(client.real_file(&there, "b.txt", &shown).await.unwrap(), Some(linked.join("b.txt")));
}

#[tokio::test]
async fn with_uncommitted_work_shown_the_base_comes_from_git_even_when_it_is_the_head() {
    let repo = Repo::new();
    repo.write("a.txt", "edited\n");
    let (client, key) = repo.open().await;
    client.toggle_uncommitted();
    let Refs { base, head, .. } = client.mr(&key).await.unwrap().refs;
    assert_eq!(client.real_file(&key, "a.txt", &base).await.unwrap(), None, "the old side is not the edited file");
    assert_eq!(client.real_file(&key, "a.txt", &head).await.unwrap(), Some(repo.path().join("a.txt")));
}

#[tokio::test]
async fn the_key_finds_a_linked_worktree_on_a_detached_head() {
    let repo = Repo::new();
    let linked = repo.path().join("linked tree");
    repo.git(&["worktree", "add", "-q", "--detach", linked.to_str().unwrap(), "main"]);
    std::fs::write(linked.join("only-there.txt"), "x\n").unwrap();
    let client = Client::open(&linked, None).await.unwrap();
    let key = client.here_key().await.unwrap();
    assert_eq!((key.host.as_deref(), key.project.as_str()), (Some("local"), "local/linked-tree"));
    let mr = client.mr(&key).await.unwrap();
    assert_eq!(mr.source_branch, repo.git(&["rev-parse", "main"])[..7]);
    assert!(mr.title.ends_with(&format!(" ~ {}", mr.source_branch)), "{}", mr.title);
    assert_eq!(mr.uncommitted, 1);
    assert_eq!(client.here_key().await.unwrap(), key, "the same number every time");
}

#[tokio::test]
async fn the_queue_sorts_worktrees_here_ahead_and_the_rest_with_their_counts() {
    let repo = Repo::new();
    repo.write("mine.txt", "one\ntwo\n");
    repo.commit("mine");
    repo.write("dirty.txt", "x\n");
    let (behind, ahead) = (repo.path().join("behind"), repo.path().join("ahead"));
    repo.git(&["worktree", "add", "-q", "-b", "same", behind.to_str().unwrap(), "main"]);
    repo.git(&["worktree", "add", "-q", "-b", "further", ahead.to_str().unwrap(), "main"]);
    repo.git(&["-C", ahead.to_str().unwrap(), "commit", "-q", "--allow-empty", "-m", "ahead"]);
    let (client, _) = repo.open().await;
    let Fetched { queue, .. } = client.queue().await.unwrap();
    let branches = |rows: &[QueueMr]| rows.iter().map(|row| row.title.clone()).collect::<Vec<_>>();
    assert_eq!(
        (branches(&queue.authored), branches(&queue.review_requested), branches(&queue.open)),
        (vec!["work".to_owned()], vec!["further".to_owned()], vec!["same".to_owned()])
    );
    assert_eq!(queue.me, repo.path().file_name().unwrap().to_str().unwrap());
    let here = &queue.authored[0];
    assert_eq!(here.worktree, Some(WorktreeRow { commits: 1, uncommitted: 3, no_diff: None }), "the two worktree folders and dirty.txt");
    assert_eq!(here.size.map(|size| (size.additions, size.files)), Some((2, 1)));
}

#[tokio::test]
async fn a_worktree_with_no_merge_base_keeps_its_row_and_the_others_stay() {
    let repo = Repo::new();
    let island = repo.path().join("island");
    repo.git(&["worktree", "add", "-q", "-b", "island", island.to_str().unwrap(), "main"]);
    repo.git(&["-C", island.to_str().unwrap(), "switch", "-q", "--orphan", "alone"]);
    repo.git(&["-C", island.to_str().unwrap(), "commit", "-q", "--allow-empty", "-m", "alone"]);
    let (client, _) = repo.open().await;
    let Fetched { queue, .. } = client.queue().await.unwrap();
    assert_eq!(queue.authored[0].title, "work");
    let [row] = queue.open.try_into().unwrap();
    assert_eq!(row.title, "alone");
    assert_eq!(row.worktree.unwrap().no_diff.as_deref(), Some("main shares no history with this branch"));
}

#[tokio::test]
async fn a_locked_worktree_with_its_folder_gone_keeps_its_row_and_opens_saying_why() {
    let repo = Repo::new();
    let gone = repo.path().join("gone");
    repo.git(&["worktree", "add", "-q", "-b", "gone", gone.to_str().unwrap(), "main"]);
    repo.git(&["worktree", "lock", gone.to_str().unwrap()]);
    std::fs::remove_dir_all(&gone).unwrap();
    let (client, _) = repo.open().await;
    let Fetched { queue, .. } = client.queue().await.unwrap();
    assert_eq!(queue.authored[0].title, "work");
    let [row] = queue.open.try_into().unwrap();
    assert!(row.worktree.as_ref().unwrap().no_diff.is_some(), "{row:?}");
    assert!(client.mr(&row.key()).await.unwrap().no_diff.is_some());
}

#[tokio::test]
async fn an_unborn_worktree_keeps_its_row_and_revu_can_start_there() {
    let repo = Repo::new();
    let unborn = repo.path().join("unborn");
    repo.git(&["worktree", "add", "-q", "--orphan", "-b", "unborn", unborn.to_str().unwrap()]);
    let client = Client::open(&unborn, None).await.unwrap();
    let key = client.here_key().await.unwrap();
    assert!(client.mr(&key).await.unwrap().no_diff.is_some());
    let Fetched { queue, .. } = client.queue().await.unwrap();
    assert_eq!(queue.authored[0].title, "unborn");
    assert_eq!(queue.open.len(), 1, "the other worktree stays");
}

#[test]
fn a_worktree_list_keeps_an_unborn_entry_of_either_hash() {
    let zeros = ["0".repeat(40), "0".repeat(64)];
    let listing = git::parse_listing(&format!(
        "worktree /w/a\0HEAD {}\0branch refs/heads/a\0\0worktree /w/b\0HEAD {}\0branch refs/heads/b\0\0",
        zeros[0], zeros[1]
    ));
    assert_eq!(listing.worktrees.iter().map(Worktree::label).collect::<Vec<_>>(), ["a", "b"]);
}

#[tokio::test]
async fn revu_can_start_in_a_submodule_and_reads_its_own_branch() {
    let (inner, outer) = (Repo::new(), Repo::new());
    outer.git(&["-c", "protocol.file.allow=always", "submodule", "add", "-q", inner.path().to_str().unwrap(), "sub"]);
    let sub = outer.path().join("sub");
    let client = Client::open(&sub, Some("HEAD")).await.unwrap();
    let key = client.here_key().await.unwrap();
    let mr = client.mr(&key).await.unwrap();
    assert_eq!((key.project.as_str(), mr.no_diff), ("local/sub", None));
    let Fetched { queue, .. } = client.queue().await.unwrap();
    assert_eq!((queue.authored.len(), queue.review_requested.len(), queue.open.len()), (1, 0, 0), "only the submodule's own folder");
}

#[tokio::test]
async fn a_linked_worktree_of_a_submodule_lists_no_git_folder() {
    let (inner, outer) = (Repo::new(), Repo::new());
    outer.git(&["-c", "protocol.file.allow=always", "submodule", "add", "-q", inner.path().to_str().unwrap(), "sub"]);
    let linked = outer.path().join("linked");
    outer.git(&["-C", outer.path().join("sub").to_str().unwrap(), "worktree", "add", "-q", "-b", "lk", linked.to_str().unwrap()]);
    let client = Client::open(&linked, Some("HEAD")).await.unwrap();
    let Fetched { queue, .. } = client.queue().await.unwrap();
    let titles: Vec<&str> = queue.authored.iter().chain(&queue.review_requested).chain(&queue.open).map(|row| row.title.as_str()).collect();
    assert_eq!(titles, ["lk"]);
}

#[tokio::test]
async fn a_broken_index_loses_only_the_uncommitted_count() {
    let repo = Repo::new();
    repo.write("b.txt", "new\n");
    repo.commit("work");
    let (client, key) = repo.open().await;
    std::fs::write(repo.path().join(".git/index"), "garbage").unwrap();
    let mr = client.mr(&key).await.unwrap();
    assert_eq!((mr.uncommitted, mr.no_diff), (0, None));
    assert_eq!(client.diffs(&key).await.unwrap().len(), 1);
    client.toggle_uncommitted();
    assert!(client.mr(&key).await.is_err(), "uncommitted work cannot be shown");
}

#[tokio::test]
async fn a_target_gone_a_moment_is_a_failure_with_its_reason_and_the_diff_comes_back() {
    let repo = Repo::new();
    repo.write("mine.txt", "mine\n");
    repo.commit("mine");
    repo.git(&["branch", "dev", "main"]);
    let (client, key) = repo.open_on("dev").await;
    let before = client.mr(&key).await.unwrap();
    repo.git(&["branch", "-m", "dev", "tmp"]);
    assert_eq!(client.mr(&key).await.unwrap_err().to_string(), "dev is not a commit here");
    assert!(client.diffs(&key).await.is_err());
    let Fetched { queue, .. } = client.queue().await.unwrap();
    assert_eq!(queue.authored[0].worktree.as_ref().unwrap().no_diff.as_deref(), Some("dev is not a commit here"));
    repo.git(&["branch", "-m", "tmp", "dev"]);
    assert_eq!(client.mr(&key).await.unwrap().refs, before.refs);
    assert_eq!(client.diffs(&key).await.unwrap().len(), 1);
}

#[test]
fn a_huge_file_withholds_its_body() {
    let body = "+x\n".repeat(20_001);
    let record = ":000000 100644 0000000 abcdef0 A\0big.txt\0\0";
    let [file] = split::files(&format!("{record}diff --git a/big.txt b/big.txt\n@@ -0,0 +1,20001 @@\n{body}")).try_into().unwrap();
    assert!(file.too_large && file.diff.is_empty());
}

#[test]
fn a_worktree_list_leaves_out_bare_and_prunable_entries() {
    let sha = "abcdef1234567890abcdef1234567890abcdef12";
    let listing = git::parse_listing(&format!(
        "worktree /w/widgets.git\0bare\0\0worktree /w/a\0HEAD {sha}\0branch refs/heads/feat/x\0\0\
         worktree /w/b\0HEAD {sha}\0detached\0\0worktree /w/c\0HEAD {sha}\0branch refs/heads/old\0prunable gone\0\0"
    ));
    assert_eq!(listing.repo, "widgets");
    assert_eq!(listing.worktrees.iter().map(Worktree::label).collect::<Vec<_>>(), ["feat/x", "abcdef1"]);
}
