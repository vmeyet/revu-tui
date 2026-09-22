use crate::auth::Source;
use crate::ctx::Ctx;
use anyhow::Result;
use serde::Serialize;

#[derive(Serialize)]
struct Whoami<'a> {
    host: &'a str,
    username: &'a str,
    name: &'a str,
    id: u64,
    token_source: &'static str,
}

/// Prints the signed-in user and where the token came from.
pub async fn run(ctx: &Ctx) -> Result<()> {
    let me = ctx.forge.me().await?;
    let token_source = match ctx.source {
        Source::Env => "GITLAB_TOKEN",
        Source::Keychain => "keychain",
    };
    if ctx.json {
        return crate::ctx::emit(&Whoami { host: ctx.forge.host(), username: &me.username, name: &me.name, id: me.id, token_source });
    }
    println!("{} ({}) on {} · token from {token_source}", me.username, me.name, ctx.forge.host());
    Ok(())
}
