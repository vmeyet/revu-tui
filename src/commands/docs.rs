use crate::cli::DocsArgs;
use anyhow::{Context, Result, bail};

/// Writes the reference pages, or with `--check` fails when one on disk is out of date.
pub fn run(args: &DocsArgs) -> Result<()> {
    let pages = crate::docs::pages()?;
    if args.check {
        return check(args, &pages);
    }
    std::fs::create_dir_all(&args.dir).with_context(|| format!("creating {}", args.dir.display()))?;
    for (name, text) in &pages {
        let path = args.dir.join(name);
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

fn check(args: &DocsArgs, pages: &[(&str, String)]) -> Result<()> {
    let stale: Vec<String> = pages
        .iter()
        .filter(|(name, text)| std::fs::read_to_string(args.dir.join(name)).ok().as_deref() != Some(text.as_str()))
        .map(|(name, _)| args.dir.join(name).display().to_string())
        .collect();
    if !stale.is_empty() {
        bail!("out of date, run `revu docs`: {}", stale.join(", "));
    }
    Ok(())
}
