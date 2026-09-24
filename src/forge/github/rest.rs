//! What GitHub answers best over REST: who I am, the changed files, public comments, approvals
//! and the lookups the command line needs.
use super::Client;
use super::wire::{self, RestUser};
use crate::forge::checks::{Checks, Found, Job, JobState};
use crate::forge::{self, DiffFile, Discussion, MrKey, Note, Position};
use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
struct Repo {
    full_name: String,
}

#[derive(Deserialize)]
struct PrNumber {
    number: u64,
}

/// One changed file of `pulls/:n/files`: `patch` is the unified body, absent for binary files and
/// for diffs GitHub will not send.
#[derive(Deserialize)]
struct File {
    filename: String,
    #[serde(default)]
    previous_filename: Option<String>,
    status: String,
    #[serde(default)]
    patch: Option<String>,
    #[serde(default)]
    changes: u32,
}

impl From<File> for DiffFile {
    fn from(f: File) -> Self {
        let withheld = f.patch.is_none() && f.changes > 0;
        DiffFile {
            diff: f.patch.map(|p| if p.is_empty() || p.ends_with('\n') { p } else { format!("{p}\n") }).unwrap_or_default(),
            old_path: f.previous_filename.clone().unwrap_or_else(|| f.filename.clone()),
            new_path: f.filename,
            new_file: f.status == "added",
            deleted_file: f.status == "removed",
            renamed_file: f.status == "renamed",
            too_large: withheld,
            ..DiffFile::default()
        }
    }
}

/// A public comment as REST answers it: a line comment of a review or a PR comment.
#[derive(Deserialize)]
struct PostedComment {
    id: u64,
    node_id: String,
    body: String,
    user: RestUser,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl PostedComment {
    fn into_discussion(self, position: Option<Position>) -> Discussion {
        let resolvable = position.is_some();
        let note = Note {
            id: self.id,
            body: self.body,
            author: self.user.into(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            system: false,
            resolvable,
            resolved: false,
            position,
            suggestions: vec![],
        };
        Discussion { id: self.node_id, notes: vec![note] }
    }
}

fn decode(content: &str) -> Result<String> {
    let packed: String = content.split_whitespace().collect();
    let bytes = base64::engine::general_purpose::STANDARD.decode(packed).context("the file came back unreadable")?;
    String::from_utf8(bytes).context("the file is not text")
}

fn repo_path(project: &str) -> String {
    format!("repos/{project}")
}

#[derive(Deserialize)]
struct CheckRuns {
    check_runs: Vec<CheckRun>,
}

#[derive(Deserialize)]
struct CheckRun {
    name: String,
    status: String,
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    completed_at: Option<DateTime<Utc>>,
    html_url: String,
    app: CheckApp,
}

#[derive(Deserialize)]
struct CheckApp {
    name: String,
}

#[derive(Deserialize)]
struct WorkflowRuns {
    workflow_runs: Vec<WorkflowRun>,
}

#[derive(Deserialize)]
struct WorkflowRun {
    id: u64,
    name: String,
}

impl CheckRun {
    /// `status` says whether it finished, `conclusion` how.
    fn state(&self) -> JobState {
        match (self.status.as_str(), self.conclusion.as_deref()) {
            ("completed", Some("success" | "neutral")) => JobState::Passed,
            ("completed", Some("skipped" | "stale")) => JobState::Skipped,
            ("completed", Some("cancelled")) => JobState::Canceled,
            ("completed", Some("action_required")) => JobState::Manual,
            ("completed", _) => JobState::Failed,
            ("in_progress", _) => JobState::Running,
            _ => JobState::Pending,
        }
    }

    /// The Actions run it belongs to, read from its page: `…/actions/runs/<id>/job/<id>`.
    fn run_id(&self) -> Option<u64> {
        self.html_url.split("/actions/runs/").nth(1)?.split('/').next()?.parse().ok()
    }

    /// Grouped under its workflow's name when Actions ran it, else under the app that did.
    fn found(self, workflows: &[WorkflowRun]) -> Found {
        let stage =
            self.run_id().and_then(|id| workflows.iter().find(|w| w.id == id)).map_or_else(|| self.app.name.clone(), |w| w.name.clone());
        let seconds = self.started_at.zip(self.completed_at).map(|(from, to)| (to - from).num_seconds().max(0).unsigned_abs());
        let order = self.started_at.map_or_else(|| "~".to_owned(), |at| at.to_rfc3339());
        let state = self.state();
        Found { stage, order, job: Job { name: self.name, state, seconds, web_url: self.html_url, allowed_to_fail: false } }
    }
}

#[derive(Deserialize)]
struct RepoAccess {
    #[serde(default)]
    permissions: Option<Permissions>,
}

#[derive(Deserialize)]
struct Permissions {
    #[serde(default)]
    push: bool,
}

#[derive(Deserialize)]
struct PullHead {
    head: HeadRef,
}

#[derive(Deserialize)]
struct HeadRef {
    #[serde(default)]
    repo: Option<Repo>,
}

/// A file as the contents API gives it: base64 with line breaks, and the blob it is.
#[derive(Deserialize)]
struct Contents {
    content: String,
    sha: String,
}

impl Client {
    /// GitHub has no API to apply a suggestion: the commit is built here, on the PR's own branch,
    /// and only when I can push to it; a fork's branch or a read-only repo is left to the web.
    pub async fn apply(&self, key: &MrKey, branch: &str, suggestion: &forge::Suggestion) -> Result<()> {
        let repo = repo_path(&key.project);
        let pull_path = format!("{repo}/pulls/{}", key.number);
        let (access, pull) = tokio::try_join!(self.get::<RepoAccess>(&repo), self.get::<PullHead>(&pull_path))?;
        let own_branch = pull.head.repo.is_some_and(|r| r.full_name == key.project);
        if !own_branch || !access.permissions.is_some_and(|p| p.push) {
            bail!("GitHub has no API to apply this suggestion here: o opens it on the web");
        }
        let path = format!("{repo}/contents/{}", suggestion.path);
        let file: Contents = self.get(&format!("{path}?ref={branch}")).await?;
        let text = decode(&file.content)?;
        let proposal =
            crate::review::suggestion::Proposal { above: suggestion.above, below: suggestion.below, text: suggestion.text.clone() };
        let changed = crate::review::suggestion::apply_to(&text, suggestion.line, &proposal)?;
        let body = json!({
            "message": format!("Apply suggestion to {}", suggestion.path),
            "content": base64::engine::general_purpose::STANDARD.encode(changed),
            "sha": file.sha,
            "branch": branch,
        });
        self.put_json::<serde_json::Value>(&path, &body).await.map(|_| ())
    }

    /// The check runs on `head`, grouped by workflow; `None` when nothing ran on it.
    pub async fn checks(&self, key: &MrKey, head: &str) -> Result<Option<Checks>> {
        let repo = repo_path(&key.project);
        let runs_path = format!("{repo}/commits/{head}/check-runs?per_page=100");
        let workflows_path = format!("{repo}/actions/runs?head_sha={head}&per_page=100");
        let (runs, workflows) = tokio::try_join!(self.get::<CheckRuns>(&runs_path), self.get::<WorkflowRuns>(&workflows_path))?;
        if runs.check_runs.is_empty() {
            return Ok(None);
        }
        let found = runs.check_runs.into_iter().map(|run| run.found(&workflows.workflow_runs)).collect();
        let web_url = format!("https://{}/{}/pull/{}/checks", self.host(), key.project, key.number);
        Ok(Some(Checks::from_jobs(Some(web_url), found)))
    }

    pub async fn me(&self) -> Result<forge::User> {
        self.get::<RestUser>("user").await.map(forge::User::from)
    }

    /// The `owner/repo` of the repository with this numeric id.
    pub async fn project_path(&self, id: u64) -> Result<String> {
        let repo: Repo = self.get(&format!("repositories/{id}")).await.with_context(|| format!("repository {id}"))?;
        Ok(repo.full_name)
    }

    /// The open PR whose head is `branch` in the repository itself, if any.
    pub async fn mr_for_branch(&self, project: &str, branch: &str) -> Result<Option<u64>> {
        let owner = project.split('/').next().unwrap_or_default();
        let head: String = url::form_urlencoded::byte_serialize(format!("{owner}:{branch}").as_bytes()).collect();
        let found: Vec<PrNumber> = self.get(&format!("{}/pulls?state=open&head={head}", repo_path(project))).await?;
        Ok(found.first().map(|p| p.number))
    }

    /// The whole file at `sha`, raw, to show the lines around a hunk.
    pub async fn file(&self, project: &str, path: &str, sha: &str) -> Result<String> {
        self.get_raw(&format!("{}/contents/{path}?ref={sha}", repo_path(project))).await
    }

    pub async fn diffs(&self, key: &MrKey) -> Result<Vec<DiffFile>> {
        let files: Vec<File> = self.get_all(&format!("{}/pulls/{}/files", repo_path(&key.project), key.number)).await?;
        Ok(files.into_iter().map(DiffFile::from).collect())
    }

    /// A public comment: on a line of the head commit when `position` is given, else on the PR.
    pub async fn comment(&self, key: &MrKey, body: &str, position: Option<&Position>) -> Result<Discussion> {
        let Some(position) = position else {
            let path = format!("{}/issues/{}/comments", repo_path(&key.project), key.number);
            return self.post_json::<PostedComment>(&path, &json!({"body": body})).await.map(|c| c.into_discussion(None));
        };
        let (side, line) = wire::side_of(position.line).context("the line has no number")?;
        let mut payload = json!({"body": body, "commit_id": position.refs.head, "path": position.path(), "line": line, "side": side});
        if let Some((start_side, start_line)) = position.start.and_then(wire::side_of) {
            payload["start_line"] = json!(start_line);
            payload["start_side"] = json!(start_side);
        }
        let path = format!("{}/pulls/{}/comments", repo_path(&key.project), key.number);
        self.post_json::<PostedComment>(&path, &payload).await.map(|c| c.into_discussion(Some(position.clone())))
    }

    /// An approval is a review of its own. GitHub has no taking it back for the one who gave it.
    /// GitHub deletes the branch itself when the repo says so; `sha` refuses a push made after the reader looked.
    pub async fn merge(&self, key: &MrKey, head: &str, plan: forge::MergePlan) -> Result<()> {
        let method = match plan.method {
            forge::MergeMethod::Merge => "merge",
            forge::MergeMethod::Squash => "squash",
            forge::MergeMethod::Rebase => "rebase",
        };
        let path = format!("{}/pulls/{}/merge", repo_path(&key.project), key.number);
        self.put_json::<serde_json::Value>(&path, &json!({"sha": head, "merge_method": method})).await.map(|_| ()).map_err(merge_refused)
    }

    pub async fn approve(&self, key: &MrKey, approve: bool) -> Result<()> {
        if !approve {
            bail!("GitHub cannot unapprove; request changes or dismiss the review from the web");
        }
        let path = format!("{}/pulls/{}/reviews", repo_path(&key.project), key.number);
        self.post_json::<serde_json::Value>(&path, &json!({"event": "APPROVE"})).await.map(|_| ()).map_err(cannot_approve)
    }
}

/// The answers GitHub gives a merge it will not do, in words the reader can act on.
fn merge_refused(err: anyhow::Error) -> anyhow::Error {
    let text = err.to_string();
    match () {
        () if text.contains("HTTP 409") => anyhow!("the branch moved since you read it: refresh with r and look at the new commits"),
        () if text.contains("HTTP 405") => {
            anyhow!("GitHub will not merge it yet: {}", text.rsplit_once(" 405 ").map_or(text.as_str(), |(_, rest)| rest))
        }
        () if text.contains("HTTP 403") => anyhow!("you are not allowed to merge into this branch"),
        () => err,
    }
}

/// GitHub answers 422 to approving one's own PR, or while a pending review is open.
fn cannot_approve(err: anyhow::Error) -> anyhow::Error {
    let text = err.to_string();
    if text.contains("HTTP 422") { anyhow!("GitHub refused the approval: {}", text.rsplit(" 422 ").next().unwrap_or(&text)) } else { err }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::auth::Credentials;
    use crate::forge::{LineRef, Refs};
    use wiremock::matchers::{body_partial_json, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn key() -> MrKey {
        MrKey::new("acme/widgets", 42)
    }

    fn client(server: &MockServer) -> Client {
        Client::with_base(&Credentials { host: "github.com".into(), token: "ghp_xxxx".into() }, &server.uri()).unwrap()
    }

    fn posted(id: u64) -> serde_json::Value {
        json!({"id": id, "node_id": format!("N_{id}"), "body": "hi", "user": {"id": 2, "login": "nina"}, "created_at": "2026-09-22T10:00:00Z", "updated_at": "2026-09-22T10:00:00Z"})
    }

    #[tokio::test]
    async fn merge_sends_the_head_and_the_method_and_words_the_refusals() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/repos/acme/widgets/pulls/42/merge"))
            .and(body_partial_json(json!({"sha": "bbbb", "merge_method": "squash"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"merged": true, "sha": "cccc"})))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/repos/acme/widgets/pulls/43/merge"))
            .respond_with(
                ResponseTemplate::new(409).set_body_json(json!({"message": "Head branch was modified. Review and try the merge again."})),
            )
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/repos/acme/widgets/pulls/44/merge"))
            .respond_with(ResponseTemplate::new(405).set_body_json(json!({"message": "Required status check \"ci\" is expected."})))
            .mount(&server)
            .await;
        let client = client(&server);
        let plan = forge::MergePlan { method: forge::MergeMethod::Squash, remove_branch: false };
        client.merge(&key(), "bbbb", plan).await.unwrap();
        let moved = client.merge(&MrKey::new("acme/widgets", 43), "bbbb", plan).await.unwrap_err().to_string();
        assert!(moved.contains("the branch moved"), "{moved}");
        let early = client.merge(&MrKey::new("acme/widgets", 44), "bbbb", plan).await.unwrap_err().to_string();
        assert!(early.contains("GitHub will not merge it yet") && early.contains("status check"), "{early}");
    }
    #[tokio::test]
    async fn files_follow_the_pages_and_read_renames_binaries_and_withheld_diffs() {
        let server = MockServer::start().await;
        let next = format!("<{}/repos/acme/widgets/pulls/42/files?per_page=100&page=2>; rel=\"next\"", server.uri());
        Mock::given(method("GET"))
            .and(path("/repos/acme/widgets/pulls/42/files"))
            .and(query_param("page", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_string(include_str!("fixtures/files_2.json")))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/widgets/pulls/42/files"))
            .respond_with(
                ResponseTemplate::new(200).insert_header("link", next.as_str()).set_body_string(include_str!("fixtures/files_1.json")),
            )
            .mount(&server)
            .await;
        let files = client(&server).diffs(&key()).await.unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.new_path.as_str()).collect();
        assert_eq!(paths, ["src/pay/charge.rs", "src/pay/key.rs", "src/pay/money.rs", "assets/logo.png", "Cargo.lock"]);
        assert!(files[0].diff.starts_with("@@ -12,3 +12,4 @@") && files[0].diff.ends_with("added\n"));
        assert!(files[1].new_file);
        assert_eq!((files[2].renamed_file, files[2].old_path.as_str(), files[2].diff.as_str()), (true, "src/pay/amount.rs", ""));
        assert!(!files[3].too_large && files[3].diff.is_empty(), "a binary file has no patch and no changes counted");
        assert!(files[4].too_large);
    }

    #[tokio::test]
    async fn a_line_comment_goes_on_the_head_commit_and_a_plain_one_on_the_conversation() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/acme/widgets/pulls/42/comments"))
            .and(body_partial_json(
                json!({"commit_id": "bbbb", "path": "src/a.rs", "line": 14, "side": "RIGHT", "start_line": 12, "start_side": "RIGHT"}),
            ))
            .respond_with(ResponseTemplate::new(201).set_body_json(posted(7)))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/repos/acme/widgets/issues/42/comments"))
            .and(body_partial_json(json!({"body": "hi"})))
            .respond_with(ResponseTemplate::new(201).set_body_json(posted(8)))
            .expect(1)
            .mount(&server)
            .await;
        let refs = Refs { base: "aaaa".into(), start: "aaaa".into(), head: "bbbb".into() };
        let position = Position {
            refs,
            old_path: "src/a.rs".into(),
            new_path: "src/a.rs".into(),
            line: LineRef { old: None, new: Some(14) },
            start: Some(LineRef { old: Some(12), new: Some(12) }),
        };
        let client = client(&server);
        let on_line = client.comment(&key(), "hi", Some(&position)).await.unwrap();
        assert!(on_line.notes[0].resolvable && on_line.notes[0].position.is_some());
        let plain = client.comment(&key(), "hi", None).await.unwrap();
        assert_eq!((plain.id.as_str(), plain.notes[0].id), ("N_8", 8));
    }

    #[tokio::test]
    async fn lookups_find_the_repository_and_the_pr_of_a_branch() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repositories/7"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"full_name": "acme/widgets"})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/widgets/pulls"))
            .and(query_param("head", "acme:feat/checkout"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"number": 42}])))
            .mount(&server)
            .await;
        let client = client(&server);
        assert_eq!(client.project_path(7).await.unwrap(), "acme/widgets");
        assert_eq!(client.mr_for_branch("acme/widgets", "feat/checkout").await.unwrap(), Some(42));
    }

    #[tokio::test]
    async fn approving_is_a_review_and_unapproving_is_refused_with_the_way_out() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/acme/widgets/pulls/42/reviews"))
            .and(body_partial_json(json!({"event": "APPROVE"})))
            .respond_with(
                ResponseTemplate::new(422)
                    .set_body_json(json!({"message": "Unprocessable Entity", "errors": ["Can not approve your own pull request"]})),
            )
            .mount(&server)
            .await;
        let client = client(&server);
        let err = client.approve(&key(), true).await.unwrap_err().to_string();
        assert!(err.contains("refused") && err.contains("your own pull request"), "{err}");
        let err = client.approve(&key(), false).await.unwrap_err().to_string();
        assert!(err.contains("cannot unapprove"), "{err}");
    }
    #[tokio::test]
    async fn a_file_is_read_raw_at_a_commit() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/widgets/contents/src/pay/charge.rs"))
            .and(query_param("ref", "abc123"))
            .and(wiremock::matchers::header("accept", "application/vnd.github.raw+json"))
            .respond_with(ResponseTemplate::new(200).set_body_string("fn main() {}\n"))
            .mount(&server)
            .await;
        assert_eq!(client(&server).file("acme/widgets", "src/pay/charge.rs", "abc123").await.unwrap(), "fn main() {}\n");
    }
    #[tokio::test]
    async fn checks_group_check_runs_by_their_workflow() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/widgets/commits/beef/check-runs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"total_count": 3, "check_runs": [
                {"name": "flaky", "status": "completed", "conclusion": "failure", "started_at": "2026-09-23T11:10:40Z", "completed_at": "2026-09-23T11:10:44Z",
                 "html_url": "https://github.com/acme/widgets/actions/runs/77/job/3", "app": {"name": "GitHub Actions"}},
                {"name": "lint", "status": "completed", "conclusion": "success", "started_at": "2026-09-23T11:10:33Z", "completed_at": "2026-09-23T11:10:36Z",
                 "html_url": "https://github.com/acme/widgets/actions/runs/77/job/1", "app": {"name": "GitHub Actions"}},
                {"name": "coverage", "status": "queued", "conclusion": null, "started_at": null, "completed_at": null,
                 "html_url": "https://codecov.example/run/9", "app": {"name": "Codecov"}}
            ]})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/widgets/actions/runs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"workflow_runs": [{"id": 77, "name": "ci"}]})))
            .mount(&server)
            .await;
        let checks = client(&server).checks(&key(), "beef").await.unwrap().unwrap();
        let stages: Vec<(&str, Vec<(&str, JobState)>)> =
            checks.stages.iter().map(|s| (s.name.as_str(), s.jobs.iter().map(|j| (j.name.as_str(), j.state)).collect())).collect();
        assert_eq!(
            stages,
            vec![("ci", vec![("flaky", JobState::Failed), ("lint", JobState::Passed)]), ("Codecov", vec![("coverage", JobState::Pending)])]
        );
        assert_eq!(checks.web_url.as_deref(), Some("https://github.com/acme/widgets/pull/42/checks"));
        assert_eq!(checks.stages[0].jobs[0].seconds, Some(4));
    }

    fn suggestion() -> forge::Suggestion {
        forge::Suggestion { id: None, path: "src/a.rs".into(), line: 2, above: 0, below: 0, text: "B".into() }
    }

    async fn repo_and_pull(server: &MockServer, push: bool, head_repo: &str) {
        Mock::given(method("GET"))
            .and(path("/repos/acme/widgets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"full_name": "acme/widgets", "permissions": {"push": push}})))
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/widgets/pulls/42"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"head": {"repo": {"full_name": head_repo}}})))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn a_suggestion_becomes_a_commit_on_my_own_branch() {
        let server = MockServer::start().await;
        repo_and_pull(&server, true, "acme/widgets").await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/widgets/contents/src/a.rs"))
            .and(query_param("ref", "feat/sum"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"content": "YQpi\nCmMK\n", "sha": "blob1"})))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/repos/acme/widgets/contents/src/a.rs"))
            .and(body_partial_json(json!({"content": "YQpCCmMK", "sha": "blob1", "branch": "feat/sum"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"commit": {"sha": "c0ffee"}})))
            .expect(1)
            .mount(&server)
            .await;
        client(&server).apply(&key(), "feat/sum", &suggestion()).await.unwrap();
    }

    #[tokio::test]
    async fn a_fork_or_a_read_only_repo_is_left_to_the_web() {
        let server = MockServer::start().await;
        repo_and_pull(&server, true, "someone/widgets").await;
        let err = client(&server).apply(&key(), "feat/sum", &suggestion()).await.unwrap_err().to_string();
        assert!(err.contains("o opens it on the web"), "{err}");
    }
}
