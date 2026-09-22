pub mod store;

#[cfg(test)]
pub use store::MemoryStore;
pub use store::{SecretStore, SecurityCli};

use crate::config::Config;
use anyhow::{Result, bail};
use std::fmt;

pub const SERVICE: &str = "gitlabmr";
pub const DEFAULT_HOST: &str = "gitlab.com";

#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    pub host: String,
    pub token: String,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials").field("host", &self.host).field("token", &"<redacted>").finish()
    }
}

/// Where the token came from, so `whoami` can say it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Env,
    Keychain,
}

/// The process environment the resolution looks at; a struct so tests build one without `set_var`.
#[derive(Clone, Debug, Default)]
pub struct Env {
    pub token: Option<String>,
    pub host: Option<String>,
}

impl Env {
    pub fn from_process() -> Self {
        let read = |key: &str| std::env::var(key).ok().filter(|v| !v.is_empty());
        Self { token: read("GITLAB_TOKEN"), host: read("GITLAB_HOST") }
    }
}

/// `GITLAB_TOKEN` wins, then the keychain entry for the chosen host.
/// The host is the flag, then `GITLAB_HOST`, then the config, then gitlab.com.
pub fn resolve(env: &Env, store: &dyn SecretStore, config: &Config, host: Option<&str>) -> Result<(Credentials, Source)> {
    let host = pick_host(env, config, host);
    if let Some(token) = &env.token {
        return Ok((Credentials { host, token: token.clone() }, Source::Env));
    }
    match store.get(&host)? {
        Some(token) => Ok((Credentials { host, token }, Source::Keychain)),
        None => bail!("no token for {host}: run `mr login {host}` or set GITLAB_TOKEN"),
    }
}

pub fn pick_host(env: &Env, config: &Config, host: Option<&str>) -> String {
    host.map(str::to_owned).or_else(|| env.host.clone()).or_else(|| config.host.clone()).unwrap_or_else(|| DEFAULT_HOST.to_owned())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn env_token_wins_over_keychain() {
        let store = MemoryStore::default();
        store.set("gitlab.com", "glpat-keychain").unwrap();
        let env = Env { token: Some("glpat-env".into()), host: None };
        let (creds, source) = resolve(&env, &store, &Config::default(), None).unwrap();
        assert_eq!(creds.token, "glpat-env");
        assert_eq!(source, Source::Env);
    }

    #[test]
    fn keychain_entry_is_looked_up_by_host() {
        let store = MemoryStore::default();
        store.set("gitlab.example.com", "glpat-xxxx").unwrap();
        let config = Config { host: Some("gitlab.example.com".into()), ..Config::default() };
        let (creds, source) = resolve(&Env::default(), &store, &config, None).unwrap();
        assert_eq!(creds.host, "gitlab.example.com");
        assert_eq!(creds.token, "glpat-xxxx");
        assert_eq!(source, Source::Keychain);
    }

    #[test]
    fn missing_token_names_the_login_command() {
        let err = resolve(&Env::default(), &MemoryStore::default(), &Config::default(), Some("gl.acme.dev")).unwrap_err();
        assert!(err.to_string().contains("mr login gl.acme.dev"), "{err}");
    }

    #[test]
    fn debug_never_prints_the_token() {
        let creds = Credentials { host: "gitlab.com".into(), token: "glpat-secret".into() };
        assert!(!format!("{creds:?}").contains("secret"));
    }
}
