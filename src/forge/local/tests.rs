use super::split;
use crate::forge::{DiffFile, FileKind};
use std::path::PathBuf;
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

    fn git(&self, args: &[&str]) -> String {
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

#[test]
fn a_huge_file_withholds_its_body() {
    let body = "+x\n".repeat(20_001);
    let record = ":000000 100644 0000000 abcdef0 A\0big.txt\0\0";
    let [file] = split::files(&format!("{record}diff --git a/big.txt b/big.txt\n@@ -0,0 +1,20001 @@\n{body}")).try_into().unwrap();
    assert!(file.too_large && file.diff.is_empty());
}
