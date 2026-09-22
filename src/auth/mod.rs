pub mod store;

#[cfg(test)]
pub use store::MemoryStore;
pub use store::{SecretStore, SecurityCli};

use crate::config::Config;
use crate::forge::Kind;
use anyhow::{Result, bail};
use std::fmt;

pub const SERVICE: &str = "revu";
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

/// What the resolution looks at around the process: token and host variables, and the host of
/// the checkout it runs in. A struct, so tests build one without `set_var` or a git repo.
#[derive(Clone, Debug, Default)]
pub struct Env {
    /// `GITLAB_TOKEN`, for GitLab hosts.
    pub gitlab_token: Option<String>,
    /// `GITHUB_TOKEN`, else `GH_TOKEN`, for GitHub hosts.
    pub github_token: Option<String>,
    /// `GITLAB_HOST`.
    pub host: Option<String>,
    /// The host of the `origin` remote of the checkout under the working directory.
    pub origin: Option<String>,
}

impl Env {
    pub fn from_process() -> Self {
        let read = |key: &str| std::env::var(key).ok().filter(|v| !v.is_empty());
        let origin = std::env::current_dir().ok().and_then(|dir| crate::mrref::origin_host(&dir));
        Self {
            gitlab_token: read("GITLAB_TOKEN"),
            github_token: read("GITHUB_TOKEN").or_else(|| read("GH_TOKEN")),
            host: read("GITLAB_HOST"),
            origin,
        }
    }

    fn token_for(&self, kind: Kind) -> Option<&String> {
        match kind {
            Kind::GitLab => self.gitlab_token.as_ref(),
            Kind::GitHub => self.github_token.as_ref(),
        }
    }
}

/// The token variable of each forge, for messages.
pub fn token_variable(kind: Kind) -> &'static str {
    match kind {
        Kind::GitLab => "GITLAB_TOKEN",
        Kind::GitHub => "GITHUB_TOKEN",
    }
}

/// The token variable of the host's forge wins, then the keychain entry for the chosen host.
pub fn resolve(env: &Env, store: &dyn SecretStore, config: &Config, host: Option<&str>) -> Result<(Credentials, Source)> {
    let has_token = |h: &str| env.token_for(Kind::for_host(h, config)).is_some() || store.get(h).ok().flatten().is_some();
    let host = pick_host(env, config, host, has_token);
    let kind = Kind::for_host(&host, config);
    if let Some(token) = env.token_for(kind) {
        return Ok((Credentials { host, token: token.clone() }, Source::Env));
    }
    match store.get(&host)? {
        Some(token) => Ok((Credentials { host, token }, Source::Keychain)),
        None => bail!("no token for {host}: run `revu login {host}` or set {}", token_variable(kind)),
    }
}

/// The flag, then `GITLAB_HOST`, then the checkout's own host when `usable` says a token is there
/// for it, then the configured host, then gitlab.com.
pub fn pick_host(env: &Env, config: &Config, host: Option<&str>, usable: impl Fn(&str) -> bool) -> String {
    host.map(str::to_owned)
        .or_else(|| env.host.clone())
        .or_else(|| env.origin.clone().filter(|origin| usable(origin)))
        .or_else(|| config.host.clone())
        .unwrap_or_else(|| DEFAULT_HOST.to_owned())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn env_token_wins_over_keychain() {
        let store = MemoryStore::default();
        store.set("gitlab.com", "glpat-keychain").unwrap();
        let env = Env { gitlab_token: Some("glpat-env".into()), ..Env::default() };
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
        assert!(err.to_string().contains("revu login gl.acme.dev"), "{err}");
    }

    #[test]
    fn each_forge_reads_its_own_token_variable() {
        let env = Env { gitlab_token: Some("glpat-env".into()), github_token: Some("ghp_env".into()), ..Env::default() };
        let store = MemoryStore::default();
        let (github, _) = resolve(&env, &store, &Config::default(), Some("github.com")).unwrap();
        assert_eq!(github.token, "ghp_env");
        let (gitlab, _) = resolve(&env, &store, &Config::default(), Some("gitlab.com")).unwrap();
        assert_eq!(gitlab.token, "glpat-env");
        let err = resolve(&Env::default(), &store, &Config::default(), Some("github.com")).unwrap_err().to_string();
        assert!(err.contains("GITHUB_TOKEN") && err.contains("revu login github.com"), "{err}");
    }

    #[test]
    fn the_checkout_host_wins_only_when_a_token_is_there_for_it() {
        let store = MemoryStore::default();
        let config = Config { host: Some("gitlab.com".into()), ..Config::default() };
        let env = Env { origin: Some("github.com".into()), ..Env::default() };
        store.set("gitlab.com", "glpat-xxxx").unwrap();
        assert_eq!(resolve(&env, &store, &config, None).unwrap().0.host, "gitlab.com", "no GitHub token yet");
        store.set("github.com", "ghp_xxxx").unwrap();
        assert_eq!(resolve(&env, &store, &config, None).unwrap().0.host, "github.com");
        assert_eq!(resolve(&env, &store, &config, Some("gitlab.com")).unwrap().0.host, "gitlab.com", "the flag still wins");
    }

    #[test]
    fn debug_never_prints_the_token() {
        let creds = Credentials { host: "gitlab.com".into(), token: "glpat-secret".into() };
        assert!(!format!("{creds:?}").contains("secret"));
    }
}
