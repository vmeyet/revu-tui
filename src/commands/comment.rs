use super::target;
use crate::cli::CommentArgs;
use crate::ctx::Ctx;
use crate::diff;
use crate::forge::{DiffFile, Forge, LineRef, MrKey, Position};
use anyhow::{Context, Result, bail};

/// Posts a public comment, on a line when `--at` names one.
pub async fn run(ctx: &Ctx, args: CommentArgs) -> Result<()> {
    let key = target::resolve(&ctx.forge, Some(&args.mr)).await?;
    let body = args.text.join(" ");
    let position = match args.at.as_deref() {
        Some(at) => Some(anchor(&ctx.forge, &key, at).await?),
        None => None,
    };
    let discussion = ctx.forge.comment(&key, &body, position.as_ref()).await?;
    if ctx.json {
        return crate::ctx::emit(&discussion);
    }
    println!("posted discussion {} on {}{}", discussion.id, ctx.forge.kind().sigil(), key.number);
    Ok(())
}

/// `path:line` on the new side of the current diff.
async fn anchor(forge: &Forge, key: &MrKey, at: &str) -> Result<Position> {
    let (path, line) = parse_at(at)?;
    let (mr, diffs) = tokio::try_join!(forge.mr(key), forge.diffs(key))?;
    let name = format!("{}{}", forge.kind().sigil(), key.number);
    let file = diffs.iter().find(|f| f.new_path == path).with_context(|| format!("{path} is not in the diff of {name}"))?;
    let old = line_at(file, line).with_context(|| format!("line {line} of {path} is not in the diff of {name}"))?.old;
    Ok(Position {
        refs: mr.refs,
        old_path: file.old_path.clone(),
        new_path: file.new_path.clone(),
        line: LineRef { old, new: Some(line) },
        start: None,
    })
}

/// The diff line whose new-side number is `line`; `None` when the line is not part of the diff.
fn line_at(file: &DiffFile, line: u32) -> Option<diff::Line> {
    diff::parse(&file.diff).into_iter().flat_map(|h| h.lines).find(|l| l.new == Some(line))
}

fn parse_at(at: &str) -> Result<(String, u32)> {
    let Some((path, line)) = at.rsplit_once(':') else { bail!("--at wants path:line, got {at:?}") };
    let line: u32 = line.parse().with_context(|| format!("--at wants path:line, got {at:?}"))?;
    if path.is_empty() || line == 0 {
        bail!("--at wants path:line, got {at:?}");
    }
    Ok((path.to_owned(), line))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn at_splits_on_the_last_colon() {
        assert_eq!(parse_at("src/a.rs:13").unwrap(), ("src/a.rs".to_owned(), 13));
        assert_eq!(parse_at("c:/odd/path.rs:2").unwrap(), ("c:/odd/path.rs".to_owned(), 2));
        for bad in ["src/a.rs", "src/a.rs:x", ":3", "src/a.rs:0"] {
            assert!(parse_at(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn old_line_comes_from_the_diff() {
        let file = DiffFile { diff: "@@ -10,3 +10,4 @@\n a\n-b\n+c\n+d\n e\n".into(), ..DiffFile::default() };
        assert_eq!(line_at(&file, 10).unwrap().old, Some(10));
        assert_eq!(line_at(&file, 11).unwrap().old, None);
        assert_eq!(line_at(&file, 13).unwrap().old, Some(12));
        assert!(line_at(&file, 99).is_none());
    }
}
