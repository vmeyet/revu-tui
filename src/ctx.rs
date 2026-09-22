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

/// Prints `value` as pretty JSON, what every command does under `--json`.
pub(crate) fn emit<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
