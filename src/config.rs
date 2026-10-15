use crate::diff::words::InlineRule;
use crate::forge::Kind;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Table, Value};

mod ignored;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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
    /// What revu counts about how it is used, on this machine only; off by default.
    #[serde(default, skip_serializing_if = "Usage::is_default")]
    pub usage: Usage,
    #[serde(default, skip_serializing_if = "Keys::is_default")]
    pub keys: Keys,
    /// `[share]`: where `Y` posts an MR, through a command of the user's choosing.
    #[serde(default, skip_serializing_if = "Share::is_default")]
    pub share: Share,
    /// `[agent]`: the coding-agent session `a g` opens on an MR.
    #[serde(default, skip_serializing_if = "Agent::is_default")]
    pub agent: Agent,
    /// Per-host settings, for a host whose name does not say which forge it runs.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub hosts: BTreeMap<String, Host>,
}

/// `[keys]`: keys the user adds on top of revu's own.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keys {
    /// `azerty`: `(` and `)` do what `[` and `]` do, which a Mac French keyboard types with ⌥⇧.
    #[serde(default, skip_serializing_if = "Layout::is_default")]
    pub layout: Layout,
    /// An action name (`next_thread`) to one key or a list: `"n"`, `")n"`, `"ctrl-p"`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bind: BTreeMap<String, Bind>,
    /// `q` and `ctrl-c` need a second press; `false` quits on the first.
    #[serde(default = "on")]
    pub quit_confirm: bool,
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
pub enum Layout {
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
pub struct Host {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forge: Option<Kind>,
    /// Who I am there, from the last login.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

/// Which MRs the queue shows beyond the ones GitLab lists for me.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Queue {
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
    /// How many MRs that need me are loaded ahead, so opening them needs no network; 0 turns it off.
    #[serde(default = "default_prefetch", skip_serializing_if = "is_default_prefetch")]
    pub prefetch: usize,
}

const PREFETCH: usize = 5;

impl Default for Queue {
    fn default() -> Self {
        Self {
            watch_labels: vec![],
            views: BTreeMap::new(),
            rules: crate::forge::rules::Rules::default(),
            ready: Ready::default(),
            prefetch: PREFETCH,
        }
    }
}

fn default_prefetch() -> usize {
    PREFETCH
}

#[expect(clippy::trivially_copy_pass_by_ref, reason = "serde's skip_serializing_if passes a reference")]
fn is_default_prefetch(n: &usize) -> bool {
    *n == PREFETCH
}

/// `[queue.ready]`: a command whose output names the MRs ready for review.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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

/// `[usage]`: count the actions used and the time per screen, into the cache, for `revu usage`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Off by default: nothing is counted until this is true.
    #[serde(default)]
    pub enabled: bool,
}

impl Usage {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

fn on() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    /// How wide the diff reads in zen, `zz`, in columns; unset, it grows with the screen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zen_width: Option<u16>,
    /// Whether the compact layout follows the width, always shows or never does.
    #[serde(default, skip_serializing_if = "ScreenLayout::is_default")]
    pub layout: ScreenLayout,
    /// Below this many columns the window is compact; unset, `Widths::default` says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_width: Option<u16>,
    /// Below this many columns the window is narrow; unset, `Widths::default` says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narrow_width: Option<u16>,
    /// From this many columns the queue, the diff and the right pane sit side by side; unset, `Widths::default` says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wide_width: Option<u16>,
    /// The compact layout ends on a row of labels to tap; `false` gives the row back to the page.
    #[serde(default = "on")]
    pub tap_bar: bool,
}

impl Default for Tui {
    fn default() -> Self {
        Self {
            theme: None,
            ascii: false,
            images: None,
            queue: QueueLayout::default(),
            zen_width: None,
            layout: ScreenLayout::default(),
            compact_width: None,
            narrow_width: None,
            wide_width: None,
            tap_bar: true,
        }
    }
}

/// Narrower than this, a diff line and its gutters no longer fit.
const MIN_ZEN_WIDTH: u16 = 60;

/// `[tui] layout`: the compact layout below `compact_width`, at any width, or never.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreenLayout {
    #[default]
    Auto,
    Compact,
    Full,
}

impl ScreenLayout {
    #[expect(clippy::trivially_copy_pass_by_ref, reason = "serde's skip_serializing_if hands a reference")]
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// The terminal widths the layout turns on. Below `compact` one page shows at a time, full screen.
/// Below `narrow` an open MR hides the queue unless it has the keys, the right pane is a page of
/// its own and the diff shows one line number; from `wide` the queue, the diff and the right pane
/// sit side by side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Widths {
    pub compact: u16,
    pub narrow: u16,
    pub wide: u16,
}

impl Default for Widths {
    fn default() -> Self {
        Self { compact: 70, narrow: 120, wide: 150 }
    }
}

impl Tui {
    /// `layout` moves the compact limit past every window, or under every one; unset, it never passes the narrow one.
    pub fn widths(&self) -> Widths {
        let default = Widths::default();
        let narrow = self.narrow_width.unwrap_or(default.narrow);
        let compact = match self.layout {
            ScreenLayout::Auto => self.compact_width.unwrap_or(default.compact.min(narrow)),
            ScreenLayout::Compact => u16::MAX,
            ScreenLayout::Full => 0,
        };
        Widths { compact, narrow, wide: self.wide_width.unwrap_or(default.wide) }
    }

    fn check(&self) -> Result<()> {
        if let Some(width) = self.zen_width.filter(|&width| width < MIN_ZEN_WIDTH) {
            bail!("`tui.zen_width = {width}` is too narrow: {MIN_ZEN_WIDTH} is the least");
        }
        let Widths { narrow, wide, .. } = self.widths();
        if self.compact_width == Some(0) {
            bail!("`tui.compact_width = 0` is no width: 1 is the least, and `tui.layout = \"full\"` turns the compact layout off");
        }
        if narrow == 0 {
            bail!("`tui.narrow_width = 0` is no width: 1 is the least");
        }
        if let Some(compact) = self.compact_width.filter(|&compact| compact > narrow) {
            bail!(
                "`tui.compact_width = {compact}` is above `tui.narrow_width = {narrow}`: a compact window ends where a narrow one starts, or before"
            );
        }
        if narrow > wide {
            bail!(
                "`tui.narrow_width = {narrow}` is above `tui.wide_width = {wide}`: a narrow window ends where a wide one starts, or before"
            );
        }
        Ok(())
    }
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
    #[expect(clippy::trivially_copy_pass_by_ref, reason = "serde's skip_serializing_if hands a reference")]
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
pub struct Ai {
    #[serde(default, skip_serializing_if = "Typesafe::is_default")]
    pub typesafe: Typesafe,
    #[serde(default, skip_serializing_if = "Anthropic::is_default")]
    pub anthropic: Anthropic,
}

impl Ai {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// `[ai.typesafe]`: Jev ranks the queue and the files of an MR.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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

/// `[agent]`: `a g` opens a coding-agent session that belongs to the MR, in a worktree on its branch.
/// Claude Code is the agent unless `start` names another program.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agent {
    /// Off by default: nothing runs until this is true.
    #[serde(default)]
    pub enabled: bool,
    /// Another agent than Claude Code: the command that starts its session, run as words in the MR's worktree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    /// The command that opens again the session `start` made; `start` runs again without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<String>,
    /// Project path → its checkout on this machine, for MRs of a project revu was not started in.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub checkouts: BTreeMap<String, String>,
    /// Where the session opens: beside revu when it fits, always beside, or never.
    #[serde(default, skip_serializing_if = "AgentShow::is_default")]
    pub show: AgentShow,
    /// For a terminal revu cannot split itself: the command that opens a pane beside revu and runs `{command}` there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split: Option<String>,
}

/// `[agent] show`: a pane beside revu when revu is wide enough for both, at any width, or never.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentShow {
    #[default]
    Auto,
    Split,
    Window,
}

impl AgentShow {
    #[expect(clippy::trivially_copy_pass_by_ref, reason = "serde's skip_serializing_if hands a reference")]
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

impl Agent {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Both commands split into words and name only placeholders revu fills; every checkout is a full path.
    fn check(&self) -> Result<()> {
        if let Some(start) = &self.start {
            crate::agent::check(start, "agent.start")?;
        }
        match (&self.open, &self.start) {
            (Some(_), None) => bail!("`agent.open` needs `agent.start`: it opens again what that command started"),
            (Some(open), Some(_)) => crate::agent::check(open, "agent.open")?,
            (None, _) => {}
        }
        if let Some(split) = &self.split {
            crate::agent::check_split(split, "agent.split")?;
        }
        for (project, folder) in &self.checkouts {
            if !folder.starts_with('/') && !folder.starts_with("~/") {
                bail!("`agent.checkouts.\"{project}\" = \"{folder}\"` is not a full path: start it with `/` or `~/`");
            }
        }
        Ok(())
    }

    /// The checkouts as folders, `~/` standing for `home`.
    pub fn folders(&self, home: &Path) -> BTreeMap<String, PathBuf> {
        let folder = |written: &String| written.strip_prefix("~/").map_or_else(|| PathBuf::from(written), |rest| home.join(rest));
        self.checkouts.iter().map(|(project, written)| (project.clone(), folder(written))).collect()
    }
}

/// `^v`: which program opens a file, by glob on its path, as in `"*.md" = "glow -p"`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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

/// A config as read from its file.
#[derive(Debug, Default)]
pub struct Loaded {
    pub config: Config,
    /// One line for each key the file holds that this revu ignores.
    pub warnings: Vec<String>,
}

/// What the config file holds that this revu ignores; nothing when it does not load, since the command then says why.
pub fn warnings() -> Vec<String> {
    Config::read(&Config::path()).map(|loaded| loaded.warnings).unwrap_or_default()
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
        Self::read(path).map(|loaded| loaded.config)
    }

    /// The config at `path`, with a warning for each key in it this revu ignores.
    pub fn read(path: &Path) -> Result<Loaded> {
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

    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::path())
    }

    /// Rewrites only the keys that changed since the file was read, so the user's comments and layout
    /// stay; a crash mid-write leaves the old file whole.
    pub fn save_to(&self, path: &Path) -> Result<()> {
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
    fn zen_width_is_unset_by_default_and_refuses_a_column_too_narrow() {
        let load = |toml: &str| {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.toml");
            std::fs::write(&path, toml).unwrap();
            Config::load_from(&path).map_err(|e| format!("{e:#}"))
        };
        assert_eq!(load("").unwrap().tui.zen_width, None, "the column grows with the screen");
        assert_eq!(load("[tui]\nzen_width = 120\n").unwrap().tui.zen_width, Some(120));
        let narrow = load("[tui]\nzen_width = 40\n").unwrap_err();
        assert!(narrow.contains("zen_width") && narrow.contains("config.toml"), "{narrow}");
    }

    fn widths_of(toml: &str) -> Result<Widths, String> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, toml).unwrap();
        Config::load_from(&path).map(|config| config.tui.widths()).map_err(|e| format!("{e:#}"))
    }

    #[test]
    fn layout_widths_default_to_120_and_150_and_refuse_a_narrow_above_the_wide() {
        assert_eq!(widths_of("").unwrap(), Widths { compact: 70, narrow: 120, wide: 150 });
        assert_eq!(widths_of("[tui]\nnarrow_width = 100\n").unwrap(), Widths { compact: 70, narrow: 100, wide: 150 });
        assert_eq!(widths_of("[tui]\nwide_width = 180\n").unwrap(), Widths { compact: 70, narrow: 120, wide: 180 });
        assert_eq!(widths_of("[tui]\nnarrow_width = 130\nwide_width = 130\n").unwrap(), Widths { compact: 70, narrow: 130, wide: 130 });
        let zero = widths_of("[tui]\nnarrow_width = 0\n").unwrap_err();
        assert!(zero.contains("narrow_width = 0") && zero.contains("config.toml"), "{zero}");
        let no_wide = widths_of("[tui]\nwide_width = 0\n").unwrap_err();
        assert!(no_wide.contains("wide_width = 0") && no_wide.contains("config.toml"), "{no_wide}");
        let above = widths_of("[tui]\nnarrow_width = 160\n").unwrap_err();
        assert!(above.contains("narrow_width = 160") && above.contains("wide_width = 150"), "{above}");
        let below = widths_of("[tui]\nwide_width = 100\n").unwrap_err();
        assert!(below.contains("narrow_width = 120") && below.contains("wide_width = 100"), "{below}");
    }

    #[test]
    fn the_compact_width_defaults_to_70_and_never_passes_the_narrow_one() {
        assert_eq!(widths_of("[tui]\ncompact_width = 60\n").unwrap().compact, 60);
        assert_eq!(widths_of("[tui]\ncompact_width = 100\nnarrow_width = 100\n").unwrap().compact, 100);
        assert_eq!(widths_of("[tui]\nnarrow_width = 50\n").unwrap().compact, 50, "an unset one follows a narrow width set under it");
        let above = widths_of("[tui]\ncompact_width = 130\n").unwrap_err();
        assert!(above.contains("compact_width = 130") && above.contains("narrow_width = 120") && above.contains("config.toml"), "{above}");
        let zero = widths_of("[tui]\ncompact_width = 0\n").unwrap_err();
        assert!(zero.contains("compact_width = 0") && zero.contains("layout = \"full\""), "{zero}");
    }

    #[test]
    fn the_layout_setting_forces_the_compact_layout_or_turns_it_off() {
        assert_eq!(widths_of("[tui]\nlayout = \"auto\"\n").unwrap().compact, 70);
        assert_eq!(widths_of("[tui]\nlayout = \"compact\"\n").unwrap().compact, u16::MAX, "every window is under it");
        assert_eq!(widths_of("[tui]\nlayout = \"full\"\ncompact_width = 60\n").unwrap().compact, 0, "no window is under it");
        assert!(widths_of("[tui]\nlayout = \"phone\"\n").unwrap_err().contains("layout"));
    }

    #[test]
    fn the_tap_bar_is_on_unless_switched_off() {
        assert!(toml::from_str::<Config>("").unwrap().tui.tap_bar);
        assert!(toml::from_str::<Config>("[tui]\ntheme = \"nord\"\n").unwrap().tui.tap_bar, "another key leaves it on");
        assert!(!toml::from_str::<Config>("[tui]\ntap_bar = false\n").unwrap().tui.tap_bar);
        assert!(!toml::to_string_pretty(&Config::default()).unwrap().contains("[tui]"), "the default is not written");
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
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[queue.rules]\nstale = 3\n").unwrap();
        assert!(Config::read(&path).unwrap().warnings[0].contains("`queue.rules.stale`"), "a typo is named");
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

    #[test]
    fn the_agent_is_off_by_default_and_its_settings_are_checked_with_their_key() {
        let load = |toml: &str| {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.toml");
            std::fs::write(&path, toml).unwrap();
            Config::load_from(&path).map_err(|e| format!("{e:#}"))
        };
        assert!(!load("").unwrap().agent.enabled);
        assert!(!toml::to_string_pretty(&Config::default()).unwrap().contains("[agent]"), "the default is not written");
        let custom =
            load("[agent]\nenabled = true\nstart = \"aider --message 'review {ref} in {dir}'\"\nopen = \"aider --restore\"\n").unwrap();
        assert_eq!(custom.agent.open.as_deref(), Some("aider --restore"));
        let typo = load("[agent]\nstart = \"aider {titel}\"\n").unwrap_err();
        assert!(typo.contains("agent.start") && typo.contains("{titel}") && typo.contains("config.toml"), "{typo}");
        let quote = load("[agent]\nstart = \"aider\"\nopen = \"aider 'again\"\n").unwrap_err();
        assert!(quote.contains("agent.open"), "{quote}");
        let alone = load("[agent]\nopen = \"aider --restore\"\n").unwrap_err();
        assert!(alone.contains("agent.open") && alone.contains("agent.start"), "{alone}");
        assert_eq!(custom.agent.show, AgentShow::Auto);
        assert_eq!(load("[agent]\nshow = \"split\"\n").unwrap().agent.show, AgentShow::Split);
        assert_eq!(load("[agent]\nshow = \"window\"\n").unwrap().agent.show, AgentShow::Window);
        let unknown = load("[agent]\nshow = \"tab\"\n").unwrap_err();
        assert!(unknown.contains("show") && unknown.contains("config.toml"), "{unknown}");
        let split = load("[agent]\nsplit = \"kitty @ launch --location=vsplit --cwd {dir} {command}\"\n").unwrap();
        assert!(split.agent.split.is_some());
        let glued = load("[agent]\nsplit = \"kitty @ launch --cmd={command}\"\n").unwrap_err();
        assert!(glued.contains("agent.split") && glued.contains("config.toml"), "{glued}");
        let relative = load("[agent.checkouts]\n\"acme/widgets\" = \"code/widgets\"\n").unwrap_err();
        assert!(relative.contains("acme/widgets") && relative.contains("full path"), "{relative}");
    }

    #[test]
    fn agent_checkouts_read_a_tilde_as_the_home_folder() {
        let config: Config =
            toml::from_str("[agent.checkouts]\n\"acme/widgets\" = \"~/code/widgets\"\n\"acme/api\" = \"/srv/api\"\n").unwrap();
        let folders = config.agent.folders(Path::new("/Users/nina"));
        assert_eq!(folders["acme/widgets"], PathBuf::from("/Users/nina/code/widgets"));
        assert_eq!(folders["acme/api"], PathBuf::from("/srv/api"));
    }
}
