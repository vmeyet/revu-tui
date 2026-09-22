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

pub async fn run(ctx: &Ctx) -> Result<()> {
    let me = ctx.gitlab.me().await?;
    let token_source = match ctx.source {
        Source::Env => "GITLAB_TOKEN",
        Source::Keychain => "keychain",
    };
    if ctx.json {
        return ctx.emit(&Whoami { host: ctx.gitlab.host(), username: &me.username, name: &me.name, id: me.id, token_source });
    }
    println!("{} ({}) on {} · token from {token_source}", me.username, me.name, ctx.gitlab.host());
    Ok(())
}
