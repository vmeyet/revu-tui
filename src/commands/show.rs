use super::target;
use crate::cache::keys;
use crate::cli::RefArgs;
use crate::ctx::Ctx;
use crate::diff::{self, LineKind};
use crate::forge::{DiffFile, Discussion, FileKind, Kind, Mr, MrKey, Note, Position};
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
    pub(crate) old_path: String,
    pub(crate) new_path: String,
    pub(crate) additions: usize,
    pub(crate) deletions: usize,
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
    let pipeline = mr.pipeline.as_ref().map(|p| format!("pipeline {}", p.status.word()));
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
    let state = match file.change {
        _ if file.too_large => "too large",
        FileKind::Added => "new",
        FileKind::Deleted => "deleted",
        FileKind::Renamed => "renamed",
        _ if file.diff.is_empty() => "binary",
        _ => "",
    };
    let path = if file.change == FileKind::Renamed { format!("{} → {}", file.old_path, file.new_path) } else { file.new_path.clone() };
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
    use super::*;
    use crate::auth::{Credentials, HostOrigin, Source};
    use crate::cache::Cache;
    use crate::config::Config;
    use crate::forge::Forge;
    use crate::forge::gitlab::fixture;
    use chrono::TimeZone;
    use wiremock::matchers::{any, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

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
            DiffFile { old_path: "old.rs".into(), new_path: "new.rs".into(), change: FileKind::Renamed, ..DiffFile::default() },
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

    fn ctx(forge: Forge, host_origin: HostOrigin) -> Ctx {
        Ctx {
            credentials: Credentials { host: forge.host().to_owned(), token: "ghp_xxxx".into() },
            forge,
            source: Source::Keychain,
            host_origin,
            config: Config::default(),
            cache: Cache::in_dir("/nonexistent"),
            json: false,
            project: None,
        }
    }

    /// github.com without the pull request: GraphQL answers 200 with a `NOT_FOUND` error, REST answers 404.
    async fn github_without_the_mr(host_origin: HostOrigin) -> (MockServer, Ctx) {
        let server = MockServer::start().await;
        let unresolved = serde_json::json!({
            "data": {"viewer": {"login": "nina"}, "repository": null},
            "errors": [{"type": "NOT_FOUND", "path": ["repository"], "message": "Could not resolve to a Repository with the name 'acme/widgets'."}],
        });
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_json(unresolved)).mount(&server).await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({"message": "Not Found"})))
            .mount(&server)
            .await;
        let credentials = Credentials { host: "github.com".into(), token: "ghp_xxxx".into() };
        let forge = Forge::GitHub(crate::forge::github::Client::with_base(&credentials, &server.uri()).unwrap());
        (server, ctx(forge, host_origin))
    }

    fn gitlab_on(server: &MockServer) -> Forge {
        let credentials = Credentials { host: "gitlab.com".into(), token: "glpat-xxxx".into() };
        Forge::GitLab(crate::forge::gitlab::Client::with_base(&credentials, &format!("{}/api/v4/", server.uri())).unwrap())
    }

    /// A GitLab host that answers every request with `status`.
    async fn gitlab_answering(status: u16, host_origin: HostOrigin) -> (MockServer, Ctx) {
        let server = MockServer::start().await;
        Mock::given(any()).respond_with(ResponseTemplate::new(status)).mount(&server).await;
        let forge = gitlab_on(&server);
        (server, ctx(forge, host_origin))
    }

    async fn failure(ctx: &Ctx) -> String {
        let err = fetch(ctx, &MrKey::new("acme/widgets", 42)).await.unwrap_err();
        ctx.with_host_hint(err).to_string()
    }

    #[tokio::test]
    async fn an_mr_not_found_on_the_host_of_the_checkout_names_the_host_and_suggests_host() {
        let (_server, ctx) = github_without_the_mr(HostOrigin::Checkout).await;
        assert_eq!(
            failure(&ctx).await,
            "acme/widgets#42 was not found on github.com (the host of this checkout): pass --host to ask another forge"
        );
    }

    #[tokio::test]
    async fn an_mr_not_found_on_a_named_host_does_not_suggest_host() {
        let (_server, ctx) = github_without_the_mr(HostOrigin::Named).await;
        assert_eq!(failure(&ctx).await, "acme/widgets#42 was not found on github.com, or this token cannot see it");
    }

    #[tokio::test]
    async fn an_mr_not_found_on_the_config_or_default_host_says_which_it_is() {
        for (host_origin, why) in [(HostOrigin::Config, "the host in the config"), (HostOrigin::Default, "the default host")] {
            let (_server, ctx) = gitlab_answering(404, host_origin).await;
            assert_eq!(
                failure(&ctx).await,
                format!("acme/widgets!42 was not found on 127.0.0.1 ({why}): pass --host to ask another forge")
            );
        }
    }

    #[tokio::test]
    async fn another_http_failure_keeps_its_message() {
        for status in [500, 401] {
            let (_server, ctx) = gitlab_answering(status, HostOrigin::Checkout).await;
            let said = failure(&ctx).await;
            assert!(said.contains(&format!("HTTP {status}")) && !said.contains("--host"), "{said}");
        }
    }

    /// What `revu show` prints of an MR whose title, author, branch, file path and comment the forge answers through `mark`.
    async fn shown_through_gitlab(mark: fn(&str) -> String) -> String {
        let server = MockServer::start().await;
        let mr = serde_json::json!({"id": 1042, "iid": 42, "project_id": 7, "title": mark("feat: charge cards"), "state": "opened", "draft": false,
            "author": {"id": 5, "username": mark("omar"), "name": "Omar"}, "source_branch": mark("feat/checkout"), "target_branch": "main",
            "web_url": "https://gitlab.com/acme/widgets/-/merge_requests/42", "updated_at": "2026-09-22T10:00:00Z", "sha": "bbbb",
            "diff_refs": {"base_sha": "aaaa", "head_sha": "bbbb", "start_sha": "aaaa"}});
        let diffs = serde_json::json!([{"diff": "@@ -1 +1 @@\n-a\n+b\n", "old_path": mark("src/pay.rs"), "new_path": mark("src/pay.rs")}]);
        let note = serde_json::json!({"id": 502, "body": mark("Should this retry?"), "author": {"id": 3, "username": mark("lea"), "name": "Léa"},
            "created_at": "2026-09-22T10:05:00Z", "updated_at": "2026-09-22T10:05:00Z", "system": false, "resolvable": true, "resolved": false});
        let discussions = serde_json::json!([{"id": "9f2c", "individual_note": false, "notes": [note]}]);
        let mr_path = "/api/v4/projects/acme%2Fwidgets/merge_requests/42";
        for (at, answer) in
            [("", mr), ("/diffs", diffs), ("/discussions", discussions), ("/approvals", serde_json::json!({"approved": false}))]
        {
            let answered = ResponseTemplate::new(200).set_body_json(answer);
            Mock::given(method("GET")).and(path(format!("{mr_path}{at}"))).respond_with(answered).mount(&server).await;
        }
        let forge = gitlab_on(&server);
        let cache = tempfile::tempdir().unwrap();
        let ctx = Ctx { cache: Cache::in_dir(cache.path()), ..ctx(forge, HostOrigin::Named) };
        let (mr, diffs, discussions) = fetch(&ctx, &MrKey::new("acme/widgets", 42)).await.unwrap();
        let now = Utc.with_ymd_and_hms(2026, 9, 22, 12, 0, 0).unwrap();
        text(Kind::GitLab, &mr, &diffs, &discussions, Theme::plain(), now)
    }

    #[tokio::test]
    async fn forge_text_carrying_escapes_bidi_and_invisible_characters_prints_as_its_clean_text_and_keeps_the_columns() {
        let hostile = |text: &str| {
            let (head, tail) = text.split_at(2);
            format!("\x1b{head}\u{9b}\u{202e}\u{2066}\u{200b}{tail}\u{2069}\u{e0041}\x07\r")
        };
        assert_eq!(shown_through_gitlab(hostile).await, shown_through_gitlab(str::to_owned).await);
    }

    #[test]
    fn file_stats_count_added_and_removed_lines() {
        let stat = FileStat::from(&diffs()[0]);
        assert_eq!((stat.additions, stat.deletions), (2, 1));
    }
}
