use super::Config;
use anyhow::{Context, Result};
use std::path::Path;
use toml_edit::{DocumentMut, Item, Table, Value};

impl Config {
    pub(crate) fn save(&self) -> Result<()> {
        self.save_to(&Self::path())
    }

    /// Rewrites only the keys that changed since the file was read, so the user's comments and layout
    /// stay; a crash mid-write leaves the old file whole.
    fn save_to(&self, path: &Path) -> Result<()> {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
        let text = match std::fs::read_to_string(&path) {
            Ok(kept) => self.written_over(&kept)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => toml::to_string_pretty(self)?,
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        write_whole(&path, &text)
    }

    fn written_over(&self, kept: &str) -> Result<String> {
        let file: DocumentMut = kept.parse()?;
        let before: DocumentMut = toml::to_string_pretty(&toml::from_str::<Self>(kept)?)?.parse()?;
        let after: DocumentMut = toml::to_string_pretty(self)?.parse()?;
        let mut edited = DocumentMut::from(patched(file.as_table(), before.as_table(), after.as_table()));
        edited.set_trailing(file.trailing().clone());
        Ok(edited.to_string())
    }
}

/// `file` with what changed from `before` to `after` written over it; every other key keeps its text.
fn patched(file: &Table, before: &Table, after: &Table) -> Table {
    let mut table = file.clone();
    table.retain(|key, _| after.contains_key(key) || !before.contains_key(key));
    for (key, item) in after {
        let old = before.get(key);
        if old.is_some_and(|old| rendered(key, old) == rendered(key, item)) {
            continue;
        }
        let new = match (table.get(key), item) {
            (Some(Item::Table(kept)), Item::Table(item)) => {
                Item::Table(patched(kept, old.and_then(Item::as_table).unwrap_or(&Table::new()), item))
            }
            (Some(Item::Value(kept)), Item::Value(value)) => Item::Value(with_decor(value, kept)),
            _ => item.clone(),
        };
        match table.get_mut(key) {
            Some(slot) => *slot = new,
            None => drop(table.insert(key, new)),
        }
    }
    table
}

/// An item as the file would show it, sub-tables included, which `Item`'s own `Display` leaves out.
fn rendered(key: &str, item: &Item) -> String {
    let mut table = Table::new();
    table.insert(key, item.clone());
    DocumentMut::from(table).to_string()
}

fn with_decor(value: &Value, kept: &Value) -> Value {
    let mut value = value.clone();
    *value.decor_mut() = kept.decor().clone();
    value
}

/// Through a temp file and a rename, so the file is always either the old one or the new one.
fn write_whole(path: &Path, text: &str) -> Result<()> {
    let dir = path.parent().context("the config path has no folder")?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let temp = tempfile::Builder::new().prefix(".tmp-").tempfile_in(dir).context("creating a temp file")?;
    std::fs::write(temp.path(), text)?;
    temp.persist(path).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Host;
    use std::path::PathBuf;

    const TYPED: &str = "# my revu setup\nhost = \"gitlab.com\" # work\nusername = \"nina\"\n\n[queue]\nwatch_labels = [ \"infra\" ] # ops\n\n# colours\n[tui]\ntheme = 'nord' # cold\nascii = false\n\n[notify]\nenabled = true # said twice on purpose\n\n[hosts.\"git.acme.dev\"]\nforge = \"github\"\n# the end\n";

    fn typed_config() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, TYPED).unwrap();
        (dir, path)
    }

    #[test]
    fn saving_rewrites_only_the_changed_key_and_keeps_the_comments() {
        let (dir, path) = typed_config();
        let mut config = Config::load_from(&path).unwrap();
        config.tui.theme = Some("dracula".into());
        config.save_to(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, TYPED.replace("'nord'", "\"dracula\""));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "no temp file is left behind");
    }

    #[test]
    fn a_key_set_to_none_is_dropped_and_a_new_table_lands_at_the_end() {
        let (_dir, path) = typed_config();
        let mut config = Config::load_from(&path).unwrap();
        config.username = None;
        config.hosts.insert("gl.acme.dev".into(), Host { username: Some("nina".into()), ..Host::default() });
        config.save_to(&path).unwrap();
        let expected =
            TYPED.replace("username = \"nina\"\n", "").replace("# the end", "\n[hosts.\"gl.acme.dev\"]\nusername = \"nina\"\n# the end");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
        assert_eq!(Config::load_from(&path).unwrap(), config);
    }

    #[test]
    fn saving_twice_changes_nothing_the_second_time() {
        let (_dir, path) = typed_config();
        let mut config = Config::load_from(&path).unwrap();
        config.tui.theme = Some("dracula".into());
        config.host = None;
        config.save_to(&path).unwrap();
        let once = std::fs::read_to_string(&path).unwrap();
        config.save_to(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), once);
    }

    #[test]
    fn a_first_save_creates_the_folder_and_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("revu/config.toml");
        let config = Config { host: Some("gitlab.com".into()), ..Config::default() };
        config.save_to(&path).unwrap();
        assert_eq!(Config::load_from(&path).unwrap(), config);
    }

    #[test]
    fn saving_keeps_the_keys_this_revu_ignores() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[tui]\ntheme = \"nord\"\nsparkles = true\n\n[sync]\nevery = 5\n").unwrap();
        let mut config = Config::load_from(&path).unwrap();
        config.tui.theme = Some("dracula".into());
        config.save_to(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, "[tui]\ntheme = \"dracula\"\nsparkles = true\n\n[sync]\nevery = 5\n");
    }
}
