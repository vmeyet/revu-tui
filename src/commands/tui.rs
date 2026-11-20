use super::target;
use crate::ctx::{Ctx, ForgeArgs};
use anyhow::Result;

/// Resolves the MR asked for before the screen changes, so a bad reference fails on the shell; `host` and `all` are
/// the forge flags `:forge` opens with after `:local`.
pub async fn run(ctx: Ctx, mr: Option<String>, host: Option<String>, all: bool) -> Result<()> {
    let start = match mr {
        Some(mr) => Some(target::resolve(&ctx.forge, Some(&mr)).await.map_err(|err| ctx.with_host_hint(err))?),
        None => None,
    };
    crate::tui::run(ctx, start, ForgeArgs { host, all }).await
}
