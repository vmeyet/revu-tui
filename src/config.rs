use crate::diff::words::InlineRule;
use crate::forge::Kind;
use anyhow::{Context, Result, bail};
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
    #[serde(default, skip_serializing_if = "Notify::is_default")]
    pub notify: Notify,
    #[serde(default, skip_serializing_if = "Keys::is_default")]
    pub keys: Keys,
    /// `[share]`: where `Y` posts an MR, through a command of the user's choosing.
    #[serde(default, skip_serializing_if = "Share::is_default")]
    pub share: Share,
    /// Per-host settings, for a host whose name does not say which forge it runs.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub hosts: BTreeMap<String, Host>,
}

/// `[keys]`: keys the user adds on top of revu's own.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Keys {
    /// `azerty`: `(` and `)` do what `[` and `]` do, which a Mac French keyboard types with ⌥⇧.
    #[serde(default, skip_serializing_if = "Layout::is_default")]
    pub layout: Layout,
    /// An action name (`next_thread`) to one key or a list: `"n"`, `")n"`, `"ctrl-p"`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bind: BTreeMap<String, Bind>,
}

impl Keys {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    #[default]
    Qwerty,
    Azerty,
}

impl Layout {
    #[allow(clippy::trivially_copy_pass_by_ref, reason = "serde's skip_serializing_if hands a reference")]
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// One key, or several for the same action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Bind {
    One(String),
    Many(Vec<String>),
}

impl Bind {
    pub fn all(&self) -> &[String] {
        match self {
            Bind::One(key) => std::slice::from_ref(key),
            Bind::Many(keys) => keys,
        }
    }
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
    /// Saved filters: `'` then a view's first letter, or `1`–`9` in name order, applies it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub views: BTreeMap<String, String>,
    /// The "needs me" rules: what leaves To review, Watching and Open, and at which thresholds.
    #[serde(default, skip_serializing_if = "crate::forge::rules::Rules::is_default")]
    pub rules: crate::forge::rules::Rules,
    /// Where "ready for review" comes from: a command printing MR links.
    #[serde(default, skip_serializing_if = "Ready::is_default")]
    pub ready: Ready,
}

/// `[queue.ready]`: a command whose output names the MRs ready for review.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ready {
    /// Run as words, never through a shell; every MR link it prints counts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

impl Ready {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

impl Queue {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Every view is a query that parses, and no two start with the same letter, since `'` picks by it.
    fn check(&self) -> Result<()> {
        self.rules.check()?;
        if let Some(command) = &self.ready.command {
            crate::ready::check(command)?;
        }
        let mut firsts: BTreeMap<char, &str> = BTreeMap::new();
        for (name, query) in &self.views {
            crate::query::Query::parse(query).map_err(|e| anyhow::anyhow!("`queue.views.{name}`: {e}"))?;
            let Some(first) = name.chars().next() else { bail!("`queue.views` has a view with no name") };
            if let Some(other) = firsts.insert(first.to_ascii_lowercase(), name) {
                bail!("`queue.views.{other}` and `queue.views.{name}` both start with `{first}`: ' picks a view by its first letter");
            }
        }
        Ok(())
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

/// `[notify]`: a macOS notification when an MR lands in To review while the TUI runs. On by default.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Notify {
    #[serde(default = "on")]
    pub enabled: bool,
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

fn on() -> bool {
    true
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tui {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default)]
    pub ascii: bool,
    /// Pictures in comments, drawn in the thread pane where the terminal can; `false` shows their `[image: …]` line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<bool>,
    #[serde(default, skip_serializing_if = "QueueLayout::is_default")]
    pub queue: QueueLayout,
}

/// How a queue row reads: two lines (the title, then number, author, age and size), or one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QueueLayout {
    #[default]
    Comfortable,
    Compact,
}

impl QueueLayout {
    #[allow(clippy::trivially_copy_pass_by_ref, reason = "serde's skip_serializing_if hands a reference")]
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

impl Tui {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// `[ai]`: each provider is switched on by itself, and both are off until then, because enabling
/// one sends MR content to it. Keys are never here: they live in the keychain (`revu ai login`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ai {
    #[serde(default, skip_serializing_if = "Typesafe::is_default")]
    pub typesafe: Typesafe,
    #[serde(default, skip_serializing_if = "Anthropic::is_default")]
    pub anthropic: Anthropic,
    /// The single `[ai] enabled/provider/model` switch this table replaced; read only to point at the new keys.
    #[serde(default, skip_serializing)]
    enabled: Option<toml::Value>,
    #[serde(default, skip_serializing)]
    provider: Option<toml::Value>,
    #[serde(default, skip_serializing)]
    model: Option<toml::Value>,
}

impl Ai {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }

    fn check(&self) -> Result<()> {
        if self.enabled.is_some() || self.provider.is_some() || self.model.is_some() {
            anyhow::bail!(
                "`[ai] enabled`, `provider` and `model` moved to one table per provider: \
                 `[ai.anthropic] enabled = true` (with `model`), `[ai.typesafe] enabled = true`"
            );
        }
        Ok(())
    }
}

/// `[ai.typesafe]`: Jev ranks the queue and the files of an MR.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Typesafe {
    #[serde(default)]
    pub enabled: bool,
}

impl Typesafe {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// `[ai.anthropic]`: Claude answers questions about the MR.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Anthropic {
    #[serde(default)]
    pub enabled: bool,
    /// `claude-opus-5` when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl Anthropic {
    pub const DEFAULT_MODEL: &str = "claude-opus-5";

    fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub fn model(&self) -> &str {
        self.model.as_deref().unwrap_or(Self::DEFAULT_MODEL)
    }
}

/// `[share]`: `Y` posts an MR by piping a message to a command, so revu knows no chat tool.
/// A bare `command` is the one target; `[share.targets.<name>]` adds named ones, and `Y` asks which.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Share {
    /// Run as words, never through a shell; the message arrives on its stdin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// The message, with placeholders like `{ref}` and `{title}`; also the template of named targets that set none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    /// Named destinations, each with its own command and, optionally, its own template.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub targets: BTreeMap<String, ShareTarget>,
}

/// `[share.targets.<name>]`: one more place `Y` can post to.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareTarget {
    /// Run as words, never through a shell; the message arrives on its stdin.
    pub command: String,
    /// This target's message; `[share] template`, then the built-in one, when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
}

impl Share {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Every command splits into words and every template names only placeholders revu fills.
    fn check(&self) -> Result<()> {
        if let Some(command) = &self.command {
            crate::program::check(command, "share.command")?;
        }
        if let Some(template) = &self.template {
            crate::share::check_template(template, "share.template")?;
        }
        for (name, target) in &self.targets {
            crate::program::check(&target.command, &format!("share.targets.{name}.command"))?;
            if let Some(template) = &target.template {
                crate::share::check_template(template, &format!("share.targets.{name}.template"))?;
            }
        }
        Ok(())
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

/// `$XDG_CONFIG_HOME/revu`, else `~/.config/revu`: where terminal tools keep their config on
/// every system, macOS included, so it sits next to gh, helix and a dotfiles repo.
pub fn dir() -> PathBuf {
    dir_from(std::env::var_os("XDG_CONFIG_HOME"), dirs::home_dir())
}

fn dir_from(xdg: Option<std::ffi::OsString>, home: Option<PathBuf>) -> PathBuf {
    let xdg = xdg.map(PathBuf::from).filter(|p| p.is_absolute());
    xdg.or_else(|| home.map(|h| h.join(".config"))).unwrap_or_else(|| PathBuf::from(".")).join("revu")
}

impl Config {
    /// Who I am on `host`: its own entry, else the top-level name when it is the configured host.
    pub fn username_for(&self, host: &str) -> Option<String> {
        let own = self.hosts.get(host).and_then(|h| h.username.clone());
        own.or_else(|| (self.host.as_deref() == Some(host)).then(|| self.username.clone()).flatten())
    }

    pub fn path() -> PathBuf {
        dir().join("config.toml")
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
        config.queue.check().with_context(|| format!("in {}", path.display()))?;
        config.ai.check().with_context(|| format!("in {}", path.display()))?;
        config.share.check().with_context(|| format!("in {}", path.display()))?;
        crate::keymap::Keymap::new(&config.keys).with_context(|| format!("in {}", path.display()))?;
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
            tui: Tui { theme: Some("nord".into()), ..Tui::default() },
            ai: Ai::default(),
            open: Open { default: Some("hx".into()), files: BTreeMap::from([("*.md".into(), "glow -p".into())]) },
            hosts: BTreeMap::from([("git.acme.dev".into(), Host { forge: Some(Kind::GitHub), ..Host::default() })]),
            notify: Notify { enabled: false },
            keys: Keys {
                layout: Layout::Azerty,
                bind: BTreeMap::from([("next_thread".into(), Bind::One("N".into())), ("jump".into(), Bind::Many(vec!["ctrl-p".into()]))]),
            },
            share: Share {
                command: Some("slack send '#review'".into()),
                template: None,
                targets: BTreeMap::from([("team".into(), ShareTarget { command: "hook".into(), template: Some("{url}".into()) })]),
            },
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
    fn each_ai_provider_has_its_own_table_and_the_old_switch_names_the_new_one() {
        let config: Config = toml::from_str("[ai.anthropic]\nenabled = true\n\n[ai.typesafe]\nenabled = true\n").unwrap();
        assert!(config.ai.anthropic.enabled && config.ai.typesafe.enabled);
        assert_eq!(config.ai.anthropic.model(), "claude-opus-5");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[ai]\nenabled = true\nprovider = \"anthropic\"\n").unwrap();
        let err = format!("{:#}", Config::load_from(&path).unwrap_err());
        assert!(err.contains("[ai.anthropic] enabled = true"), "{err}");
        assert!(toml::from_str::<Config>("[ai.anthropic]\nkey = \"sk-ant\"\n").is_err(), "a key in the config is refused");
    }

    #[test]
    fn views_must_parse_and_start_with_distinct_letters() {
        let load = |toml: &str| {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.toml");
            std::fs::write(&path, toml).unwrap();
            Config::load_from(&path).map_err(|e| format!("{e:#}"))
        };
        let ok = load("[queue.views]\nfront = \"@me ~frontend\"\nbacklog = \"draft:no size:small\"\n").unwrap();
        assert_eq!(ok.queue.views.len(), 2);
        let bad = load("[queue.views]\nfront = \"size:huge\"\n").unwrap_err();
        assert!(bad.contains("queue.views.front") && bad.contains("size: takes small or large") && bad.contains("config.toml"), "{bad}");
        let clash = load("[queue.views]\nfront = \"@me\"\nfixes = \"fix\"\n").unwrap_err();
        assert!(clash.contains("both start with `f`"), "{clash}");
    }

    #[test]
    fn rules_load_with_defaults_and_bad_thresholds_fail_with_the_file() {
        let load = |toml: &str| {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.toml");
            std::fs::write(&path, toml).unwrap();
            Config::load_from(&path).map_err(|e| format!("{e:#}"))
        };
        let partial = load("[queue.rules]\nstale_days = 30\n").unwrap();
        assert_eq!((partial.queue.rules.stale_days, partial.queue.rules.reviewed_comments), (30, 3), "unset keys keep their default");
        assert!(partial.queue.rules.enabled);
        assert!(!load("[queue.rules]\nenabled = false\n").unwrap().queue.rules.enabled);
        let zero = load("[queue.rules]\nstale_days = 0\n").unwrap_err();
        assert!(zero.contains("stale_days") && zero.contains("config.toml"), "{zero}");
        let typo = load("[queue.rules]\nstale = 3\n").unwrap_err();
        assert!(typo.contains("stale"), "{typo}");
    }

    #[test]
    fn the_ready_command_must_split_into_words() {
        let load = |toml: &str| {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.toml");
            std::fs::write(&path, toml).unwrap();
            Config::load_from(&path).map_err(|e| format!("{e:#}"))
        };
        let ok = load("[queue.ready]\ncommand = \"slack messages '#review' --json\"\n").unwrap();
        assert_eq!(ok.queue.ready.command.as_deref(), Some("slack messages '#review' --json"));
        let bad = load("[queue.ready]\ncommand = \"slack 'open\"\n").unwrap_err();
        assert!(bad.contains("queue.ready.command") && bad.contains("config.toml"), "{bad}");
    }

    #[test]
    fn unknown_keys_fail_loudly() {
        let err = toml::from_str::<Config>("token = \"glpat-x\"").unwrap_err();
        assert!(err.to_string().contains("token"), "{err}");
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

    #[test]
    fn share_commands_and_templates_are_checked_with_the_target_named() {
        let load = |toml: &str| {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.toml");
            std::fs::write(&path, toml).unwrap();
            Config::load_from(&path).map_err(|e| format!("{e:#}"))
        };
        let ok =
            load("[share]\ncommand = \"slack send '#review'\"\n[share.targets.team]\ncommand = \"hook\"\ntemplate = \"{ref} {url}\"\n")
                .unwrap();
        assert_eq!(ok.share.targets["team"].template.as_deref(), Some("{ref} {url}"));
        let typo = load("[share.targets.team]\ncommand = \"hook\"\ntemplate = \"{titel}\"\n").unwrap_err();
        assert!(typo.contains("share.targets.team.template") && typo.contains("{titel}") && typo.contains("config.toml"), "{typo}");
        let quote = load("[share]\ncommand = \"slack 'open\"\n").unwrap_err();
        assert!(quote.contains("share.command"), "{quote}");
    }
}
