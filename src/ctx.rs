//! What every command opens first: the forge, the config, the cache and the checkout's project.
use crate::auth::{self, Credentials, Env, SecretStore, Source};
use crate::cache::Cache;
use crate::config::Config;
use crate::forge::{Forge, Kind};
use anyhow::Result;
use serde::Serialize;

/// Everything a command needs, opened once from the config, the environment and the keychain.
pub struct Ctx {
    pub(crate) forge: Forge,
    pub(crate) credentials: Credentials,
    pub(crate) source: Source,
    pub(crate) config: Config,
    pub(crate) cache: Cache,
    pub(crate) json: bool,
    /// The project of the checkout the command runs in: the queue shows only it unless `--all`.
    pub(crate) project: Option<String>,
}

impl Ctx {
    /// Reads the config, finds the token and the checkout's project.
    pub fn open(host: Option<&str>, json: bool) -> Result<Self> {
        let config = Config::load()?;
        let store = crate::legacy::Keychain::open();
        let ctx = Self::build(&Env::from_process(), &store, config, host, json)?;
        let project = std::env::current_dir().ok().and_then(|dir| crate::mrref::checkout_project(&dir, &ctx.credentials.host));
        Ok(Self { project, ..ctx })
    }

    /// `--all`: every project, wherever the command runs.
    pub fn everywhere(self, all: bool) -> Self {
        if all { Self { project: None, ..self } } else { self }
    }

    pub(crate) fn build(env: &Env, store: &dyn SecretStore, config: Config, host: Option<&str>, json: bool) -> Result<Self> {
        let (credentials, source) = auth::resolve(env, store, &config, host)?;
        let cache = Cache::for_host(&credentials.host);
        let forge = Forge::connect(Kind::for_host(&credentials.host, &config), &credentials)?;
        Ok(Self { forge, credentials, source, config, cache, json, project: None })
    }
}

/// Another host I am logged in to, ready for the queue to ask.
#[derive(Clone)]
pub(crate) struct Home {
    pub host: String,
    pub forge: Forge,
    pub cache: Cache,
}

impl Home {
    /// This host's queue across every project, each row tagged with the host.
    pub async fn queue(&self) -> Result<crate::forge::Queue> {
        let queue = self.forge.queue(None).await?.on_host(&self.host);
        let (cache, saved) = (self.cache.clone(), queue.clone());
        let _ = tokio::task::spawn_blocking(move || cache.write_entry(&crate::cache::keys::queue(None), &saved)).await;
        Ok(queue)
    }

    pub fn cached(&self) -> Option<crate::forge::Queue> {
        self.cache.read_entry(&crate::cache::keys::queue(None)).map(|e: crate::cache::Entry<crate::forge::Queue>| e.value)
    }
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
        let store = crate::legacy::Keychain::open();
        others(&Env::from_process(), &store, &self.config, &self.credentials.host)
    }
}

pub(crate) fn others(env: &Env, store: &dyn SecretStore, config: &Config, current: &str) -> Vec<Home> {
    let mut hosts: Vec<&String> = config.host.iter().chain(config.hosts.keys()).filter(|h| h.as_str() != current).collect();
    hosts.sort();
    hosts.dedup();
    hosts
        .into_iter()
        .filter_map(|host| {
            let (credentials, _) = auth::resolve(env, store, config, Some(host)).ok()?;
            let forge = Forge::connect(Kind::for_host(host, config), &credentials).ok()?;
            Some(Home { host: host.clone(), forge, cache: Cache::for_host(host) })
        })
        .collect()
}

/// Prints `value` as pretty JSON, what every command does under `--json`.
pub(crate) fn emit<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
