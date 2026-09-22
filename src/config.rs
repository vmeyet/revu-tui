use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
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
    #[serde(default, skip_serializing_if = "Tui::is_default")]
    pub tui: Tui,
    #[serde(default, skip_serializing_if = "Ai::is_default")]
    pub ai: Ai,
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

impl Config {
    pub fn path() -> PathBuf {
        dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("gitlabmr").join("config.toml")
    }

    pub fn load() -> Result<Self> {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
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
    use super::*;

    #[test]
    fn round_trips_through_toml() {
        let config = Config {
            host: Some("gitlab.com".into()),
            username: Some("nina".into()),
            queue: Queue { watch_labels: vec!["infra".into()], ..Queue::default() },
            tui: Tui { theme: Some("nord".into()), ascii: false },
            ai: Ai::default(),
        };
        let text = toml::to_string_pretty(&config).unwrap();
        assert!(!text.contains("[ai]"), "defaults are not written:\n{text}");
        assert_eq!(toml::from_str::<Config>(&text).unwrap(), config);
    }

    #[test]
    fn unknown_keys_fail_loudly() {
        let err = toml::from_str::<Config>("token = \"glpat-x\"").unwrap_err();
        assert!(err.to_string().contains("token"), "{err}");
    }
}
