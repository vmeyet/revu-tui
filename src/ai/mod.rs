//! The two AI providers: which one is on, and where its key comes from.
//! Keys live in the keychain (service `revu`, one account per provider) or an environment
//! variable; they never touch the config, the cache or a log line.
pub mod triage;
pub mod typesafe;

use crate::auth::SecretStore;
use crate::config::Ai;
use anyhow::Result;
use serde::Serialize;
use std::fmt;

/// An AI provider revu can talk to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    /// TypeSafe's Jev: typed judgments that rank the queue and the files.
    Typesafe,
    /// Anthropic's Claude: questions about the MR.
    Anthropic,
}

impl Provider {
    pub const ALL: [Provider; 2] = [Provider::Typesafe, Provider::Anthropic];

    /// The keychain account under service `revu`.
    pub fn account(self) -> &'static str {
        match self {
            Provider::Typesafe => "typesafe",
            Provider::Anthropic => "anthropic",
        }
    }

    /// The variable that overrides the keychain.
    pub fn variable(self) -> &'static str {
        match self {
            Provider::Typesafe => "TYPESAFE_API_KEY",
            Provider::Anthropic => "ANTHROPIC_API_KEY",
        }
    }

    pub fn enabled(self, ai: &Ai) -> bool {
        match self {
            Provider::Typesafe => ai.typesafe.enabled,
            Provider::Anthropic => ai.anthropic.enabled,
        }
    }

    /// Where a key for it is made, for the login prompt.
    pub fn key_page(self) -> &'static str {
        match self {
            Provider::Typesafe => "https://typesafe.ai",
            Provider::Anthropic => "https://console.anthropic.com/settings/keys",
        }
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.account())
    }
}

/// A key, printable only as `<redacted>`.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    /// The raw key, for the one header that carries it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Where a key was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum KeySource {
    Env,
    Keychain,
}

/// The variables the key lookup reads; a struct so tests build one without `set_var`.
#[derive(Clone, Debug, Default)]
pub struct KeyEnv {
    pub typesafe: Option<String>,
    pub anthropic: Option<String>,
}

impl KeyEnv {
    pub fn from_process() -> Self {
        let read = |p: Provider| std::env::var(p.variable()).ok().filter(|v| !v.trim().is_empty());
        Self { typesafe: read(Provider::Typesafe), anthropic: read(Provider::Anthropic) }
    }

    fn get(&self, provider: Provider) -> Option<&String> {
        match provider {
            Provider::Typesafe => self.typesafe.as_ref(),
            Provider::Anthropic => self.anthropic.as_ref(),
        }
    }
}

/// The provider's key: its variable wins, then the keychain.
pub fn key(provider: Provider, env: &KeyEnv, store: &dyn SecretStore) -> Result<Option<(Secret, KeySource)>> {
    if let Some(key) = env.get(provider) {
        return Ok(Some((Secret::new(key.trim()), KeySource::Env)));
    }
    Ok(store.get(provider.account())?.map(|key| (Secret::new(key), KeySource::Keychain)))
}

/// What `revu ai status` shows for one provider: never the key itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Status {
    pub provider: Provider,
    pub enabled: bool,
    pub key: Option<KeySource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

pub fn status(ai: &Ai, env: &KeyEnv, store: &dyn SecretStore) -> Vec<Status> {
    Provider::ALL
        .iter()
        .map(|&provider| Status {
            provider,
            enabled: provider.enabled(ai),
            key: key(provider, env, store).ok().flatten().map(|(_, source)| source),
            model: (provider == Provider::Anthropic).then(|| ai.anthropic.model().to_owned()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::auth::MemoryStore;

    #[test]
    fn the_variable_wins_over_the_keychain() {
        let store = MemoryStore::default();
        store.set("anthropic", "sk-ant-keychain").unwrap();
        let env = KeyEnv { anthropic: Some("sk-ant-env".into()), ..KeyEnv::default() };
        let (key, source) = key(Provider::Anthropic, &env, &store).unwrap().unwrap();
        assert_eq!((key.expose(), source), ("sk-ant-env", KeySource::Env));
        let (key, source) = super::key(Provider::Anthropic, &KeyEnv::default(), &store).unwrap().unwrap();
        assert_eq!((key.expose(), source), ("sk-ant-keychain", KeySource::Keychain));
        assert!(super::key(Provider::Typesafe, &KeyEnv::default(), &store).unwrap().is_none());
    }

    #[test]
    fn status_reports_the_switch_and_the_key_source_apart() {
        let store = MemoryStore::default();
        store.set("typesafe", "ts-key").unwrap();
        let mut ai = Ai::default();
        ai.anthropic.enabled = true;
        let [typesafe, anthropic] = status(&ai, &KeyEnv::default(), &store).try_into().unwrap();
        assert_eq!((typesafe.enabled, typesafe.key), (false, Some(KeySource::Keychain)));
        assert_eq!((anthropic.enabled, anthropic.key), (true, None));
        assert_eq!(anthropic.model.as_deref(), Some("claude-opus-5"));
    }

    #[test]
    fn a_secret_never_prints() {
        let secret = Secret::new("sk-ant-api03-secret");
        assert_eq!(format!("{secret:?}"), "<redacted>");
        let status = Status { provider: Provider::Anthropic, enabled: true, key: Some(KeySource::Env), model: None };
        assert!(!serde_json::to_string(&status).unwrap().contains("sk-ant"));
    }
}
