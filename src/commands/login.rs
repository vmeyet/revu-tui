use crate::auth::{self, Credentials, Env, SecretStore, SecurityCli};
use crate::cache::Cache;
use crate::cli::LoginArgs;
use crate::config::Config;
use crate::forge::{Forge, Kind};
use anyhow::{Context, Result, bail};
use std::io::{ErrorKind, IsTerminal, Read, Write};
use std::process::Command;

/// Verifies a token and stores it in the keychain.
pub async fn run(args: LoginArgs, host_flag: Option<&str>, json: bool) -> Result<()> {
    let mut config = Config::load()?;
    let host = auth::pick_host(&Env::from_process(), &config, args.host.as_deref().or(host_flag), |_| true);
    let kind = Kind::for_host(&host, &config);
    let token = read_token(&args, &host, kind)?;
    let credentials = Credentials { host: host.clone(), token };
    let forge = Forge::connect(kind, &credentials)?;
    let me = forge.me().await.with_context(|| format!("token rejected: it needs the `{}` scope", scope(kind)))?;
    SecurityCli::new(auth::SERVICE).set(&host, &credentials.token)?;
    remember(&mut config, &host, &me.username);
    config.save()?;
    if json {
        println!("{}", serde_json::json!({"host": host, "username": me.username}));
    } else {
        println!("logged in to {host} as {}", me.username);
        if args.from_glab || args.from_gh {
            println!("note: the forge's own CLI still keeps its copy of this token");
        }
    }
    Ok(())
}

/// The first host logged in becomes the default; every host remembers who I am there.
fn remember(config: &mut Config, host: &str, username: &str) {
    config.hosts.entry(host.to_owned()).or_default().username = Some(username.to_owned());
    if config.host.is_none() {
        config.host = Some(host.to_owned());
    }
    if config.host.as_deref() == Some(host) {
        config.username = Some(username.to_owned());
    }
}

/// Forgets a host: keychain entry, config and cache.
pub fn logout(host: Option<&str>) -> Result<()> {
    let mut config = Config::load()?;
    let host = auth::pick_host(&Env::default(), &config, host, |_| true);
    SecurityCli::new(auth::SERVICE).delete(&host)?;
    if let Some(entry) = config.hosts.get_mut(&host) {
        entry.username = None;
    }
    if config.host.as_deref() == Some(&host) {
        config.host = None;
        config.username = None;
    }
    config.save()?;
    Cache::for_host(&host).clear()?;
    println!("logged out of {host}");
    Ok(())
}

fn scope(kind: Kind) -> &'static str {
    match kind {
        Kind::GitLab => "api",
        Kind::GitHub => "repo",
    }
}

fn read_token(args: &LoginArgs, host: &str, kind: Kind) -> Result<String> {
    let raw = if args.from_glab {
        glab_token(host)?
    } else if args.from_gh {
        gh_token(host)?
    } else if args.token.as_deref() == Some("-") {
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text)?;
        text
    } else if let Some(other) = &args.token {
        bail!("--token only accepts `-` (read from stdin), got {other:?}; a token in argv shows up in `ps`")
    } else {
        prompt(host, kind)?
    };
    let token = raw.trim().to_owned();
    if token.is_empty() {
        bail!("empty token");
    }
    Ok(token)
}

/// Runs a forge CLI, saying how to go on without it when it is not installed.
fn forge_cli(program: &str, args: &[&str], host: &str) -> Result<std::process::Output> {
    match Command::new(program).args(args).output() {
        Err(err) if err.kind() == ErrorKind::NotFound => {
            bail!(
                "{program} is not installed: install it and log in with it, or run `mr login {host}` to paste a token (`--token -` reads stdin)"
            )
        }
        other => other.with_context(|| format!("running {program}")),
    }
}

fn glab_token(host: &str) -> Result<String> {
    let output = forge_cli("glab", &["auth", "status", "--show-token", "--hostname", host], host)?;
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    text.lines()
        .find_map(|line| line.split("Token found:").nth(1))
        .map(|token| token.trim().to_owned())
        .with_context(|| format!("glab holds no token for {host}: run `glab auth login --hostname {host}` first"))
}

fn gh_token(host: &str) -> Result<String> {
    let output = forge_cli("gh", &["auth", "token", "--hostname", host], host)?;
    if !output.status.success() {
        bail!("gh holds no token for {host}: run `gh auth login --hostname {host}` first");
    }
    Ok(String::from_utf8(output.stdout)?)
}

/// Where to make a token, and the flag that borrows the forge CLI's one instead.
fn token_page(host: &str, kind: Kind) -> (String, &'static str) {
    match kind {
        Kind::GitLab => (format!("https://{host}/-/user_settings/personal_access_tokens?scopes=api"), "--from-glab"),
        Kind::GitHub => (format!("https://{host}/settings/tokens"), "--from-gh"),
    }
}

fn prompt(host: &str, kind: Kind) -> Result<String> {
    let (page, borrow) = token_page(host, kind);
    if !std::io::stdin().is_terminal() {
        bail!("no terminal to prompt on: pipe the token with `--token -`, or use `{borrow}`");
    }
    eprintln!("Create a token with the `{}` scope at {page} (or run `mr login {borrow}`)", scope(kind));
    eprint!("Token (hidden): ");
    std::io::stderr().flush()?;
    let token = read_hidden()?;
    eprintln!();
    Ok(token)
}

/// Reads one line with echo off, restoring the terminal even when the read fails.
fn read_hidden() -> Result<String> {
    crossterm::terminal::enable_raw_mode()?;
    let line = read_raw_line();
    crossterm::terminal::disable_raw_mode()?;
    line
}

fn read_raw_line() -> Result<String> {
    use crossterm::event::{Event, KeyCode, KeyModifiers, read};
    let mut line = String::new();
    loop {
        if let Event::Key(key) = read()? {
            match key.code {
                KeyCode::Enter => return Ok(line),
                KeyCode::Backspace => {
                    line.pop();
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => bail!("cancelled"),
                KeyCode::Char(c) => line.push(c),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn each_forge_names_its_scope_page_and_cli() {
        assert_eq!(token_page("github.com", Kind::GitHub), ("https://github.com/settings/tokens".into(), "--from-gh"));
        assert_eq!(token_page("gitlab.com", Kind::GitLab).1, "--from-glab");
        assert_eq!((scope(Kind::GitHub), scope(Kind::GitLab)), ("repo", "api"));
    }

    #[test]
    fn a_missing_cli_says_how_to_go_on_without_it() {
        let err = forge_cli("mr-no-such-cli", &[], "github.com").unwrap_err().to_string();
        assert!(err.contains("not installed") && err.contains("mr login github.com") && err.contains("--token -"), "{err}");
    }

    #[test]
    fn the_first_login_sets_the_default_and_every_host_keeps_its_name() {
        let mut config = Config::default();
        remember(&mut config, "gitlab.com", "nina");
        remember(&mut config, "github.com", "nina-gh");
        assert_eq!(config.host.as_deref(), Some("gitlab.com"));
        assert_eq!(config.username_for("gitlab.com").as_deref(), Some("nina"));
        assert_eq!(config.username_for("github.com").as_deref(), Some("nina-gh"));
    }
}
