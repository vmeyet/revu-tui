use super::target;
use crate::cli::ApproveArgs;
use crate::ctx::Ctx;
use crate::forge::ApprovalChange;
use anyhow::Result;

/// Approves the MR, or takes the approval back with `--undo`.
pub async fn run(ctx: &Ctx, args: ApproveArgs) -> Result<()> {
    let key = target::resolve(&ctx.forge, args.mr.as_deref()).await?;
    let (change, verb) = if args.undo { (ApprovalChange::Unapprove, "unapproved") } else { (ApprovalChange::Approve, "approved") };
    ctx.forge.approve(&key, change).await?;
    if ctx.json {
        return crate::ctx::emit(&serde_json::json!({"project": key.project, "number": key.number, "approved": !args.undo}));
    }
    println!("{verb} {}{}", ctx.forge.kind().sigil(), key.number);
    Ok(())
}
