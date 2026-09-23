//! Cached answers under `~/.cache/revu/<host>/`, one JSON file per key. The files hold MR
//! content, so directories are 0700 and files 0600; writes go through a temp name then a rename.
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

const DIR_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;

#[derive(Clone, Debug)]
pub struct Cache {
    dir: PathBuf,
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
    pub fn for_host(host: &str) -> Self {
        Self::in_dir(root().join(host))
    }

    pub fn in_dir(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
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
        let path = self.dir.join(key);
        let parent = path.parent().context("cache key has no parent")?;
        create_private_dirs(&self.dir, parent)?;
        let temp = tempfile::Builder::new().prefix(".tmp-").tempfile_in(parent).context("creating a temp file")?;
        std::fs::set_permissions(temp.path(), private(FILE_MODE))?;
        std::fs::write(temp.path(), serde_json::to_vec(value)?)?;
        temp.persist(&path).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    pub fn read_entry<T: DeserializeOwned>(&self, key: &str) -> Option<Entry<T>> {
        self.read(key)
    }

    pub fn write_entry<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.write(key, &Entry::now(value))
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

    /// The directory of one MR: its project path with `+` for `/`, then its number.
    fn dir(key: &MrKey) -> String {
        format!("mr/{}/{}", slug(&key.project), key.number)
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
