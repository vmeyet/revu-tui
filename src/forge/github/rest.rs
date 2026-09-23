//! What GitHub answers best over REST: who I am, the changed files, public comments, approvals
//! and the lookups the command line needs.
use super::Client;
use super::wire::{self, RestUser};
use crate::forge::checks::{Checks, Found, Job, JobState};
use crate::forge::{self, DiffFile, Discussion, MrKey, Note, Position};
use anyhow::{Context, Result, anyhow, bail};
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
        };
        Discussion { id: self.node_id, notes: vec![note] }
    }
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

impl Client {
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
    pub async fn approve(&self, key: &MrKey, approve: bool) -> Result<()> {
        if !approve {
            bail!("GitHub cannot unapprove; request changes or dismiss the review from the web");
        }
        let path = format!("{}/pulls/{}/reviews", repo_path(&key.project), key.number);
        self.post_json::<serde_json::Value>(&path, &json!({"event": "APPROVE"})).await.map(|_| ()).map_err(cannot_approve)
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
}
