use crate::auth::{self, Credentials, Env, SecretStore, SecurityCli};
use crate::cache::Cache;
use crate::cli::LoginArgs;
use crate::config::Config;
use crate::forge::{Forge, Kind};
use anyhow::{Context, Result, bail};
use std::io::{IsTerminal, Read, Write};
use std::process::Command;

/// Verifies a token and stores it in the keychain.
pub async fn run(args: LoginArgs, host_flag: Option<&str>, json: bool) -> Result<()> {
    let mut config = Config::load()?;
    let host = auth::pick_host(&Env::from_process(), &config, args.host.as_deref().or(host_flag));
    let token = read_token(&args, &host)?;
    let credentials = Credentials { host: host.clone(), token };
    let forge = Forge::connect(Kind::for_host(&host, &config), &credentials)?;
    let me = forge.me().await.context("token rejected: it needs the `api` scope")?;
    SecurityCli::new(auth::SERVICE).set(&host, &credentials.token)?;
    config.host = Some(host.clone());
    config.username = Some(me.username.clone());
    config.save()?;
    if json {
        println!("{}", serde_json::json!({"host": host, "username": me.username}));
    } else {
        println!("logged in to {host} as {}", me.username);
        if args.from_glab {
            println!("note: glab still keeps its own copy of this token in its config file");
        }
    }
    Ok(())
}

/// Forgets a host: keychain entry, config and cache.
pub fn logout(host: Option<&str>) -> Result<()> {
    let mut config = Config::load()?;
    let host = auth::pick_host(&Env::default(), &config, host);
    SecurityCli::new(auth::SERVICE).delete(&host)?;
    if config.host.as_deref() == Some(&host) {
        config.host = None;
        config.username = None;
        config.save()?;
    }
    Cache::for_host(&host).clear()?;
    println!("logged out of {host}");
    Ok(())
}

fn read_token(args: &LoginArgs, host: &str) -> Result<String> {
    let raw = if args.from_glab {
        glab_token(host)?
    } else if args.token.as_deref() == Some("-") {
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text)?;
        text
    } else if let Some(other) = &args.token {
        bail!("--token only accepts `-` (read from stdin), got {other:?}; a token in argv shows up in `ps`")
    } else {
        prompt(host)?
    };
    let token = raw.trim().to_owned();
    if token.is_empty() {
        bail!("empty token");
    }
    Ok(token)
}

fn glab_token(host: &str) -> Result<String> {
    let output = Command::new("glab").args(["auth", "status", "--show-token", "--hostname", host]).output().context("running glab")?;
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    text.lines()
        .find_map(|line| line.split("Token found:").nth(1))
        .map(|token| token.trim().to_owned())
        .with_context(|| format!("glab holds no token for {host}: run `glab auth login --hostname {host}` first"))
}

fn prompt(host: &str) -> Result<String> {
    if !std::io::stdin().is_terminal() {
        bail!("no terminal to prompt on: use `--token -` and pipe the token");
    }
    eprintln!("Create a token with the `api` scope at https://{host}/-/user_settings/personal_access_tokens?scopes=api");
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
