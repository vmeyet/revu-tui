use super::target;
use crate::cache::keys;
use crate::cli::RefArgs;
use crate::ctx::Ctx;
use crate::diff::{self, LineKind};
use crate::forge::{DiffFile, Discussion, Kind, Mr, MrKey, Note, Position};
use crate::render::{self, Cell, Style, Theme, cell, right};
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;

const BODY_W: usize = 80;

/// Prints the MR header, its files and its unresolved threads.
pub async fn run(ctx: &Ctx, args: RefArgs) -> Result<()> {
    let key = target::resolve(&ctx.forge, args.mr.as_deref()).await?;
    let (mr, diffs, discussions) = fetch(ctx, &key).await?;
    if ctx.json {
        return crate::ctx::emit(&Shown { files: diffs.iter().map(FileStat::from).collect(), mr: &mr, discussions: &discussions });
    }
    print!("{}", text(ctx.forge.kind(), &mr, &diffs, &discussions, Theme::detect(), Utc::now()));
    Ok(())
}

/// The three answers together, written to the cache so the TUI opens the MR without waiting.
pub(crate) async fn fetch(ctx: &Ctx, key: &MrKey) -> Result<(Mr, Vec<DiffFile>, Vec<Discussion>)> {
    let forge = &ctx.forge;
    let (mr, diffs, discussions) = tokio::try_join!(forge.mr(key), forge.diffs(key), forge.discussions(key))?;
    ctx.cache.write_entry(&keys::mr(key), &mr)?;
    ctx.cache.write_entry(&keys::diffs(key, &mr.refs.head), &diffs)?;
    ctx.cache.write_entry(&keys::discussions(key), &discussions)?;
    Ok((mr, diffs, discussions))
}

#[derive(Serialize)]
struct Shown<'a> {
    mr: &'a Mr,
    files: Vec<FileStat>,
    discussions: &'a [Discussion],
}

#[derive(Serialize)]
pub(crate) struct FileStat {
    pub old_path: String,
    pub new_path: String,
    pub additions: usize,
    pub deletions: usize,
}

impl From<&DiffFile> for FileStat {
    fn from(file: &DiffFile) -> Self {
        let lines = diff::parse(&file.diff).into_iter().flat_map(|h| h.lines);
        let (additions, deletions) = lines.fold((0, 0), |(a, d), line| match line.kind {
            LineKind::Added => (a + 1, d),
            LineKind::Removed => (a, d + 1),
            LineKind::Context => (a, d),
        });
        Self { old_path: file.old_path.clone(), new_path: file.new_path.clone(), additions, deletions }
    }
}

pub(crate) fn text(kind: Kind, mr: &Mr, diffs: &[DiffFile], discussions: &[Discussion], theme: Theme, now: DateTime<Utc>) -> String {
    let open: Vec<&Discussion> = discussions.iter().filter(|d| unresolved(d)).collect();
    let files: Vec<Vec<Cell>> = diffs.iter().map(|f| file_row(f, &FileStat::from(f))).collect();
    let threads: Vec<Vec<Cell>> = open.iter().filter_map(|d| thread_row(d, now)).collect();
    let header = format!(
        "{} {}\n{}\n{}\n",
        theme.paint(&format!("{}{}", kind.sigil(), mr.number), Style::Accent),
        theme.paint(&mr.title, Style::Bold),
        theme.paint(&meta(mr, discussions.len(), open.len(), now), Style::Plain),
        theme.paint(&mr.web_url, Style::Dim)
    );
    let files_block =
        format!("\n{} {}\n{}", theme.paint("FILES", Style::Bold), theme.paint(&diffs.len().to_string(), Style::Dim), theme.table(&files));
    let threads_block = if threads.is_empty() {
        String::new()
    } else {
        format!(
            "\n{} {}\n{}",
            theme.paint("UNRESOLVED", Style::Bold),
            theme.paint(&threads.len().to_string(), Style::Dim),
            theme.table(&threads)
        )
    };
    header + &files_block + &threads_block
}

fn meta(mr: &Mr, threads: usize, open: usize, now: DateTime<Utc>) -> String {
    let pipeline = mr.pipeline.as_ref().map(|p| format!("pipeline {}", p.status.to_ascii_lowercase()));
    let approvals = match mr.approvals.approved_by.len() {
        0 if mr.approvals.approved => Some("approved".to_owned()),
        0 => None,
        _ => Some(format!("approved by {}", mr.approvals.approved_by.iter().map(|u| u.username.as_str()).collect::<Vec<_>>().join(", "))),
    };
    let threads = (threads > 0).then(|| format!("{threads} threads, {open} unresolved"));
    let conflicts = mr.conflicts.then(|| "conflicts".to_owned());
    [
        Some(mr.author.username.clone()),
        Some(format!("{} → {}", mr.source_branch, mr.target_branch)),
        Some(render::age(mr.updated_at, now)),
        pipeline,
        approvals,
        threads,
        conflicts,
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}

fn file_row(file: &DiffFile, stat: &FileStat) -> Vec<Cell> {
    let state = match file {
        f if f.too_large => "too large",
        f if f.new_file => "new",
        f if f.deleted_file => "deleted",
        f if f.renamed_file => "renamed",
        f if f.diff.is_empty() => "binary",
        _ => "",
    };
    let path = if file.renamed_file { format!("{} → {}", file.old_path, file.new_path) } else { file.new_path.clone() };
    vec![
        cell(format!("  {path}"), Style::Plain),
        right(format!("+{}", stat.additions), Style::Ok),
        right(format!("−{}", stat.deletions), Style::Bad),
        cell(state, Style::Dim),
    ]
}

fn unresolved(discussion: &Discussion) -> bool {
    discussion.notes.first().is_some_and(|n| n.resolvable && !n.resolved)
}

/// `path:line` of a note on the diff, the path alone when it names no line.
fn anchor_label(position: &Position) -> String {
    match position.line.number() {
        Some(line) => format!("{}:{line}", position.path()),
        None => position.path().to_owned(),
    }
}

fn thread_row(discussion: &Discussion, now: DateTime<Utc>) -> Option<Vec<Cell>> {
    let note: &Note = discussion.notes.iter().find(|n| !n.system)?;
    let anchor = note.position.as_ref().map_or_else(|| "(mr)".to_owned(), anchor_label);
    Some(vec![
        cell(format!("  {anchor}"), Style::Accent),
        cell(&note.author.username, Style::Plain),
        right(render::age(note.updated_at, now), Style::Dim),
        cell(render::truncate(&note.body, BODY_W), Style::Plain),
    ])
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::forge::gitlab::fixture;
    use chrono::TimeZone;

    fn mr() -> Mr {
        fixture::mr(
            r#"{"id": 1042, "iid": 42, "project_id": 7, "title": "feat: charge cards at checkout", "state": "opened", "draft": false,
                "author": {"id": 5, "username": "omar", "name": "Omar"}, "source_branch": "feat/checkout", "target_branch": "main",
                "web_url": "https://gitlab.com/acme/widgets/-/merge_requests/42", "updated_at": "2026-09-22T10:00:00Z", "sha": "bbbb",
                "diff_refs": {"base_sha": "aaaa", "head_sha": "bbbb", "start_sha": "aaaa"},
                "head_pipeline": {"status": "success"}, "has_conflicts": true,
                "approvals": {"approved": true, "approved_by": [{"user": {"id": 3, "username": "lea", "name": "Léa"}}]}}"#,
        )
    }

    fn diffs() -> Vec<DiffFile> {
        vec![
            DiffFile {
                diff: "@@ -1,2 +1,3 @@\n a\n-b\n+c\n+d\n".into(),
                old_path: "src/pay/charge.rs".into(),
                new_path: "src/pay/charge.rs".into(),
                ..DiffFile::default()
            },
            DiffFile { old_path: "old.rs".into(), new_path: "new.rs".into(), renamed_file: true, ..DiffFile::default() },
        ]
    }

    #[test]
    fn header_files_and_open_threads_are_laid_out() {
        let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();
        let discussions = vec![
            fixture::discussion(include_str!("../forge/gitlab/fixtures/diff_note.json")),
            fixture::discussion(include_str!("../forge/gitlab/fixtures/discussions.json")),
        ];
        let out = text(Kind::GitLab, &mr(), &diffs(), &discussions, Theme::plain(), now);
        assert!(
            out.starts_with("!42 feat: charge cards at checkout\nomar · feat/checkout → main · 2h · pipeline success · approved by lea"),
            "{out}"
        );
        assert!(out.contains("1 unresolved · conflicts\n"), "{out}");
        assert!(out.contains("FILES 2\n  src/pay/charge.rs  +2  −1\n  old.rs → new.rs    +0  −0  renamed\n"), "{out}");
        assert!(out.contains("UNRESOLVED 1\n  src/pay/charge.rs:57  "), "{out}");
    }

    #[test]
    fn file_stats_count_added_and_removed_lines() {
        let stat = FileStat::from(&diffs()[0]);
        assert_eq!((stat.additions, stat.deletions), (2, 1));
    }
}
