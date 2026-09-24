use super::target;
use crate::cli::ReadyArgs;
use crate::ctx::Ctx;
use anyhow::{Result, bail};

/// Marks my own MR ready for review, or a draft again with `--undo`.
pub async fn run(ctx: &Ctx, args: ReadyArgs) -> Result<()> {
    let key = target::resolve(&ctx.forge, args.mr.as_deref()).await?;
    let mr = ctx.forge.mr(&key).await?;
    let name = format!("{}{}", ctx.forge.kind().sigil(), key.number);
    if let Some(reason) = mr.draft_refusal() {
        bail!("cannot change {name}: {reason}");
    }
    let draft = args.undo;
    ctx.forge.set_draft(&key, draft).await?;
    if ctx.json {
        return crate::ctx::emit(&serde_json::json!({"project": key.project, "number": key.number, "draft": draft}));
    }
    println!("{name} is {}", if draft { "a draft" } else { "ready for review" });
    Ok(())
}
