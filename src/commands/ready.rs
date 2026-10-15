use super::target;
use crate::cli::ReadyArgs;
use crate::ctx::Ctx;
use crate::forge::Readiness;
use anyhow::{Result, bail};

/// Marks my own MR ready for review, or a draft again with `--undo`.
pub async fn run(ctx: &Ctx, args: ReadyArgs) -> Result<()> {
    let key = target::resolve(&ctx.forge, args.mr.as_deref()).await?;
    let mr = ctx.forge.mr(&key).await?;
    let name = format!("{}{}", ctx.forge.kind().sigil(), key.number);
    if let Some(reason) = mr.draft_refusal() {
        bail!("cannot change {name}: {reason}");
    }
    let readiness = if args.undo { Readiness::Draft } else { Readiness::Ready };
    ctx.forge.set_readiness(&key, readiness).await?;
    if ctx.json {
        return crate::ctx::emit(&serde_json::json!({"project": key.project, "number": key.number, "draft": args.undo}));
    }
    println!("{name} is {}", readiness.described());
    Ok(())
}
