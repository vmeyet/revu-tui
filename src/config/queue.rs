use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Which MRs the queue shows beyond the ones GitLab lists for me.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Queue {
    /// MRs carrying one of these labels land in Watching.
    #[serde(default)]
    pub(crate) watch_labels: Vec<String>,
    /// Saved filters: `'` then a view's first letter, or `1`–`9` in name order, applies it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) views: BTreeMap<String, String>,
    /// The "needs me" rules: what leaves To review, Watching and Open, and at which thresholds.
    #[serde(default, skip_serializing_if = "crate::forge::rules::Rules::is_default")]
    pub(crate) rules: crate::forge::rules::Rules,
    /// Where "ready for review" comes from: a command printing MR links.
    #[serde(default, skip_serializing_if = "Ready::is_default")]
    pub(crate) ready: Ready,
    /// How many MRs that need me are loaded ahead, so opening them needs no network; 0 turns it off.
    #[serde(default = "default_prefetch", skip_serializing_if = "is_default_prefetch")]
    pub(crate) prefetch: usize,
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
pub(crate) struct Ready {
    /// Run as words, never through a shell; every MR link it prints counts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) command: Option<String>,
}

impl Ready {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

impl Queue {
    pub(super) fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Every view is a query that parses, and no two start with the same letter, since `'` picks by it.
    pub(super) fn check(&self) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use crate::config::Config;

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
}
