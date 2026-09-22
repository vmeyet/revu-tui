use super::target;
use crate::cli::RefArgs;
use crate::ctx::Ctx;
use anyhow::Result;

/// Every draft of mine on the MR goes public as one review.
/// Publishes every held draft as one review.
pub async fn run(ctx: &Ctx, args: RefArgs) -> Result<()> {
    let (project_id, iid) = target::resolve(&ctx.gitlab, args.mr.as_deref()).await?;
    let drafts = ctx.gitlab.draft_notes(project_id, iid).await?;
    if drafts.is_empty() {
        println!("no drafts on !{iid}");
        return Ok(());
    }
    ctx.gitlab.publish_drafts(project_id, iid).await?;
    if ctx.json {
        return crate::ctx::emit(&serde_json::json!({"project_id": project_id, "iid": iid, "published": drafts.len()}));
    }
    println!("published {} draft{} on !{iid}", drafts.len(), if drafts.len() == 1 { "" } else { "s" });
    Ok(())
}
