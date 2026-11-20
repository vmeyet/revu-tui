use super::target;
use crate::ctx::Launch;
use anyhow::Result;

/// Opens the forge `host` and `all` name, and resolves the MR asked for before the screen changes, so a bad reference
/// fails on the shell.
pub async fn run(host: Option<String>, all: bool, mr: Option<String>) -> Result<()> {
    let launch = Launch { host, all, target: None };
    let ctx = launch.forge()?;
    let start = match mr {
        Some(mr) => Some(target::resolve(&ctx.forge, Some(&mr)).await.map_err(|err| ctx.with_host_hint(err))?),
        None => None,
    };
    crate::tui::run(ctx, start, launch).await
}
