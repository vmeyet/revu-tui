use super::target;
use crate::api::{Client, DiffFile, Position};
use crate::cli::CommentArgs;
use crate::ctx::Ctx;
use crate::diff;
use anyhow::{Context, Result, bail};

/// Posts a public comment, on a line when `--at` names one.
pub async fn run(ctx: &Ctx, args: CommentArgs) -> Result<()> {
    let (project_id, iid) = target::resolve(&ctx.gitlab, Some(&args.mr)).await?;
    let body = args.text.join(" ");
    let position = match args.at.as_deref() {
        Some(at) => Some(anchor(&ctx.gitlab, project_id, iid, at).await?),
        None => None,
    };
    let discussion = ctx.gitlab.comment(project_id, iid, &body, position.as_ref()).await?;
    if ctx.json {
        return crate::ctx::emit(&discussion);
    }
    println!("posted discussion {} on !{iid}", discussion.id);
    Ok(())
}

/// `path:line` on the new side of the current diff.
async fn anchor(gitlab: &Client, project_id: u64, iid: u64, at: &str) -> Result<Position> {
    let (path, line) = parse_at(at)?;
    let (mr, diffs) = tokio::try_join!(gitlab.mr(project_id, iid), gitlab.diffs(project_id, iid))?;
    let file = diffs.iter().find(|f| f.new_path == path).with_context(|| format!("{path} is not in the diff of !{iid}"))?;
    let old_line = line_at(file, line).with_context(|| format!("line {line} of {path} is not in the diff of !{iid}"))?.old;
    Ok(Position::line(&mr.diff_refs, &file.old_path, &file.new_path, old_line, Some(line)))
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
