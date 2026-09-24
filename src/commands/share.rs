use super::target;
use crate::cli::ShareArgs;
use crate::ctx::Ctx;
use crate::share::{self, Fields, Target};
use anyhow::{Result, bail};
use std::io::{BufRead, IsTerminal, Write};

/// Prints the message, asks unless `--yes`, then pipes it to the target's command.
pub async fn run(ctx: &Ctx, args: ShareArgs) -> Result<()> {
    let target = pick(&share::targets(&ctx.config.share), args.target.as_deref())?;
    let key = target::resolve(&ctx.forge, args.mr.as_deref()).await?;
    let mr = ctx.forge.mr(&key).await?;
    let fields = Fields::from_mr(&mr, ctx.forge.kind());
    let message = share::render(&target.template, &fields, args.note.as_deref().unwrap_or(""));
    println!("{message}");
    if args.dry_run {
        return Ok(());
    }
    if !args.yes && !confirmed(&target)? {
        println!("not shared");
        return Ok(());
    }
    share::send(&target, &message).await?;
    println!("{}", target.done(&fields.reference));
    Ok(())
}

/// The named target, or the only one; several without `--target` is a question only the user answers.
fn pick(targets: &[Target], wanted: Option<&str>) -> Result<Target> {
    let names = || targets.iter().filter_map(|t| t.name.clone()).collect::<Vec<_>>().join(", ");
    match (wanted, targets) {
        (_, []) => bail!("nothing to share to: add `[share] command` to the config (see docs/guides/share.md)"),
        (Some(name), _) => targets
            .iter()
            .find(|t| t.name.as_deref() == Some(name))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no share target named {name}; the targets are {}", names())),
        (None, [only]) => Ok(only.clone()),
        (None, _) => bail!("several share targets: pick one with --target ({})", names()),
    }
}

fn confirmed(target: &Target) -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        bail!("no terminal to ask on: pass --yes to send, or --dry-run to only print");
    }
    eprint!("send it to `{}`? [y/N] ", target.command);
    std::io::stderr().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn target(name: Option<&str>) -> Target {
        Target { name: name.map(str::to_owned), command: "cat".into(), template: share::DEFAULT_TEMPLATE.into() }
    }

    #[test]
    fn the_target_is_the_only_one_or_the_named_one() {
        assert_eq!(pick(&[target(None)], None).unwrap(), target(None));
        let both = [target(None), target(Some("team"))];
        assert_eq!(pick(&both, Some("team")).unwrap(), target(Some("team")));
        let err = pick(&both, None).unwrap_err().to_string();
        assert!(err.contains("--target") && err.contains("team"), "{err}");
        assert!(pick(&both, Some("ops")).unwrap_err().to_string().contains("no share target named ops"));
        assert!(pick(&[], None).unwrap_err().to_string().contains("[share] command"));
    }
}
