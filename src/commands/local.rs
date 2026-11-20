use crate::cli::LocalArgs;
use crate::ctx::{Launch, LocalHere};
use anyhow::Result;

/// Finds the worktree and its target before the screen changes, so a bad one fails on the shell; `host` and `all`
/// are the forge flags `:forge` opens with.
pub async fn run(args: LocalArgs, host: Option<String>, all: bool) -> Result<()> {
    let launch = Launch { host, all, target: args.target };
    let LocalHere { ctx, here } = launch.local().await?;
    crate::tui::run(ctx, Some(here), launch).await
}
