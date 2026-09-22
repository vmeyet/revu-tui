use super::target;
use crate::api::DiffFile;
use crate::cli::RefArgs;
use crate::ctx::Ctx;
use crate::render::{Style, Theme};
use anyhow::{Context, Result};
use std::io::{IsTerminal, Write};
use std::process::{Command, Stdio};

/// Prints the coloured diff, through the pager on a terminal.
pub async fn run(ctx: &Ctx, args: RefArgs) -> Result<()> {
    let (project_id, iid) = target::resolve(&ctx.gitlab, args.mr.as_deref()).await?;
    let (_, diffs, _) = super::show::fetch(ctx, project_id, iid).await?;
    let out = text(&diffs, Theme::detect());
    if std::io::stdout().is_terminal() { page(&out) } else { print_all(&out) }
}

pub(crate) fn text(diffs: &[DiffFile], theme: Theme) -> String {
    diffs.iter().map(|file| file_text(file, theme)).collect()
}

fn file_text(file: &DiffFile, theme: Theme) -> String {
    let head = format!("diff --git a/{} b/{}\n--- a/{}\n+++ b/{}\n", file.old_path, file.new_path, file.old_path, file.new_path);
    let body: String = file
        .diff
        .lines()
        .map(|line| {
            let style = match line.as_bytes().first() {
                Some(b'@') => Style::Accent,
                Some(b'+') => Style::Ok,
                Some(b'-') => Style::Bad,
                _ => Style::Plain,
            };
            theme.paint(line, style) + "\n"
        })
        .collect();
    theme.paint(&head, Style::Bold) + &body
}

fn page(out: &str) -> Result<()> {
    let pager = std::env::var("PAGER").ok().filter(|p| !p.trim().is_empty()).unwrap_or_else(|| "less -R".to_owned());
    let mut parts = pager.split_whitespace();
    let program = parts.next().context("empty PAGER")?;
    let mut child = Command::new(program).args(parts).stdin(Stdio::piped()).spawn().with_context(|| format!("running {pager}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(out.as_bytes());
    }
    child.wait()?;
    Ok(())
}

fn print_all(out: &str) -> Result<()> {
    std::io::stdout().write_all(out.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn each_file_gets_a_git_header_and_coloured_signs() {
        let files = vec![DiffFile {
            diff: "@@ -1 +1 @@\n-a\n+b\n".into(),
            old_path: "x.rs".into(),
            new_path: "x.rs".into(),
            ..DiffFile::default()
        }];
        assert_eq!(text(&files, Theme::plain()), "diff --git a/x.rs b/x.rs\n--- a/x.rs\n+++ b/x.rs\n@@ -1 +1 @@\n-a\n+b\n");
        let coloured = text(&files, Theme { color: true });
        assert!(coloured.contains("\x1b[32m+b\x1b[39m") && coloured.contains("\x1b[31m-a\x1b[39m"), "{coloured:?}");
    }
}
