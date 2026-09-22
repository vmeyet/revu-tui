use super::target;
use crate::cli::RefArgs;
use crate::ctx::Ctx;
use anyhow::Result;

/// Publishes every held draft as one review.
pub async fn run(ctx: &Ctx, args: RefArgs) -> Result<()> {
    let key = target::resolve(&ctx.forge, args.mr.as_deref()).await?;
    let name = format!("{}{}", ctx.forge.kind().sigil(), key.number);
    let drafts = ctx.forge.drafts(&key).await?;
    if drafts.is_empty() {
        println!("no drafts on {name}");
        return Ok(());
    }
    ctx.forge.publish(&key, false).await?;
    if ctx.json {
        return crate::ctx::emit(&serde_json::json!({"project": key.project, "number": key.number, "published": drafts.len()}));
    }
    println!("published {} draft{} on {name}", drafts.len(), if drafts.len() == 1 { "" } else { "s" });
    Ok(())
}
