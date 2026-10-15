use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `[share]`: `Y` posts an MR by piping a message to a command, so revu knows no chat tool.
/// A bare `command` is the one target; `[share.targets.<name>]` adds named ones, and `Y` asks which.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Share {
    /// Run as words, never through a shell; the message arrives on its stdin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) command: Option<String>,
    /// The message, with placeholders like `{ref}` and `{title}`; also the template of named targets that set none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) template: Option<String>,
    /// Named destinations, each with its own command and, optionally, its own template.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) targets: BTreeMap<String, ShareTarget>,
}

/// `[share.targets.<name>]`: one more place `Y` can post to.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ShareTarget {
    /// Run as words, never through a shell; the message arrives on its stdin.
    pub(crate) command: String,
    /// This target's message; `[share] template`, then the built-in one, when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) template: Option<String>,
}

impl Share {
    pub(super) fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Every command splits into words and every template names only placeholders revu fills.
    pub(super) fn check(&self) -> Result<()> {
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
pub(crate) struct Agent {
    /// Off by default: nothing runs until this is true.
    #[serde(default)]
    pub(crate) enabled: bool,
    /// Another agent than Claude Code: the command that starts its session, run as words in the MR's worktree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) start: Option<String>,
    /// The command that opens again the session `start` made; `start` runs again without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) open: Option<String>,
    /// Project path → its checkout on this machine, for MRs of a project revu was not started in.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) checkouts: BTreeMap<String, String>,
    /// Where the session opens: beside revu when it fits, always beside, or never.
    #[serde(default, skip_serializing_if = "AgentShow::is_default")]
    pub(crate) show: AgentShow,
    /// For a terminal revu cannot split itself: the command that opens a pane beside revu and runs `{command}` there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) split: Option<String>,
}

/// `[agent] show`: a pane beside revu when revu is wide enough for both, at any width, or never.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AgentShow {
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
    pub(super) fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Both commands split into words and name only placeholders revu fills; every checkout is a full path.
    pub(super) fn check(&self) -> Result<()> {
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
    pub(crate) fn folders(&self, home: &Path) -> BTreeMap<String, PathBuf> {
        let folder = |written: &String| written.strip_prefix("~/").map_or_else(|| PathBuf::from(written), |rest| home.join(rest));
        self.checkouts.iter().map(|(project, written)| (project.clone(), folder(written))).collect()
    }
}

/// `^v`: which program opens a file, by glob on its path, as in `"*.md" = "glow -p"`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Open {
    /// For every file no glob in `files` matches; `$VISUAL`, `$EDITOR`, then `less` without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) default: Option<String>,
    /// Glob → command; the longest matching glob wins.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) files: BTreeMap<String, String>,
}

impl Open {
    pub(super) fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Every command splits into words with known placeholders, every glob parses.
    pub(super) fn check(&self) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

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
