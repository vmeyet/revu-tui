use super::target;
use crate::cli::ApproveArgs;
use crate::ctx::Ctx;
use anyhow::Result;

/// Approves the MR, or takes the approval back with `--undo`.
pub async fn run(ctx: &Ctx, args: ApproveArgs) -> Result<()> {
    let (project_id, iid) = target::resolve(&ctx.gitlab, args.mr.as_deref()).await?;
    let verb = if args.undo { "unapproved" } else { "approved" };
    if args.undo {
        ctx.gitlab.unapprove(project_id, iid).await?;
    } else {
        ctx.gitlab.approve(project_id, iid).await?;
    }
    if ctx.json {
        return crate::ctx::emit(&serde_json::json!({"project_id": project_id, "iid": iid, "approved": !args.undo}));
    }
    println!("{verb} !{iid}");
    Ok(())
}
