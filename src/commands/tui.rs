use super::target;
use crate::cli::TuiArgs;
use crate::ctx::Ctx;
use anyhow::Result;

/// Resolves the MR asked for before the screen changes, so a bad reference fails on the shell.
pub async fn run(ctx: Ctx, args: TuiArgs) -> Result<()> {
    let start = match args.mr {
        Some(mr) => Some(target::resolve(&ctx.forge, Some(&mr)).await?),
        None => None,
    };
    crate::tui::run(ctx, start).await
}
