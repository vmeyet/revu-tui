use super::prompt::confirmed;
use super::target;
use crate::cli::MergeArgs;
use crate::ctx::Ctx;
use anyhow::{Result, bail};

/// Merges my own approved MR, the way its project merges, after a yes on a terminal.
pub async fn run(ctx: &Ctx, args: MergeArgs) -> Result<()> {
    let key = target::resolve(&ctx.forge, args.mr.as_deref()).await?;
    let mr = ctx.forge.mr(&key).await?;
    let name = format!("{}{}", ctx.forge.kind().sigil(), key.number);
    if let Some(reason) = mr.merge_refusal() {
        bail!("cannot merge {name}: {reason}");
    }
    let branch = if mr.merge.remove_branch { ", delete the branch" } else { "" };
    let question = format!("merge {name} into {} ({}{branch})?", mr.target_branch, mr.merge.method.word());
    if !args.yes && !confirmed(&question, &format!("{question} Pass --yes to merge without a terminal"))? {
        println!("not merged");
        return Ok(());
    }
    ctx.forge.merge(&key, &mr.refs.head, mr.merge).await?;
    if ctx.json {
        return crate::ctx::emit(&serde_json::json!({"project": key.project, "number": key.number, "merged": true}));
    }
    println!("merged {name} into {}", mr.target_branch);
    Ok(())
}
