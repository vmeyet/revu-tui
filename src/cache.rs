//! Cached answers under `~/.cache/revu/<host>/`, one JSON file per key. The files hold MR
//! content, so directories are 0700 and files 0600; writes go through a temp name then a rename.
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const DIR_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;

#[derive(Clone, Debug)]
pub struct Cache {
    dir: PathBuf,
}

/// An MR the cache holds a folder for, and when anything in it was last written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kept {
    pub project: String,
    pub number: u64,
    pub touched: SystemTime,
}

/// The kept MRs to drop: those the forge says are merged or closed, and any untouched for `idle`,
/// which also covers an MR on a project the forge could not answer for.
pub fn to_forget<'a>(kept: &'a [Kept], finished: &HashSet<(String, u64)>, now: SystemTime, idle: Duration) -> Vec<&'a Kept> {
    let old = |k: &Kept| now.duration_since(k.touched).is_ok_and(|age| age > idle);
    kept.iter().filter(|k| finished.contains(&(k.project.clone(), k.number)) || old(k)).collect()
}

/// When anything under `path` was last written, looking one folder deep as the MR layout is.
fn newest_write(path: &Path) -> Option<SystemTime> {
    std::fs::read_dir(path)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            if meta.is_dir() { newest_write(&entry.path()) } else { meta.modified().ok() }
        })
        .max()
}

/// A cached value and when it was fetched, so a stale view can say how stale.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry<T> {
    pub value: T,
    pub fetched_at: DateTime<Utc>,
}

impl<T> Entry<T> {
    pub fn now(value: T) -> Self {
        Self { value, fetched_at: Utc::now() }
    }

    pub fn age(&self, now: DateTime<Utc>) -> Duration {
        (now - self.fetched_at).to_std().unwrap_or(Duration::ZERO)
    }
}

impl Cache {
    pub fn for_host(host: &str) -> Result<Self> {
        crate::auth::check_host(host)?;
        Ok(Self::in_dir(root().join(host)))
    }

    pub fn in_dir(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The cache root itself, for what belongs to no host.
    pub fn shared() -> Self {
        Self::in_dir(root())
    }

    /// A folder for files that are not JSON, created on first use.
    pub fn folder(&self, name: &str) -> Result<PathBuf> {
        let path = self.dir.join(name);
        std::fs::create_dir_all(&path).with_context(|| format!("creating {}", path.display()))?;
        Ok(path)
    }

    /// Whether an entry is there, without reading it: a big diff is not parsed only to be dropped.
    pub fn has(&self, key: &str) -> bool {
        self.dir.join(key).is_file()
    }

    /// The entries of folder `dir` whose names start with `prefix`, as keys `read` takes; none
    /// when the folder is not there.
    pub fn keys_in(&self, dir: &str, prefix: &str) -> Vec<String> {
        let Ok(listed) = std::fs::read_dir(self.dir.join(dir)) else { return vec![] };
        let names = listed.filter_map(|entry| entry.ok()?.file_name().into_string().ok());
        names.filter(|name| name.starts_with(prefix)).map(|name| format!("{dir}/{name}")).collect()
    }

    /// Every MR this cache holds a folder for, with when anything in it was last written.
    pub fn kept_mrs(&self) -> Vec<Kept> {
        let mrs = self.dir.join("mr");
        let Ok(projects) = std::fs::read_dir(&mrs) else { return vec![] };
        let mut kept = vec![];
        for project in projects.filter_map(Result::ok) {
            let Some(slug) = project.file_name().to_str().map(str::to_owned) else { continue };
            let Ok(numbers) = std::fs::read_dir(project.path()) else { continue };
            for folder in numbers.filter_map(Result::ok) {
                let Some(number) = folder.file_name().to_str().and_then(|n| n.parse().ok()) else { continue };
                let touched = newest_write(&folder.path()).unwrap_or(SystemTime::UNIX_EPOCH);
                kept.push(Kept { project: slug.replace('+', "/"), number, touched });
            }
        }
        kept
    }

    /// Drops everything kept for one MR; a folder already gone is fine.
    pub fn forget_mr(&self, project: &str, number: u64) -> Result<()> {
        let path = self.dir.join(keys::mr_dir(project, number));
        match std::fs::remove_dir_all(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e).with_context(|| format!("removing {}", path.display())),
            _ => Ok(()),
        }
    }

    /// A missing or unreadable file is a miss; an unreadable one is removed so it cannot fail again.
    pub fn read<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let path = self.dir.join(key);
        let raw = std::fs::read(&path).ok()?;
        let parsed = serde_json::from_slice(&raw).ok();
        if parsed.is_none() {
            let _ = std::fs::remove_file(&path);
        }
        parsed
    }

    pub fn write<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.write_bytes(key, &serde_json::to_vec(value)?)
    }

    /// A file that is not JSON, such as a downloaded picture; a missing one is a miss.
    pub fn read_bytes(&self, key: &str) -> Option<Vec<u8>> {
        std::fs::read(self.dir.join(key)).ok()
    }

    pub fn write_bytes(&self, key: &str, bytes: &[u8]) -> Result<()> {
        let path = self.dir.join(key);
        let parent = path.parent().context("cache key has no parent")?;
        create_private_dirs(&self.dir, parent)?;
        let temp = tempfile::Builder::new().prefix(".tmp-").tempfile_in(parent).context("creating a temp file")?;
        std::fs::set_permissions(temp.path(), private(FILE_MODE))?;
        std::fs::write(temp.path(), bytes)?;
        temp.persist(&path).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    pub fn read_entry<T: DeserializeOwned>(&self, key: &str) -> Option<Entry<T>> {
        self.read(key)
    }

    pub fn write_entry<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.write(key, &Entry::now(value))
    }

    /// File `name` opened to append, private like the rest. Past `max` bytes it first moves to
    /// `<name>.1`, dropping the one before, so it never holds more than twice `max`.
    pub fn append(&self, name: &str, max: u64) -> Result<std::fs::File> {
        let path = self.dir.join(name);
        create_private_dirs(&self.dir, &self.dir)?;
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > max) {
            std::fs::rename(&path, self.dir.join(format!("{name}.1"))).with_context(|| format!("rolling {}", path.display()))?;
        }
        let file =
            std::fs::OpenOptions::new().create(true).append(true).open(&path).with_context(|| format!("opening {}", path.display()))?;
        std::fs::set_permissions(&path, private(FILE_MODE))?;
        Ok(file)
    }

    pub fn clear(&self) -> Result<()> {
        match std::fs::remove_dir_all(&self.dir) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| format!("removing {}", self.dir.display())),
        }
    }
}

/// Every directory from the cache root down to `leaf` is created private, whoever created the root's parents.
fn create_private_dirs(root: &Path, leaf: &Path) -> Result<()> {
    let relative = leaf.strip_prefix(root).unwrap_or(Path::new(""));
    let mut current = root.to_path_buf();
    for step in std::iter::once(Path::new("")).chain(relative.iter().map(Path::new)) {
        current = current.join(step);
        if !current.exists() {
            std::fs::create_dir_all(&current).with_context(|| format!("creating {}", current.display()))?;
        }
        std::fs::set_permissions(&current, private(DIR_MODE))?;
    }
    Ok(())
}

#[cfg(unix)]
fn private(mode: u32) -> std::fs::Permissions {
    use std::os::unix::fs::PermissionsExt;
    std::fs::Permissions::from_mode(mode)
}

#[cfg(not(unix))]
fn private(_mode: u32) -> std::fs::Permissions {
    std::fs::metadata(".").map(|m| m.permissions()).expect("current dir is readable")
}

fn root() -> PathBuf {
    std::env::var_os("REVU_CACHE_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::cache_dir().map(|d| d.join("revu")))
        .unwrap_or_else(|| PathBuf::from(".revu-cache"))
}

/// The one place cache paths are spelled.
pub mod keys {
    use crate::forge::MrKey;

    /// One file per scope, so switching between a repo and every project never shows the other list.
    pub fn queue(project: Option<&str>) -> String {
        match project {
            Some(path) => format!("queue.{}.json", slug(path)),
            None => "queue.json".into(),
        }
    }

    /// The ready command's last output for a scope, and the MRs it named outside my lists.
    pub fn ready(project: Option<&str>) -> String {
        match project {
            Some(path) => format!("ready.{}.json", slug(path)),
            None => "ready.json".into(),
        }
    }

    /// How the queue of one scope is sorted and grouped, next to its list.
    pub fn queue_view(project: Option<&str>) -> String {
        match project {
            Some(path) => format!("queue_view.{}.json", slug(path)),
            None => "queue_view.json".into(),
        }
    }

    /// The directory of one MR: its project path with `+` for `/`, then its number.
    fn dir(key: &MrKey) -> String {
        mr_dir(&key.project, key.number)
    }

    /// The folder everything about one MR sits in.
    pub fn mr_dir(project: &str, number: u64) -> String {
        format!("mr/{}/{number}", slug(project))
    }

    /// When the cache of every host was last cleaned, in the shared cache.
    pub fn pruned() -> String {
        "pruned.json".to_owned()
    }

    fn slug(project: &str) -> String {
        project.replace('/', "+")
    }

    pub fn mr(key: &MrKey) -> String {
        format!("{}/mr.json", dir(key))
    }

    pub fn diffs(key: &MrKey, head_sha: &str) -> String {
        format!("{}/diffs.{head_sha}.json", dir(key))
    }

    pub fn discussions(key: &MrKey) -> String {
        format!("{}/discussions.json", dir(key))
    }

    /// A downloaded picture, by the link a note gave; hashed, since the link holds slashes and queries.
    pub fn image(url: &str) -> String {
        format!("images/{}", sha1_smol::Sha1::from(url.as_bytes()).digest())
    }

    pub fn drafts(key: &MrKey) -> String {
        format!("{}/drafts.json", dir(key))
    }

    pub fn state(key: &MrKey) -> String {
        format!("{}/state.json", dir(key))
    }

    /// What Jev said about an MR as a queue row.
    pub fn verdict(key: &MrKey) -> String {
        format!("{}/ai/verdict.json", dir(key))
    }

    /// Claude's answer to one request, named by a hash of the request so the same question hits it.
    pub fn answer(key: &MrKey, request: &str) -> String {
        format!("{}/ai/answer.{}.json", dir(key), sha1_smol::Sha1::from(request.as_bytes()).digest())
    }

    /// Where every answer about an MR lies, and what their names start with.
    pub fn answers(key: &MrKey) -> (String, &'static str) {
        (format!("{}/ai", dir(key)), "answer.")
    }

    /// What Jev read in an MR at one head commit.
    pub fn reading(key: &MrKey, head: &str) -> String {
        format!("{}/ai/reading.{head}.json", dir(key))
    }

    /// A file at one commit, named by a hash of its path so no path from the forge becomes a directory.
    pub fn file(key: &MrKey, sha: &str, path: &str) -> String {
        format!("{}/files/{sha}/{}.json", dir(key), sha1_smol::Sha1::from(path.as_bytes()).digest())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::forge::MrKey;
    use chrono::TimeDelta;

    fn key() -> MrKey {
        MrKey::new("acme/widgets", 42)
    }

    fn cache() -> (tempfile::TempDir, Cache) {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::in_dir(dir.path().join("gitlab.com"));
        (dir, cache)
    }

    #[test]
    fn an_appended_file_is_private_and_rolls_over_once_past_its_cap() {
        use std::io::Write;
        let (_dir, cache) = cache();
        cache.append("revu.log", 4).unwrap().write_all(b"first").unwrap();
        cache.append("revu.log", 4).unwrap().write_all(b"second").unwrap();
        cache.append("revu.log", 100).unwrap().write_all(b" third").unwrap();
        assert_eq!(cache.read_bytes("revu.log.1").as_deref(), Some(&b"first"[..]));
        assert_eq!(cache.read_bytes("revu.log").as_deref(), Some(&b"second third"[..]));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(cache.dir.join("revu.log")).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn pictures_are_kept_as_private_bytes_under_a_hashed_key() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::in_dir(dir.path().join("host"));
        let key = keys::image("https://github.com/user-attachments/assets/1f?x=1");
        assert!(key.starts_with("images/") && !key.contains("github"), "{key}");
        assert_eq!(cache.read_bytes(&key), None);
        cache.write_bytes(&key, b"\x89PNG").unwrap();
        assert_eq!(cache.read_bytes(&key).as_deref(), Some(&b"\x89PNG"[..]));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("host").join(&key)).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn a_folder_is_created_once_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::in_dir(dir.path().join("root"));
        let first = cache.folder("cargo_target").unwrap();
        std::fs::write(first.join("kept"), "").unwrap();
        let second = cache.folder("cargo_target").unwrap();
        assert!(first.is_dir());
        assert_eq!(first, second);
        assert!(second.join("kept").exists(), "a second call keeps what the folder holds");
    }

    #[test]
    fn round_trip_under_a_nested_key() {
        let (_dir, cache) = cache();
        let key = keys::diffs(&key(), "abc");
        assert_eq!(cache.read::<Vec<u32>>(&key), None);
        cache.write(&key, &vec![1, 2]).unwrap();
        assert_eq!(cache.read::<Vec<u32>>(&key), Some(vec![1, 2]));
        cache.write(&key, &vec![3]).unwrap();
        assert_eq!(cache.read::<Vec<u32>>(&key), Some(vec![3]));
    }

    #[test]
    fn corrupt_file_is_a_miss_and_is_removed() {
        let (dir, cache) = cache();
        let path = dir.path().join("gitlab.com").join(keys::queue(None));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{nope").unwrap();
        assert_eq!(cache.read::<Vec<u32>>(&keys::queue(None)), None);
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn files_and_directories_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, cache) = cache();
        cache.write(&keys::mr(&key()), &"x").unwrap();
        let host = dir.path().join("gitlab.com");
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&host), DIR_MODE);
        assert_eq!(mode(&host.join("mr")), DIR_MODE);
        assert_eq!(mode(&host.join("mr/acme+widgets/42")), DIR_MODE);
        assert_eq!(mode(&host.join(keys::mr(&key()))), FILE_MODE);
    }

    #[test]
    fn write_leaves_no_temp_file_behind() {
        let (dir, cache) = cache();
        cache.write(&keys::state(&key()), &"x").unwrap();
        let names: Vec<String> = std::fs::read_dir(dir.path().join("gitlab.com/mr/acme+widgets/42"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec!["state.json"]);
    }

    #[test]
    fn keys_in_lists_one_folders_entries_by_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::in_dir(dir.path());
        cache.write("mr/a/1/ai/answer.1.json", &1).unwrap();
        cache.write("mr/a/1/ai/answer.2.json", &2).unwrap();
        cache.write("mr/a/1/ai/verdict.json", &3).unwrap();
        let mut found = cache.keys_in("mr/a/1/ai", "answer.");
        found.sort();
        assert_eq!(found, ["mr/a/1/ai/answer.1.json", "mr/a/1/ai/answer.2.json"]);
        assert!(cache.keys_in("mr/a/2/ai", "answer.").is_empty());
    }

    #[test]
    fn kept_mrs_are_found_by_folder_and_forgotten_twice_without_harm() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::in_dir(dir.path());
        cache.write("mr/acme+widgets/42/mr.json", &1).unwrap();
        cache.write("mr/acme+widgets/42/ai/answer.1.json", &2).unwrap();
        cache.write("mr/acme+widgets/41/mr.json", &3).unwrap();
        let mut kept = cache.kept_mrs();
        kept.sort_by_key(|k| k.number);
        assert_eq!(kept.iter().map(|k| (k.project.as_str(), k.number)).collect::<Vec<_>>(), [("acme/widgets", 41), ("acme/widgets", 42)]);
        cache.forget_mr("acme/widgets", 42).unwrap();
        cache.forget_mr("acme/widgets", 42).unwrap();
        assert_eq!(cache.kept_mrs().len(), 1);
    }

    #[test]
    fn merged_closed_and_long_untouched_mrs_are_forgotten() {
        let now = SystemTime::now();
        let day = Duration::from_secs(86_400);
        let kept = |number: u64, days: u64| Kept { project: "acme/widgets".into(), number, touched: now - day * days as u32 };
        let all = [kept(40, 2), kept(41, 2), kept(42, 45)];
        let finished: HashSet<(String, u64)> = [("acme/widgets".to_owned(), 40)].into();
        let gone: Vec<u64> = to_forget(&all, &finished, now, day * 30).into_iter().map(|k| k.number).collect();
        assert_eq!(gone, [40, 42], "41 is open and was read two days ago");
    }

    #[test]
    fn a_host_that_is_a_path_gets_no_cache() {
        assert!(Cache::for_host("..").is_err());
        assert!(Cache::for_host("/").is_err());
        assert!(Cache::for_host("gitlab.com").is_ok());
    }

    #[test]
    fn clear_is_idempotent() {
        let (_dir, cache) = cache();
        cache.write(&keys::queue(None), &"x").unwrap();
        cache.clear().unwrap();
        cache.clear().unwrap();
        assert_eq!(cache.read::<String>(&keys::queue(None)), None);
    }

    #[test]
    fn entries_remember_when_they_were_fetched() {
        let (_dir, cache) = cache();
        cache.write_entry(&keys::queue(None), &"x").unwrap();
        let entry = cache.read_entry::<String>(&keys::queue(None)).unwrap();
        assert_eq!(entry.value, "x");
        let later = entry.fetched_at + TimeDelta::minutes(3);
        assert_eq!(entry.age(later), Duration::from_secs(180));
        assert_eq!(entry.age(entry.fetched_at - TimeDelta::seconds(1)), Duration::ZERO);
    }

    #[test]
    fn keys_are_stable() {
        assert_eq!(keys::discussions(&key()), "mr/acme+widgets/42/discussions.json");
    }

    #[test]
    fn queue_keys_differ_per_scope() {
        assert_eq!(keys::queue(None), "queue.json");
        assert_eq!(keys::queue(Some("acme/sub/widgets")), "queue.acme+sub+widgets.json");
    }
}
