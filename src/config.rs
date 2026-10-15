use crate::diff::words::InlineRule;
use crate::forge::Kind;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

mod ignored;
mod programs;
mod queue;
mod save;
mod tui;

#[cfg(test)]
pub(crate) use programs::ShareTarget;
pub(crate) use programs::{Agent, AgentShow, Open, Share};
use queue::Queue;
use tui::Tui;
pub(crate) use tui::{QueueLayout, Widths};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) username: Option<String>,
    #[serde(default, skip_serializing_if = "Queue::is_default")]
    pub(crate) queue: Queue,
    #[serde(default, skip_serializing_if = "Review::is_default")]
    pub(crate) review: Review,
    #[serde(default, skip_serializing_if = "Tui::is_default")]
    pub(crate) tui: Tui,
    #[serde(default, skip_serializing_if = "Ai::is_default")]
    pub(crate) ai: Ai,
    #[serde(default, skip_serializing_if = "Open::is_default")]
    pub(crate) open: Open,
    #[serde(default, skip_serializing_if = "Notify::is_default")]
    pub(crate) notify: Notify,
    /// What revu counts about how it is used, on this machine only; off by default.
    #[serde(default, skip_serializing_if = "Usage::is_default")]
    pub(crate) usage: Usage,
    #[serde(default, skip_serializing_if = "Keys::is_default")]
    pub(crate) keys: Keys,
    /// `[share]`: where `Y` posts an MR, through a command of the user's choosing.
    #[serde(default, skip_serializing_if = "Share::is_default")]
    pub(crate) share: Share,
    /// `[agent]`: the coding-agent session `a g` opens on an MR.
    #[serde(default, skip_serializing_if = "Agent::is_default")]
    pub(crate) agent: Agent,
    /// Per-host settings, for a host whose name does not say which forge it runs.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) hosts: BTreeMap<String, Host>,
}

/// `[keys]`: keys the user adds on top of revu's own.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Keys {
    /// `azerty`: `(` and `)` do what `[` and `]` do, which a Mac French keyboard types with ⌥⇧.
    #[serde(default, skip_serializing_if = "Layout::is_default")]
    pub(crate) layout: Layout,
    /// An action name (`next_thread`) to one key or a list: `"n"`, `")n"`, `"ctrl-p"`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) bind: BTreeMap<String, Bind>,
    /// `q` and `ctrl-c` need a second press; `false` quits on the first.
    #[serde(default = "on")]
    pub(crate) quit_confirm: bool,
}

impl Default for Keys {
    fn default() -> Self {
        Self { layout: Layout::default(), bind: BTreeMap::new(), quit_confirm: true }
    }
}

impl Keys {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Layout {
    #[default]
    Qwerty,
    Azerty,
}

impl Layout {
    #[expect(clippy::trivially_copy_pass_by_ref, reason = "serde's skip_serializing_if hands a reference")]
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// One key, or several for the same action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum Bind {
    One(String),
    Many(Vec<String>),
}

impl Bind {
    pub(crate) fn all(&self) -> &[String] {
        match self {
            Bind::One(key) => std::slice::from_ref(key),
            Bind::Many(keys) => keys,
        }
    }
}

/// `[hosts."git.acme.dev"] forge = "github"`: a GitHub Enterprise host, which the name alone cannot tell.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Host {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) forge: Option<Kind>,
    /// Who I am there, from the last login.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) username: Option<String>,
}

/// How a diff opens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Review {
    /// Glob patterns of files that open folded: lockfiles, snapshots, generated code.
    #[serde(default)]
    pub(crate) fold: Vec<String>,
    /// A changed pair reads as one row when each side changes at most this many words…
    #[serde(default = "inline_max_words")]
    pub(crate) inline_max_words: usize,
    /// …and both lines keep at least this share of their text, in percent.
    #[serde(default = "inline_min_same")]
    pub(crate) inline_min_same: u8,
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

    pub(crate) fn inline(&self) -> InlineRule {
        InlineRule { max_words: self.inline_max_words, min_same: self.inline_min_same }
    }
}

/// `[notify]`: a macOS notification when an MR lands in To review while the TUI runs. On by default.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Notify {
    #[serde(default = "on")]
    pub(crate) enabled: bool,
}

impl Default for Notify {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl Notify {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// `[usage]`: count the actions used and the time per screen, into the cache, for `revu usage`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Usage {
    /// Off by default: nothing is counted until this is true.
    #[serde(default)]
    pub(crate) enabled: bool,
}

impl Usage {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

fn on() -> bool {
    true
}

/// `[ai]`: each provider is switched on by itself, and both are off until then, because enabling
/// one sends MR content to it. Keys are never here: they live in the keychain (`revu ai login`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Ai {
    #[serde(default, skip_serializing_if = "Typesafe::is_default")]
    pub(crate) typesafe: Typesafe,
    #[serde(default, skip_serializing_if = "Anthropic::is_default")]
    pub(crate) anthropic: Anthropic,
}

impl Ai {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// `[ai.typesafe]`: Jev ranks the queue and the files of an MR.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Typesafe {
    #[serde(default)]
    pub(crate) enabled: bool,
}

impl Typesafe {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// `[ai.anthropic]`: Claude answers questions about the MR.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Anthropic {
    #[serde(default)]
    pub(crate) enabled: bool,
    /// `claude-opus-5` when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) model: Option<String>,
}

impl Anthropic {
    const DEFAULT_MODEL: &str = "claude-opus-5";

    fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub(crate) fn model(&self) -> &str {
        self.model.as_deref().unwrap_or(Self::DEFAULT_MODEL)
    }
}

/// A config as read from its file.
#[derive(Debug, Default)]
pub(crate) struct Loaded {
    pub(crate) config: Config,
    /// One line for each key the file holds that this revu ignores.
    pub(crate) warnings: Vec<String>,
}

/// What the config file holds that this revu ignores; nothing when it does not load, since the command then says why.
pub(crate) fn warnings() -> Vec<String> {
    Config::read(&Config::path()).map(|loaded| loaded.warnings).unwrap_or_default()
}

/// `$XDG_CONFIG_HOME/revu`, else `~/.config/revu`: where terminal tools keep their config on
/// every system, macOS included, so it sits next to gh, helix and a dotfiles repo.
pub(crate) fn dir() -> PathBuf {
    dir_from(std::env::var_os("XDG_CONFIG_HOME"), dirs::home_dir())
}

fn dir_from(xdg: Option<std::ffi::OsString>, home: Option<PathBuf>) -> PathBuf {
    let xdg = xdg.map(PathBuf::from).filter(|p| p.is_absolute());
    xdg.or_else(|| home.map(|h| h.join(".config"))).unwrap_or_else(|| PathBuf::from(".")).join("revu")
}

impl Config {
    /// Who I am on `host`: its own entry, else the top-level name when it is the configured host.
    pub(crate) fn username_for(&self, host: &str) -> Option<String> {
        let own = self.hosts.get(host).and_then(|h| h.username.clone());
        own.or_else(|| (self.host.as_deref() == Some(host)).then(|| self.username.clone()).flatten())
    }

    pub(crate) fn path() -> PathBuf {
        dir().join("config.toml")
    }

    pub(crate) fn load() -> Result<Self> {
        Self::load_from(&Self::path())
    }

    pub(crate) fn load_from(path: &Path) -> Result<Self> {
        Self::read(path).map(|loaded| loaded.config)
    }

    /// The config at `path`, with a warning for each key in it this revu ignores.
    pub(crate) fn read(path: &Path) -> Result<Loaded> {
        if !path.exists() {
            return Ok(Loaded::default());
        }
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let (config, mut warnings) = ignored::parse(&text).with_context(|| format!("parsing {}", path.display()))?;
        config.check().with_context(|| format!("in {}", path.display()))?;
        warnings.extend(crate::keymap::ignored(&config.keys));
        Ok(Loaded { config, warnings })
    }

    fn check(&self) -> Result<()> {
        self.open.check()?;
        self.queue.check()?;
        self.share.check()?;
        self.agent.check()?;
        self.tui.check()?;
        crate::keymap::Keymap::new(&self.keys).map(drop)
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
            review: Review { fold: vec!["*.lock".into()], ..Review::default() },
            tui: Tui { theme: Some("nord".into()), ..Tui::default() },
            ai: Ai::default(),
            open: Open { default: Some("hx".into()), files: BTreeMap::from([("*.md".into(), "glow -p".into())]) },
            hosts: BTreeMap::from([("git.acme.dev".into(), Host { forge: Some(Kind::GitHub), ..Host::default() })]),
            notify: Notify { enabled: false },
            usage: Usage { enabled: true },
            keys: Keys {
                layout: Layout::Azerty,
                bind: BTreeMap::from([("next_thread".into(), Bind::One("F".into())), ("jump".into(), Bind::Many(vec!["ctrl-p".into()]))]),
                quit_confirm: false,
            },
            share: Share {
                command: Some("slack send '#review'".into()),
                template: None,
                targets: BTreeMap::from([("team".into(), ShareTarget { command: "hook".into(), template: Some("{url}".into()) })]),
            },
            agent: Agent {
                enabled: true,
                checkouts: BTreeMap::from([("acme/widgets".into(), "~/code/widgets".into())]),
                ..Agent::default()
            },
        };
        let text = toml::to_string_pretty(&config).unwrap();
        assert!(text.contains("forge = \"github\""), "{text}");
        assert!(!text.contains("[ai]"), "defaults are not written:\n{text}");
        assert_eq!(toml::from_str::<Config>(&text).unwrap(), config);
    }

    #[test]
    fn each_ai_provider_has_its_own_table_and_the_old_switch_names_the_new_one() {
        let config: Config = toml::from_str("[ai.anthropic]\nenabled = true\n\n[ai.typesafe]\nenabled = true\n").unwrap();
        assert!(config.ai.anthropic.enabled && config.ai.typesafe.enabled);
        assert_eq!(config.ai.anthropic.model(), "claude-opus-5");
    }

    #[test]
    fn a_binding_on_an_action_revu_removed_is_ignored_with_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[keys.bind]\nnext_review = \"F\"\nnext_thread = \"f\"\n").unwrap();
        let loaded = Config::read(&path).unwrap();
        assert_eq!(loaded.warnings, ["`next_review` is not an action this revu knows; it is ignored"]);
        assert!(crate::keymap::Keymap::new(&loaded.config.keys).is_ok(), "the rest of the bindings load");
        std::fs::write(&path, "[keys.bind]\nnext_reviw = \"F\"\n").unwrap();
        assert!(Config::read(&path).is_err(), "a typo still stops revu, naming the real actions");
    }

    #[test]
    fn a_config_written_for_a_newer_revu_loads_with_a_warning_for_each_key_it_ignores() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[tui]\ntheme = \"nord\"\nsparkles = true\n\n[sync]\nevery = 5\n").unwrap();
        let loaded = Config::read(&path).unwrap();
        assert_eq!(loaded.config.tui.theme.as_deref(), Some("nord"));
        assert_eq!(loaded.warnings.len(), 2, "{:?}", loaded.warnings);
        std::fs::write(&path, "[ai]\nenabled = true\n").unwrap();
        assert!(Config::read(&path).unwrap().warnings[0].contains("[ai.anthropic] enabled = true"), "the moved switch says where it went");
        std::fs::write(&path, "[ai.anthropic]\nkey = \"sk-ant\"\n").unwrap();
        let err = format!("{:#}", Config::load_from(&path).unwrap_err());
        assert!(err.contains("config.toml") && err.contains("keychain"), "a key in the config is refused: {err}");
    }

    #[test]
    fn notifications_are_on_unless_switched_off() {
        assert!(toml::from_str::<Config>("").unwrap().notify.enabled);
        assert!(!toml::from_str::<Config>("[notify]\nenabled = false").unwrap().notify.enabled);
        assert!(!toml::to_string_pretty(&Config::default()).unwrap().contains("[notify]"), "the default is not written");
    }

    #[test]
    fn the_config_lives_under_xdg_config_home_else_dot_config() {
        let home = Some(PathBuf::from("/Users/nina"));
        assert_eq!(dir_from(None, home.clone()), PathBuf::from("/Users/nina/.config/revu"));
        assert_eq!(dir_from(Some("/tmp/xdg".into()), home.clone()), PathBuf::from("/tmp/xdg/revu"));
        assert_eq!(
            dir_from(Some("relative".into()), home.clone()),
            PathBuf::from("/Users/nina/.config/revu"),
            "a relative XDG path is ignored"
        );
        assert_eq!(dir_from(Some("".into()), home), PathBuf::from("/Users/nina/.config/revu"));
    }

    #[test]
    fn a_bad_key_binding_fails_with_the_config_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[keys.bind]\nnext_thread = \"c\"\n").unwrap();
        let err = format!("{:#}", Config::load_from(&path).unwrap_err());
        assert!(err.contains("config.toml") && err.contains("comment"), "{err}");
        std::fs::write(&path, "[keys]\nlayout = \"dvorak\"\n").unwrap();
        assert!(Config::load_from(&path).is_err(), "an unknown layout fails");
    }
}
