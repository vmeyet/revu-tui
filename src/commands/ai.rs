//! `revu ai login|logout|status`: the AI keys live in the keychain, never in the config.
use crate::ai::{self, KeyEnv, KeySource, Provider, Secret, Status};
use crate::auth::{self, SecretStore, SecurityCli};
use crate::cli::{AiArgs, AiCommand, AiLoginArgs};
use crate::config::Config;
use crate::render::{Style, Theme};
use anyhow::{Context, Result, bail};
use std::io::{IsTerminal, Read, Write};
use std::process::Command;

const ANTHROPIC_API: &str = "https://api.anthropic.com";
/// Where slack-tui keeps its TypeSafe key, which `revu ai login typesafe` offers to reuse.
const SLACK_TUI_SERVICE: &str = "typesafe";

/// Runs one `revu ai` subcommand.
pub async fn run(args: AiArgs, json: bool) -> Result<()> {
    let store = SecurityCli::new(auth::SERVICE);
    match args.command {
        AiCommand::Login(login) => self::login(&login, &store).await,
        AiCommand::Logout { provider } => logout(provider, &store),
        AiCommand::Status => status(&store, json),
    }
}

async fn login(args: &AiLoginArgs, store: &dyn SecretStore) -> Result<()> {
    let key = read_key(args)?;
    if args.provider == Provider::Anthropic {
        verify_anthropic(ANTHROPIC_API, &key).await?;
    }
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
    let model = status.model.as_deref().map(|m| format!(" · {m}")).unwrap_or_default();
    format!("{:<10} {state} · {key}{model}", status.provider.to_string())
}

/// The key from stdin, slack-tui's keychain entry, or a hidden prompt, in that order of asking.
fn read_key(args: &AiLoginArgs) -> Result<Secret> {
    if args.from_slack_tui && args.provider != Provider::Typesafe {
        bail!("--from-slack-tui only holds a TypeSafe key");
    }
    let raw = match args.token.as_deref() {
        Some("-") => {
            let mut text = String::new();
            std::io::stdin().read_to_string(&mut text)?;
            text
        }
        Some(other) => bail!("--token only accepts `-` (read from stdin), got {other:?}; a key in argv shows up in `ps`"),
        None if args.from_slack_tui => slack_tui_key()?.context("slack-tui holds no TypeSafe key (keychain service `typesafe`)")?,
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
    if provider == Provider::Typesafe && slack_tui_key()?.is_some() && confirm("slack-tui already stores a TypeSafe key; use it?")? {
        return slack_tui_key()?.context("the slack-tui key vanished");
    }
    eprintln!("Paste your {provider} key (make one at {})", provider.key_page());
    eprint!("Key (hidden): ");
    std::io::stderr().flush()?;
    let key = super::login::read_hidden()?;
    eprintln!();
    Ok(key)
}

fn confirm(question: &str) -> Result<bool> {
    eprint!("{question} [Y/n] ");
    std::io::stderr().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(!answer.trim().to_lowercase().starts_with('n'))
}

/// slack-tui stores its key under service `typesafe` with no fixed account, so this reads by service alone.
fn slack_tui_key() -> Result<Option<String>> {
    let output = Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", SLACK_TUI_SERVICE, "-w"])
        .output()
        .context("running security")?;
    let key = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    Ok((output.status.success() && !key.is_empty()).then_some(key))
}

/// Lists the models, which costs nothing, so a wrong key is refused before it is stored.
async fn verify_anthropic(base: &str, key: &Secret) -> Result<()> {
    let response = reqwest::Client::new()
        .get(format!("{base}/v1/models"))
        .header("x-api-key", key.expose())
        .header("anthropic-version", "2023-06-01")
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("could not reach Anthropic: {}", e.without_url()))?;
    match response.status().as_u16() {
        200..=299 => Ok(()),
        401 | 403 => bail!("Anthropic refused this key (HTTP {})", response.status().as_u16()),
        status => bail!("Anthropic answered HTTP {status} while checking the key"),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn a_key_is_checked_against_the_models_list_before_it_is_stored() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("x-api-key", "sk-ant-good"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": []})))
            .mount(&server)
            .await;
        Mock::given(method("GET")).and(path("/v1/models")).respond_with(ResponseTemplate::new(401)).mount(&server).await;
        verify_anthropic(&server.uri(), &Secret::new("sk-ant-good")).await.unwrap();
        let err = verify_anthropic(&server.uri(), &Secret::new("sk-ant-bad")).await.unwrap_err().to_string();
        assert!(err.contains("refused") && !err.contains("sk-ant-bad"), "{err}");
    }

    #[test]
    fn status_says_where_the_key_is_never_what_it_is() {
        let line = |enabled, key| {
            describe(&Status { provider: Provider::Anthropic, enabled, key, model: Some("claude-opus-5".into()) }, Theme { color: false })
        };
        assert_eq!(line(true, Some(KeySource::Env)), "anthropic  on  · key from ANTHROPIC_API_KEY · claude-opus-5");
        assert_eq!(line(false, None), "anthropic  off · no key: `revu ai login anthropic` · claude-opus-5");
        assert_eq!(line(true, None), "anthropic  on, no key · no key: `revu ai login anthropic` · claude-opus-5");
    }
}
