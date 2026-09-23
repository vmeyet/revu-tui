use crate::diff::words::InlineRule;
use crate::forge::Kind;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Queue::is_default")]
    pub queue: Queue,
    #[serde(default, skip_serializing_if = "Review::is_default")]
    pub review: Review,
    #[serde(default, skip_serializing_if = "Tui::is_default")]
    pub tui: Tui,
    #[serde(default, skip_serializing_if = "Ai::is_default")]
    pub ai: Ai,
    #[serde(default, skip_serializing_if = "Open::is_default")]
    pub open: Open,
    /// Per-host settings, for a host whose name does not say which forge it runs.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub hosts: BTreeMap<String, Host>,
}

/// `[hosts."git.acme.dev"] forge = "github"`: a GitHub Enterprise host, which the name alone cannot tell.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Host {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forge: Option<Kind>,
    /// Who I am there, from the last login.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

/// Which MRs the queue shows beyond the ones GitLab lists for me.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Queue {
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default)]
    pub projects: Vec<String>,
    /// MRs carrying one of these labels land in Watching.
    #[serde(default)]
    pub watch_labels: Vec<String>,
}

impl Queue {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// How a diff opens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Review {
    /// Glob patterns of files that open folded: lockfiles, snapshots, generated code.
    #[serde(default)]
    pub fold: Vec<String>,
    /// A changed pair reads as one row when each side changes at most this many words…
    #[serde(default = "inline_max_words")]
    pub inline_max_words: usize,
    /// …and both lines keep at least this share of their text, in percent.
    #[serde(default = "inline_min_same")]
    pub inline_min_same: u8,
}

impl Default for Review {
    fn default() -> Self {
        Self { fold: vec![], inline_max_words: inline_max_words(), inline_min_same: inline_min_same() }
    }
}

fn inline_max_words() -> usize {
    InlineRule::default().max_words
}

fn inline_min_same() -> u8 {
    InlineRule::default().min_same
}

impl Review {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub fn inline(&self) -> InlineRule {
        InlineRule { max_words: self.inline_max_words, min_same: self.inline_min_same }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tui {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default)]
    pub ascii: bool,
}

impl Tui {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ai {
    /// Off by default: enabling it sends MR content to the configured provider.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl Ai {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// `v`: which program opens a file, by glob on its path, as in `"*.md" = "glow -p"`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Open {
    /// For every file no glob in `files` matches; `$VISUAL`, `$EDITOR`, then `less` without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// Glob → command; the longest matching glob wins.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub files: BTreeMap<String, String>,
}

impl Open {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Every command splits into words with known placeholders, every glob parses.
    fn check(&self) -> Result<()> {
        if let Some(command) = &self.default {
            crate::open::check(command).with_context(|| format!("`[open] default = {command:?}`"))?;
        }
        for (glob, command) in &self.files {
            glob::Pattern::new(glob).with_context(|| format!("`[open.files] {glob:?}` is not a glob"))?;
            crate::open::check(command).with_context(|| format!("`[open.files] {glob:?} = {command:?}`"))?;
        }
        Ok(())
    }
}

impl Config {
    /// Who I am on `host`: its own entry, else the top-level name when it is the configured host.
    pub fn username_for(&self, host: &str) -> Option<String> {
        let own = self.hosts.get(host).and_then(|h| h.username.clone());
        own.or_else(|| (self.host.as_deref() == Some(host)).then(|| self.username.clone()).flatten())
    }

    pub fn path() -> PathBuf {
        dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("revu").join("config.toml")
    }

    pub fn load() -> Result<Self> {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let config: Self = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        config.open.check().with_context(|| format!("in {}", path.display()))?;
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        std::fs::write(path, toml::to_string_pretty(self)?).with_context(|| format!("writing {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn round_trips_through_toml() {
        let config = Config {
            host: Some("gitlab.com".into()),
            username: Some("nina".into()),
            queue: Queue { watch_labels: vec!["infra".into()], ..Queue::default() },
            review: Review { fold: vec!["*.lock".into()], ..Review::default() },
            tui: Tui { theme: Some("nord".into()), ascii: false },
            ai: Ai::default(),
            open: Open { default: Some("hx".into()), files: BTreeMap::from([("*.md".into(), "glow -p".into())]) },
            hosts: BTreeMap::from([("git.acme.dev".into(), Host { forge: Some(Kind::GitHub), ..Host::default() })]),
        };
        let text = toml::to_string_pretty(&config).unwrap();
        assert!(text.contains("forge = \"github\""), "{text}");
        assert!(!text.contains("[ai]"), "defaults are not written:\n{text}");
        assert_eq!(toml::from_str::<Config>(&text).unwrap(), config);
    }

    #[test]
    fn a_bad_open_command_names_its_key_at_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[open.files]\n\"*.md\" = \"glow {path}\"\n").unwrap();
        let err = format!("{:#}", Config::load_from(&path).unwrap_err());
        assert!(err.contains("*.md") && err.contains("{path}"), "{err}");
        std::fs::write(&path, "[open]\ndefault = \"\"\n").unwrap();
        assert!(format!("{:#}", Config::load_from(&path).unwrap_err()).contains("empty"));
    }

    #[test]
    fn unknown_keys_fail_loudly() {
        let err = toml::from_str::<Config>("token = \"glpat-x\"").unwrap_err();
        assert!(err.to_string().contains("token"), "{err}");
    }
}
