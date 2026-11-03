//! The AI providers: which one is on, and where its key comes from.
//! Keys live in the keychain (service `revu`, one account per provider) or an environment
//! variable; they never touch the config, the cache or a log line.
#[expect(dead_code, reason = "nothing asks Claude any more; the next change in the stack deletes it")]
pub(crate) mod anthropic;
pub(crate) mod triage;
pub(crate) mod typesafe;

use crate::auth::SecretStore;
use crate::config::Ai;
use anyhow::Result;
use serde::Serialize;
use std::fmt;

/// An AI provider revu can talk to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Provider {
    /// TypeSafe's Jev: typed judgments that rank the queue and the files.
    Typesafe,
}

impl Provider {
    pub(crate) const ALL: [Provider; 1] = [Provider::Typesafe];

    /// The keychain account under service `revu`.
    pub(crate) fn account(self) -> &'static str {
        match self {
            Provider::Typesafe => "typesafe",
        }
    }

    /// The variable that overrides the keychain.
    pub(crate) fn variable(self) -> &'static str {
        match self {
            Provider::Typesafe => "TYPESAFE_API_KEY",
        }
    }

    pub(crate) fn enabled(self, ai: &Ai) -> bool {
        match self {
            Provider::Typesafe => ai.typesafe.enabled,
        }
    }

    /// Where a key for it is made, for the login prompt.
    pub(crate) fn key_page(self) -> &'static str {
        match self {
            Provider::Typesafe => "https://typesafe.ai",
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
pub(crate) struct Secret(String);

impl Secret {
    pub(crate) fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    /// The raw key, for the one header that carries it.
    pub(crate) fn expose(&self) -> &str {
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
pub(crate) enum KeySource {
    Env,
    Keychain,
}

/// The variables the key lookup reads; a struct so tests build one without `set_var`.
#[derive(Clone, Debug, Default)]
pub(crate) struct KeyEnv {
    pub(crate) typesafe: Option<String>,
}

impl KeyEnv {
    pub(crate) fn from_process() -> Self {
        let read = |p: Provider| std::env::var(p.variable()).ok().filter(|v| !v.trim().is_empty());
        Self { typesafe: read(Provider::Typesafe) }
    }

    fn get(&self, provider: Provider) -> Option<&String> {
        match provider {
            Provider::Typesafe => self.typesafe.as_ref(),
        }
    }
}

/// The provider's key: its variable wins, then the keychain.
pub(crate) fn key(provider: Provider, env: &KeyEnv, store: &dyn SecretStore) -> Result<Option<(Secret, KeySource)>> {
    if let Some(key) = env.get(provider) {
        return Ok(Some((Secret::new(key.trim()), KeySource::Env)));
    }
    Ok(store.get(provider.account())?.map(|key| (Secret::new(key), KeySource::Keychain)))
}

/// What `revu ai status` shows for one provider: never the key itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Status {
    pub(crate) provider: Provider,
    pub(crate) enabled: bool,
    pub(crate) key: Option<KeySource>,
}

pub(crate) fn status(ai: &Ai, env: &KeyEnv, store: &dyn SecretStore) -> Vec<Status> {
    Provider::ALL
        .iter()
        .map(|&provider| Status {
            provider,
            enabled: provider.enabled(ai),
            key: key(provider, env, store).ok().flatten().map(|(_, source)| source),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::MemoryStore;

    #[test]
    fn the_variable_wins_over_the_keychain() {
        let store = MemoryStore::default();
        store.set("typesafe", "ts-keychain").unwrap();
        let env = KeyEnv { typesafe: Some("ts-env".into()) };
        let (key, source) = key(Provider::Typesafe, &env, &store).unwrap().unwrap();
        assert_eq!((key.expose(), source), ("ts-env", KeySource::Env));
        let (key, source) = super::key(Provider::Typesafe, &KeyEnv::default(), &store).unwrap().unwrap();
        assert_eq!((key.expose(), source), ("ts-keychain", KeySource::Keychain));
        assert!(super::key(Provider::Typesafe, &KeyEnv::default(), &MemoryStore::default()).unwrap().is_none());
    }

    #[test]
    fn status_reports_the_switch_and_the_key_source_apart() {
        let store = MemoryStore::default();
        store.set("typesafe", "ts-key").unwrap();
        let [off] = status(&Ai::default(), &KeyEnv::default(), &store).try_into().unwrap();
        assert_eq!((off.enabled, off.key), (false, Some(KeySource::Keychain)));
        let mut ai = Ai::default();
        ai.typesafe.enabled = true;
        let [on] = status(&ai, &KeyEnv::default(), &MemoryStore::default()).try_into().unwrap();
        assert_eq!((on.enabled, on.key), (true, None));
    }

    #[test]
    fn a_secret_never_prints() {
        let secret = Secret::new("ts-secret");
        assert_eq!(format!("{secret:?}"), "<redacted>");
        let status = Status { provider: Provider::Typesafe, enabled: true, key: Some(KeySource::Env) };
        assert!(!serde_json::to_string(&status).unwrap().contains("ts-secret"));
    }
}
