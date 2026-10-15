pub(crate) mod store;

#[cfg(test)]
pub(crate) use store::MemoryStore;
pub(crate) use store::{SecretStore, SecurityCli};

use crate::config::Config;
use crate::forge::Kind;
use anyhow::{Context, Result, bail, ensure};
use std::fmt;

pub(crate) const SERVICE: &str = "revu";
const DEFAULT_HOST: &str = "gitlab.com";
const GITHUB_HOST: &str = "github.com";

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Credentials {
    pub(crate) host: String,
    pub(crate) token: String,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials").field("host", &self.host).field("token", &"<redacted>").finish()
    }
}

/// Where the token came from, so `whoami` can say it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    Env,
    Keychain,
}

/// What the resolution looks at around the process: token and host variables, and the host of
/// the checkout it runs in. A struct, so tests build one without `set_var` or a git repo.
#[derive(Clone, Default)]
pub(crate) struct Env {
    /// `GITLAB_TOKEN`, for the `GITLAB_HOST` host, else gitlab.com.
    pub(crate) gitlab_token: Option<String>,
    /// `GITHUB_TOKEN`, else `GH_TOKEN`, for github.com.
    pub(crate) github_token: Option<String>,
    /// `GITLAB_HOST`.
    pub(crate) host: Option<String>,
    /// The host of the `origin` remote of the checkout under the working directory.
    pub(crate) origin: Option<String>,
}

impl fmt::Debug for Env {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let redacted = |token: &Option<String>| token.as_ref().map(|_| "<redacted>");
        f.debug_struct("Env")
            .field("gitlab_token", &redacted(&self.gitlab_token))
            .field("github_token", &redacted(&self.github_token))
            .field("host", &self.host)
            .field("origin", &self.origin)
            .finish()
    }
}

impl Env {
    pub(crate) fn from_process() -> Self {
        let read = |key: &str| std::env::var(key).ok().filter(|v| !v.is_empty());
        let origin = std::env::current_dir().ok().and_then(|dir| crate::mrref::origin_host(&dir));
        Self {
            gitlab_token: read("GITLAB_TOKEN"),
            github_token: read("GITHUB_TOKEN").or_else(|| read("GH_TOKEN")),
            host: read("GITLAB_HOST"),
            origin,
        }
    }

    /// A token variable goes only to the host `--host` or `GITLAB_HOST` names, else its forge's default
    /// host: the config's `host` keeps its keychain entry, so a token never reaches a host it was not minted for.
    fn token_for(&self, host: &str, config: &Config, named: Option<&str>) -> Option<&String> {
        let kind = Kind::for_host(host, config);
        let token = match kind {
            Kind::GitLab => self.gitlab_token.as_ref(),
            Kind::GitHub => self.github_token.as_ref(),
        };
        let target = named.or(self.host.as_deref()).filter(|h| Kind::for_host(h, config) == kind).unwrap_or(default_host(kind));
        token.filter(|_| host.eq_ignore_ascii_case(target))
    }
}

fn default_host(kind: Kind) -> &'static str {
    match kind {
        Kind::GitLab => DEFAULT_HOST,
        Kind::GitHub => GITHUB_HOST,
    }
}

/// The token variable of each forge, for messages.
pub(crate) fn token_variable(kind: Kind) -> &'static str {
    match kind {
        Kind::GitLab => "GITLAB_TOKEN",
        Kind::GitHub => "GITHUB_TOKEN",
    }
}

/// How to give `host` a token without the keychain, said after the login command.
fn variable_hint(host: &str, kind: Kind) -> String {
    let variable = token_variable(kind);
    if host.eq_ignore_ascii_case(default_host(kind)) {
        return format!(" or set {variable}");
    }
    match kind {
        Kind::GitLab => format!(" or set {variable} and GITLAB_HOST={host}"),
        Kind::GitHub => format!(" or set {variable} with --host {host}"),
    }
}

/// The host this run is for, with the token its variable gives it (see `Env::token_for`), else its keychain entry.
pub(crate) fn resolve(env: &Env, store: &dyn SecretStore, config: &Config, named: Option<&str>) -> Result<(Credentials, Source)> {
    let token_for = |h: &str| env.token_for(h, config, named);
    let has_token = |h: &str| token_for(h).is_some() || store.get(h).ok().flatten().is_some();
    let host = pick_host(env, config, named, has_token)?;
    if let Some(token) = token_for(&host) {
        return Ok((Credentials { host, token: token.clone() }, Source::Env));
    }
    match store.get(&host)? {
        Some(token) => Ok((Credentials { host, token }, Source::Keychain)),
        None => bail!("no token for {host}: run `revu login {host}`{}", variable_hint(&host, Kind::for_host(&host, config))),
    }
}

/// An extra host the config lists: its keychain entry only, since a token variable goes to one host.
pub(crate) fn resolve_extra(store: &dyn SecretStore, host: &str) -> Result<Credentials> {
    check_host(host)?;
    let token = store.get(host)?.with_context(|| format!("no token for {host}: run `revu login {host}`"))?;
    Ok(Credentials { host: host.to_owned(), token })
}

/// The flag, then `GITLAB_HOST`, then the checkout's own host when `usable` says a token is there
/// for it, then the configured host, then gitlab.com.
pub(crate) fn pick_host(env: &Env, config: &Config, host: Option<&str>, usable: impl Fn(&str) -> bool) -> Result<String> {
    let host = host
        .map(str::to_owned)
        .or_else(|| env.host.clone())
        .or_else(|| env.origin.clone().filter(|origin| usable(origin)))
        .or_else(|| config.host.clone())
        .unwrap_or_else(|| DEFAULT_HOST.to_owned());
    check_host(&host)?;
    Ok(host)
}

/// A host names a cache folder, so only a plain `name` or `name:port` passes: `..`, `/` or a path
/// would reach outside the cache. The URL parser keeps `.` and `..` as names, hence the dot rule.
pub(crate) fn check_host(host: &str) -> Result<()> {
    let parsed = url::Url::parse(&format!("https://{host}")).ok();
    let whole = parsed.and_then(|url| authority(&url)).is_some_and(|authority| authority.eq_ignore_ascii_case(host));
    let dotted = host.starts_with('.') || host.contains("..");
    ensure!(whole && !dotted, "not a host name: {host:?}");
    Ok(())
}

fn authority(url: &url::Url) -> Option<String> {
    let name = url.host_str()?;
    Some(url.port().map_or_else(|| name.to_owned(), |port| format!("{name}:{port}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Host;

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
    fn a_token_variable_goes_to_the_host_named_by_the_flag() {
        let env = Env { gitlab_token: Some("glpat-corp".into()), ..Env::default() };
        let (corp, source) = resolve(&env, &MemoryStore::default(), &Config::default(), Some("gitlab.corp.example")).unwrap();
        assert_eq!((corp.token.as_str(), source), ("glpat-corp", Source::Env));
    }

    #[test]
    fn an_extra_host_uses_its_keychain_entry_never_the_token_variable() {
        let store = MemoryStore::default();
        assert!(resolve_extra(&store, "gitlab.com").is_err(), "no keychain entry, no token");
        store.set("gitlab.com", "glpat-keychain").unwrap();
        assert_eq!(resolve_extra(&store, "gitlab.com").unwrap().token, "glpat-keychain");
    }

    #[test]
    fn the_config_host_never_gets_the_token_variable() {
        let config = Config { host: Some("gl.acme.dev".into()), ..Config::default() };
        let env = Env { gitlab_token: Some("glpat-env".into()), ..Env::default() };
        let store = MemoryStore::default();
        let err = resolve(&env, &store, &config, None).unwrap_err().to_string();
        assert!(err.contains("revu login gl.acme.dev") && err.contains("GITLAB_HOST=gl.acme.dev"), "{err}");
        store.set("gl.acme.dev", "glpat-keychain").unwrap();
        let (creds, source) = resolve(&env, &store, &config, None).unwrap();
        assert_eq!((creds.host.as_str(), creds.token.as_str(), source), ("gl.acme.dev", "glpat-keychain", Source::Keychain));
    }

    #[test]
    fn a_token_variable_goes_to_the_host_gitlab_host_names() {
        let config = Config { host: Some("gitlab.com".into()), ..Config::default() };
        let env = Env { gitlab_token: Some("glpat-env".into()), host: Some("gl.acme.dev".into()), ..Env::default() };
        let (creds, source) = resolve(&env, &MemoryStore::default(), &config, None).unwrap();
        assert_eq!((creds.host.as_str(), source), ("gl.acme.dev", Source::Env));
    }

    #[test]
    fn a_gitlab_com_checkout_gets_its_token_variable_over_the_config_host() {
        let config = Config { host: Some("gl.acme.dev".into()), ..Config::default() };
        let env = Env { gitlab_token: Some("glpat-env".into()), origin: Some("gitlab.com".into()), ..Env::default() };
        let store = MemoryStore::default();
        store.set("gl.acme.dev", "glpat-keychain").unwrap();
        let (creds, source) = resolve(&env, &store, &config, None).unwrap();
        assert_eq!((creds.host.as_str(), creds.token.as_str(), source), ("gitlab.com", "glpat-env", Source::Env));
    }

    #[test]
    fn a_github_enterprise_config_host_never_gets_the_github_token_variable() {
        let config = Config {
            host: Some("git.acme.dev".into()),
            hosts: [("git.acme.dev".into(), Host { forge: Some(Kind::GitHub), ..Host::default() })].into(),
            ..Config::default()
        };
        let env = Env { github_token: Some("ghp_env".into()), ..Env::default() };
        let err = resolve(&env, &MemoryStore::default(), &config, None).unwrap_err().to_string();
        assert!(err.contains("revu login git.acme.dev") && err.contains("--host git.acme.dev"), "{err}");
    }

    #[test]
    fn a_checkout_host_other_than_the_default_never_gets_the_token_variable() {
        let env = Env { gitlab_token: Some("glpat-env".into()), origin: Some("gl.acme.dev".into()), ..Env::default() };
        let store = MemoryStore::default();
        assert_eq!(resolve(&env, &store, &Config::default(), None).unwrap().0.host, "gitlab.com", "no token for the checkout's host");
        store.set("gl.acme.dev", "glpat-keychain").unwrap();
        let (creds, source) = resolve(&env, &store, &Config::default(), None).unwrap();
        assert_eq!((creds.host.as_str(), creds.token.as_str(), source), ("gl.acme.dev", "glpat-keychain", Source::Keychain));
    }

    #[test]
    fn a_github_token_variable_keeps_github_com_when_the_stated_host_is_gitlab() {
        let config = Config { host: Some("gl.acme.dev".into()), ..Config::default() };
        let env = Env { github_token: Some("ghp_xxxx".into()), origin: Some("github.com".into()), ..Env::default() };
        assert_eq!(resolve(&env, &MemoryStore::default(), &config, None).unwrap().0.host, "github.com");
    }

    #[test]
    fn a_github_token_variable_goes_to_the_named_github_host() {
        let config =
            Config { hosts: [("git.acme.dev".into(), Host { forge: Some(Kind::GitHub), ..Host::default() })].into(), ..Config::default() };
        let env = Env { github_token: Some("ghp_xxxx".into()), ..Env::default() };
        assert_eq!(resolve(&env, &MemoryStore::default(), &config, Some("git.acme.dev")).unwrap().0.token, "ghp_xxxx");
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
    fn only_a_plain_host_name_passes() {
        for host in ["gitlab.com", "gl.acme.dev", "gitlab.acme.dev:8443", "GitLab.com", "localhost", "[fd00::1]:8443"] {
            assert!(check_host(host).is_ok(), "{host}");
        }
        for host in ["", ".", "..", "/", "../etc", "a/b", "a\\b", "a..b", "nina@gitlab.com", "gitlab.com?x", "gitlab.com:x", "gitlab.com/"]
        {
            assert!(check_host(host).is_err(), "{host:?}");
        }
    }

    #[test]
    fn a_path_as_host_is_refused_wherever_it_comes_from() {
        let config = Config { host: Some("..".into()), ..Config::default() };
        assert!(pick_host(&Env::default(), &config, None, |_| true).is_err(), "from the config");
        let env = Env { host: Some("/".into()), ..Env::default() };
        assert!(pick_host(&env, &Config::default(), None, |_| true).is_err(), "from GITLAB_HOST");
        assert!(pick_host(&Env::default(), &Config::default(), Some("../x"), |_| true).is_err(), "from the flag");
    }

    #[test]
    fn debug_never_prints_the_token() {
        let creds = Credentials { host: "gitlab.com".into(), token: "glpat-secret".into() };
        assert!(!format!("{creds:?}").contains("secret"));
    }

    #[test]
    fn debug_of_the_env_never_prints_its_tokens() {
        let env = Env { gitlab_token: Some("glpat-xxxx".into()), github_token: Some("ghp_xxxx".into()), ..Env::default() };
        let printed = format!("{env:?}");
        assert!(!printed.contains("glpat-xxxx") && !printed.contains("ghp_xxxx"), "{printed}");
    }
}
