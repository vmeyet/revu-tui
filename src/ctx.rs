use crate::api::Client;
use crate::auth::{self, Credentials, Env, SecretStore, SecurityCli, Source};
use crate::cache::Cache;
use crate::config::Config;
use anyhow::Result;
use serde::Serialize;

/// Everything a command needs, opened once from the config, the environment and the keychain.
pub struct Ctx {
    pub gitlab: Client,
    pub credentials: Credentials,
    pub source: Source,
    pub config: Config,
    pub cache: Cache,
    pub json: bool,
}

impl Ctx {
    pub fn open(host: Option<&str>, json: bool) -> Result<Self> {
        let config = Config::load()?;
        let store = SecurityCli::new(auth::SERVICE);
        Self::build(&Env::from_process(), &store, config, host, json)
    }

    pub fn build(env: &Env, store: &dyn SecretStore, config: Config, host: Option<&str>, json: bool) -> Result<Self> {
        let (credentials, source) = auth::resolve(env, store, &config, host)?;
        let cache = Cache::for_host(&credentials.host);
        Ok(Self { gitlab: Client::new(&credentials)?, credentials, source, config, cache, json })
    }

    pub fn emit<T: Serialize>(&self, value: &T) -> Result<()> {
        println!("{}", serde_json::to_string_pretty(value)?);
        Ok(())
    }
}
