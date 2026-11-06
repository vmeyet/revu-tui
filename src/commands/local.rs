use crate::cli::LocalArgs;
use crate::ctx::Ctx;
use crate::forge::local::Client;
use anyhow::{Context, Result};

/// Finds the worktree and its target before the screen changes, so a bad one fails on the shell.
pub async fn run(args: LocalArgs) -> Result<()> {
    let dir = std::env::current_dir().context("reading the current folder")?;
    let client = Client::open(&dir, args.target.as_deref()).await?;
    let start = client.here_key().await?;
    crate::tui::run(Ctx::local(client)?, Some(start)).await
}
