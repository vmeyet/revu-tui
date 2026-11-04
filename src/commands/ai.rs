//! `revu ai login|logout|status`: the AI keys live in the keychain, never in the config.
use crate::ai::{self, KeyEnv, KeySource, Provider, Secret, Status};
use crate::auth::{self, SecretStore, SecurityCli};
use crate::cli::{AiArgs, AiCommand, AiLoginArgs};
use crate::config::Config;
use crate::render::{Style, Theme};
use anyhow::{Result, bail};
use std::io::{IsTerminal, Read};

/// Runs one `revu ai` subcommand.
pub fn run(args: AiArgs, json: bool) -> Result<()> {
    let store = SecurityCli::new(auth::SERVICE);
    match args.command {
        AiCommand::Login(login) => self::login(&login, &store),
        AiCommand::Logout { provider } => logout(provider, &store),
        AiCommand::Status => status(&store, json),
    }
}

fn login(args: &AiLoginArgs, store: &dyn SecretStore) -> Result<()> {
    let key = read_key(args)?;
    store.set(args.provider.account(), key.expose())?;
    let enabled = args.provider.enabled(&Config::load()?.ai);
    println!("stored the {} key in the keychain (service `revu`, account `{}`)", args.provider, args.provider.account());
    if !enabled {
        println!("it stays unused until the config says `[ai.{}] enabled = true`", args.provider);
    }
    Ok(())
}

fn logout(provider: Provider, store: &dyn SecretStore) -> Result<()> {
    store.delete(provider.account())?;
    println!("forgot the {provider} key");
    Ok(())
}

fn status(store: &dyn SecretStore, json: bool) -> Result<()> {
    let statuses = ai::status(&Config::load()?.ai, &KeyEnv::from_process(), store);
    if json {
        return crate::ctx::emit(&statuses);
    }
    let theme = Theme::detect();
    for line in statuses.iter().map(|s| describe(s, theme)) {
        println!("{line}");
    }
    Ok(())
}

fn describe(status: &Status, theme: Theme) -> String {
    let state = match (status.enabled, status.key) {
        (true, Some(_)) => theme.paint("on ", Style::Ok),
        (true, None) => theme.paint("on, no key", Style::Warn),
        (false, _) => theme.paint("off", Style::Dim),
    };
    let key = match status.key {
        Some(KeySource::Env) => format!("key from {}", status.provider.variable()),
        Some(KeySource::Keychain) => "key in the keychain".to_owned(),
        None => format!("no key: `revu ai login {}`", status.provider),
    };
    format!("{:<10} {state} · {key}", status.provider.to_string())
}

/// The key from stdin, or a hidden prompt.
fn read_key(args: &AiLoginArgs) -> Result<Secret> {
    let raw = match args.token.as_deref() {
        Some("-") => {
            let mut text = String::new();
            std::io::stdin().read_to_string(&mut text)?;
            text
        }
        Some(other) => bail!("--token only accepts `-` (read from stdin), got {other:?}; a key in argv shows up in `ps`"),
        None => prompt(args.provider)?,
    };
    let key = raw.trim();
    if key.is_empty() {
        bail!("empty key");
    }
    Ok(Secret::new(key))
}

fn prompt(provider: Provider) -> Result<String> {
    if !std::io::stdin().is_terminal() {
        bail!("no terminal to prompt on: pipe the key with `--token -`");
    }
    eprintln!("Paste your {provider} key (make one at {})", provider.key_page());
    super::prompt::read_hidden("Key")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_says_where_the_key_is_never_what_it_is() {
        let line = |enabled, key| describe(&Status { provider: Provider::Typesafe, enabled, key }, Theme { color: false });
        assert_eq!(line(true, Some(KeySource::Env)), "typesafe   on  · key from TYPESAFE_API_KEY");
        assert_eq!(line(false, None), "typesafe   off · no key: `revu ai login typesafe`");
        assert_eq!(line(true, None), "typesafe   on, no key · no key: `revu ai login typesafe`");
    }
}
