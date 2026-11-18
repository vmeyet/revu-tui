//! What every command opens first: the forge, the config, the cache and the checkout's project.
use crate::auth::{self, Credentials, Env, HostOrigin, Login, SecretStore, SecurityCli, Source};
use crate::cache::Cache;
use crate::config::Config;
use crate::forge::{Forge, Kind, Mr, MrKey, NotFound, ProjectPath, local};
use crate::log::OrWarn as _;
use anyhow::Result;
use serde::Serialize;

/// Everything a command needs, opened once from the config, the environment and the keychain.
#[derive(Debug)]
pub struct Ctx {
    pub(crate) forge: Forge,
    pub(crate) credentials: Credentials,
    pub(crate) source: Source,
    pub(crate) host_origin: HostOrigin,
    pub(crate) config: Config,
    pub(crate) cache: Cache,
    pub(crate) json: bool,
    /// The project of the checkout the command runs in: the queue shows only it unless `--all`.
    pub(crate) project: Option<ProjectPath>,
}

impl Ctx {
    /// Reads the config, finds the token and the checkout's project.
    pub fn open(host: Option<&str>, json: bool) -> Result<Self> {
        let config = Config::load()?;
        let store = SecurityCli::new(auth::SERVICE);
        let ctx = Self::build(&Env::from_process(), &store, config, host, json)?;
        let project = std::env::current_dir().ok().and_then(|dir| crate::mrref::checkout_project(&dir, &ctx.credentials.host));
        Ok(Self { project, ..ctx })
    }

    /// `revu local`: no token, no other host, no Jev and nothing that only makes sense on a forge.
    pub(crate) fn local(client: local::Client) -> Result<Self> {
        let mut config = Config::load()?;
        config.host = None;
        config.hosts.clear();
        config.ai.typesafe.enabled = false;
        config.notify.enabled = false;
        config.queue.prefetch = 0;
        config.queue.rules.enabled = false;
        config.queue.ready.command = None;
        Ok(Self {
            forge: Forge::Local(client),
            credentials: Credentials { host: local::HOST.to_owned(), token: String::new() },
            source: Source::Env,
            host_origin: HostOrigin::Named,
            config,
            cache: Cache::for_host(local::HOST)?,
            json: false,
            project: None,
        })
    }

    /// `--all`: every project, wherever the command runs.
    pub fn everywhere(self, all: bool) -> Self {
        if all { Self { project: None, ..self } } else { self }
    }

    /// `err` saying why this host was asked and how to ask another, when the forge did not find the MR.
    pub fn with_host_hint(&self, err: anyhow::Error) -> anyhow::Error {
        with_host_hint(self.host_origin, err)
    }

    pub(crate) fn build(env: &Env, store: &dyn SecretStore, config: Config, host: Option<&str>, json: bool) -> Result<Self> {
        let Login { credentials, source, host_origin } = auth::resolve(env, store, &config, host)?;
        let cache = Cache::for_host(&credentials.host)?;
        let forge = Forge::connect(Kind::for_host(&credentials.host, &config), &credentials)?;
        Ok(Self { forge, credentials, source, host_origin, config, cache, json, project: None })
    }
}

/// `err` in words the reader can act on when it is a `NotFound`: the host asked, why it was, and how to ask
/// another. A named host gets no such hint, and any other error passes through.
pub(crate) fn with_host_hint(origin: HostOrigin, err: anyhow::Error) -> anyhow::Error {
    let missing = match err.downcast::<NotFound>() {
        Ok(missing) => missing,
        Err(other) => return other,
    };
    let why = match origin {
        HostOrigin::Named => return anyhow::anyhow!("{missing}, or this token cannot see it"),
        HostOrigin::Checkout => "the host of this checkout",
        HostOrigin::Config => "the host in the config",
        HostOrigin::Default => "the default host",
    };
    anyhow::anyhow!("{missing} ({why}): pass --host to ask another forge")
}

/// Another host I am logged in to, ready for the queue to ask.
#[derive(Clone)]
pub(crate) struct Home {
    pub(crate) host: String,
    pub(crate) forge: Forge,
    pub(crate) cache: Cache,
}

impl Home {
    fn connect(host: &str, config: &Config, credentials: &Credentials) -> Result<Self> {
        let forge = Forge::connect(Kind::for_host(host, config), credentials)?;
        Ok(Self { host: host.to_owned(), forge, cache: Cache::for_host(host)? })
    }

    /// This host's queue across every project, each row tagged with the host.
    pub(crate) async fn queue(&self) -> Result<crate::forge::Queue> {
        let queue = self.forge.queue(None).await?.queue.on_host(&self.host);
        let (cache, saved) = (self.cache.clone(), queue.clone());
        crate::blocking::run(move || cache.write_entry(&crate::cache::keys::queue(None), &saved)).await.or_warn("caching the queue");
        Ok(queue)
    }

    pub(crate) fn cached(&self) -> Option<crate::forge::Queue> {
        self.cache.read_entry(&crate::cache::keys::queue(None)).map(|e: crate::cache::Entry<crate::forge::Queue>| e.value)
    }
}

/// Each MR read on its own from the host it lives on, all at once, answers in the order of `keys`.
pub(crate) async fn fetch_each(main: &Forge, others: &[Home], keys: &[MrKey]) -> Vec<Result<Mr>> {
    let forge_of =
        |key: &MrKey| key.host.as_deref().and_then(|h| others.iter().find(|home| home.host == h)).map_or(main, |home| &home.forge);
    futures_util::future::join_all(keys.iter().map(|key| forge_of(key).mr(key))).await
}

impl Ctx {
    /// The hosts a merged queue names: this one and every other with a login.
    pub(crate) fn hosts(&self, others: &[Home]) -> crate::forge::Hosts {
        crate::forge::Hosts {
            main: (self.credentials.host.clone(), self.forge.kind()),
            others: others.iter().map(|h| (h.host.clone(), h.forge.kind())).collect(),
        }
    }

    /// The other hosts the config remembers a login for, each with a token found for it.
    pub(crate) fn others(&self) -> Vec<Home> {
        let store = SecurityCli::new(auth::SERVICE);
        others(&store, &self.config, &self.credentials.host)
    }
}

pub(crate) fn others(store: &dyn SecretStore, config: &Config, current: &str) -> Vec<Home> {
    let mut hosts: Vec<&String> = config.host.iter().chain(config.hosts.keys()).filter(|h| !h.eq_ignore_ascii_case(current)).collect();
    hosts.sort();
    hosts.dedup();
    hosts
        .into_iter()
        .filter_map(|host| {
            // A configured host without a token is one not logged in to yet: nothing went wrong.
            let credentials = auth::resolve_extra(store, host).ok()?;
            Home::connect(host, config, &credentials).inspect_err(|e| tracing::warn!("leaving out {host}: {e:#}")).ok()
        })
        .collect()
}

/// Prints `value` as pretty JSON, what every command does under `--json`.
pub(crate) fn emit<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::MemoryStore;
    use crate::config::Host;

    #[test]
    fn the_current_host_is_left_out_of_the_others_whatever_its_case() {
        let store = MemoryStore::default();
        store.set("GitLab.com", "glpat-xxxx").unwrap();
        store.set("gl.acme.dev", "glpat-xxxx").unwrap();
        let config =
            Config { host: Some("GitLab.com".into()), hosts: [("gl.acme.dev".into(), Host::default())].into(), ..Config::default() };
        let hosts: Vec<String> = others(&store, &config, "gitlab.com").into_iter().map(|home| home.host).collect();
        assert_eq!(hosts, ["gl.acme.dev"]);
    }
}
