use crate::cli::LocalArgs;
use crate::ctx::{Ctx, ForgeArgs, LocalHere};
use anyhow::Result;

/// Finds the worktree and its target before the screen changes, so a bad one fails on the shell; `host` and `all`
/// are the forge flags `:forge` opens with.
pub async fn run(args: LocalArgs, host: Option<String>, all: bool) -> Result<()> {
    let LocalHere { ctx, here } = Ctx::local_here(args.target.as_deref()).await?;
    crate::tui::run(ctx, Some(here), ForgeArgs { host, all }).await
}
