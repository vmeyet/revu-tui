//! The tool was called `gitlabmr` before `revu`: its config, cache and keychain entries move over once.
use crate::auth::{SecretStore, SecurityCli};
use anyhow::Result;
use std::path::Path;

const OLD_NAME: &str = "gitlabmr";

/// Moves what earlier versions left elsewhere, once: the config from the OS folder (under either
/// name) to `~/.config/revu`, the cache from its old name. Never overwrites: when both exist,
/// the new one wins and the old one stays for the user to delete.
pub fn move_dirs() {
    let config = crate::config::dir();
    for old in dirs::config_dir().into_iter().flat_map(|os| [os.join(crate::auth::SERVICE), os.join(OLD_NAME)]) {
        let _ = move_dir(&old, &config);
    }
    if let Some(cache) = dirs::cache_dir() {
        let _ = move_dir(&cache.join(OLD_NAME), &cache.join(crate::auth::SERVICE));
    }
}

fn move_dir(old: &Path, new: &Path) -> std::io::Result<()> {
    if old == new || !old.is_dir() || new.exists() {
        return Ok(());
    }
    if let Some(parent) = new.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(old, new)
}

/// The keychain under the current name, falling back to the old one: a token found there is
/// copied over on first read, so nobody has to log in again after the rename.
pub struct Keychain<S, L> {
    current: S,
    old: L,
}

impl Keychain<SecurityCli, SecurityCli> {
    pub fn open() -> Self {
        Self { current: SecurityCli::new(crate::auth::SERVICE), old: SecurityCli::new(OLD_NAME) }
    }
}

impl<S: SecretStore, L: SecretStore> SecretStore for Keychain<S, L> {
    fn get(&self, account: &str) -> Result<Option<String>> {
        if let Some(secret) = self.current.get(account)? {
            return Ok(Some(secret));
        }
        let Some(secret) = self.old.get(account)? else { return Ok(None) };
        self.current.set(account, &secret)?;
        self.old.delete(account)?;
        Ok(Some(secret))
    }

    fn set(&self, account: &str, secret: &str) -> Result<()> {
        self.current.set(account, secret)
    }

    fn delete(&self, account: &str) -> Result<()> {
        self.current.delete(account)?;
        self.old.delete(account)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::auth::MemoryStore;

    fn keychain() -> Keychain<MemoryStore, MemoryStore> {
        Keychain { current: MemoryStore::default(), old: MemoryStore::default() }
    }

    #[test]
    fn an_old_token_moves_on_first_read_and_only_once() {
        let k = keychain();
        k.old.set("gitlab.com", "glpat-old").unwrap();
        assert_eq!(k.get("gitlab.com").unwrap().as_deref(), Some("glpat-old"));
        assert_eq!(k.current.get("gitlab.com").unwrap().as_deref(), Some("glpat-old"));
        assert_eq!(k.old.get("gitlab.com").unwrap(), None);
        assert_eq!(k.get("gitlab.com").unwrap().as_deref(), Some("glpat-old"));
    }

    #[test]
    fn the_current_name_wins_and_logout_clears_both() {
        let k = keychain();
        k.old.set("gitlab.com", "glpat-old").unwrap();
        k.set("gitlab.com", "glpat-new").unwrap();
        assert_eq!(k.get("gitlab.com").unwrap().as_deref(), Some("glpat-new"));
        k.delete("gitlab.com").unwrap();
        assert_eq!((k.current.get("gitlab.com").unwrap(), k.old.get("gitlab.com").unwrap()), (None, None));
    }

    #[test]
    fn directories_move_once_and_never_overwrite() {
        let root = tempfile::tempdir().unwrap();
        let (old, new) = (root.path().join("gitlabmr"), root.path().join("revu"));
        std::fs::create_dir(&old).unwrap();
        std::fs::write(old.join("config.toml"), "host = \"gitlab.com\"").unwrap();
        move_dir(&old, &new).unwrap();
        assert!(new.join("config.toml").exists() && !old.exists());
        std::fs::create_dir(&old).unwrap();
        move_dir(&old, &new).unwrap();
        assert!(old.exists(), "an old directory next to a new one is left alone");
    }

    #[test]
    fn a_directory_moves_into_a_parent_that_does_not_exist_yet() {
        let root = tempfile::tempdir().unwrap();
        let old = root.path().join("Library/Application Support/revu");
        let new = root.path().join(".config/revu");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("config.toml"), "").unwrap();
        move_dir(&old, &new).unwrap();
        assert!(new.join("config.toml").exists() && !old.exists());
        move_dir(&new, &new).unwrap();
        assert!(new.exists(), "moving a directory onto itself is a no-op");
    }
}
