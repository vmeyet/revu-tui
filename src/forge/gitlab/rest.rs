//! One MR over REST: the MR, its diffs, its discussions and my drafts, and every write.
use super::Client;
use super::wire::{self, Approvals, Discussion, DraftNote, NewDraft};
use crate::forge::checks::{Checks, Found, Job, JobState};
use crate::forge::{self, DiffFile, MrKey};
use anyhow::{Context, Result, anyhow};
use reqwest::Method;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
struct Project {
    path_with_namespace: String,
}

#[derive(Deserialize)]
struct MrIid {
    iid: u64,
}

/// GitLab takes the URL-encoded path wherever it takes a numeric project id.
pub(super) fn project_path(project: &str) -> String {
    format!("projects/{}", url_encode(project))
}

fn mr_path(key: &MrKey) -> String {
    format!("{}/merge_requests/{}", project_path(&key.project), key.number)
}

/// The newest pipeline of an MR, as `merge_requests/:iid/pipelines` lists it (newest first).
#[derive(Deserialize)]
struct PipelineRef {
    id: u64,
    #[serde(default)]
    web_url: Option<String>,
}

#[derive(Deserialize)]
struct JobWire {
    id: u64,
    name: String,
    stage: String,
    status: String,
    #[serde(default)]
    duration: Option<f64>,
    web_url: String,
    #[serde(default)]
    allow_failure: bool,
}

impl From<JobWire> for Found {
    fn from(job: JobWire) -> Self {
        let state = match job.status.as_str() {
            "success" => JobState::Passed,
            "failed" => JobState::Failed,
            "running" => JobState::Running,
            "manual" => JobState::Manual,
            "canceled" | "canceling" => JobState::Canceled,
            "skipped" => JobState::Skipped,
            _ => JobState::Pending,
        };
        Found {
            stage: job.stage,
            order: format!("{:020}", job.id),
            job: Job {
                name: job.name,
                state,
                seconds: job.duration.map(|d| d.round() as u64),
                web_url: job.web_url,
                allowed_to_fail: job.allow_failure,
            },
        }
    }
}

impl Client {
    /// The path of the project with this numeric id.
    pub async fn project_path(&self, id: u64) -> Result<String> {
        let project: Project = self.get(&format!("projects/{id}")).await.with_context(|| format!("project {id}"))?;
        Ok(project.path_with_namespace)
    }

    /// The open MR whose source is `branch`, if any.
    pub async fn mr_for_branch(&self, project: &str, branch: &str) -> Result<Option<u64>> {
        let path = format!("{}/merge_requests?state=opened&source_branch={}", project_path(project), url_encode(branch));
        let found: Vec<MrIid> = self.get(&path).await?;
        Ok(found.first().map(|m| m.iid))
    }

    /// The MR with its approvals folded in; the requests run together.
    /// Who I am only decides `mine`: when `/user` fails, the MR still loads and merge is not offered.
    pub async fn mr(&self, key: &MrKey) -> Result<forge::Mr> {
        let path = mr_path(key);
        let approvals_path = format!("{path}/approvals");
        let (mr, approvals, me) = tokio::join!(self.get::<wire::Mr>(&path), self.get::<Approvals>(&approvals_path), self.my_name());
        let mr = wire::Mr { approvals: approvals?, ..mr? }.into_model(&key.project);
        let mine = me.is_ok_and(|me| mr.author.username == me);
        Ok(forge::Mr { mine, ..mr })
    }

    /// The whole file at `sha`, to show the lines around a hunk.
    pub async fn file(&self, project: &str, path: &str, sha: &str) -> Result<String> {
        self.get_text(&format!("{}/repository/files/{}/raw?ref={sha}", project_path(project), url_encode(path))).await
    }

    /// GitLab commits the suggestion itself, on the MR's branch, from its id.
    pub async fn apply(&self, suggestion: &forge::Suggestion) -> Result<()> {
        let Some(id) = suggestion.id else { return Err(anyhow!("GitLab did not list this suggestion as appliable")) };
        self.send_empty(Method::PUT, &format!("suggestions/{id}/apply"), None).await
    }

    /// The jobs of the MR's newest pipeline; `None` when it never ran one. GitLab creates a
    /// pipeline's jobs stage by stage, so their ids give the stage order.
    pub async fn checks(&self, key: &MrKey) -> Result<Option<Checks>> {
        let pipelines: Vec<PipelineRef> = self.get(&format!("{}/pipelines?per_page=1", mr_path(key))).await?;
        let Some(newest) = pipelines.into_iter().next() else { return Ok(None) };
        let jobs: Vec<JobWire> = self.get_all(&format!("{}/pipelines/{}/jobs", project_path(&key.project), newest.id)).await?;
        Ok(Some(Checks::from_jobs(newest.web_url, jobs.into_iter().map(Found::from).collect())))
    }

    pub async fn diffs(&self, key: &MrKey) -> Result<Vec<DiffFile>> {
        self.get_all(&format!("{}/diffs", mr_path(key))).await
    }

    /// The threads, each note with its reactions; the threads still come when reactions cannot be read.
    pub async fn discussions(&self, key: &MrKey) -> Result<Vec<forge::Discussion>> {
        let path = format!("{}/discussions", mr_path(key));
        let (discussions, awards) = tokio::join!(self.get_all::<Discussion>(&path), self.awards(key));
        let awards = awards.unwrap_or_default();
        Ok(discussions?.into_iter().map(|d| with_awards(forge::Discussion::from(d), &awards)).collect())
    }

    pub async fn drafts(&self, key: &MrKey) -> Result<Vec<forge::Draft>> {
        let drafts: Vec<DraftNote> = self.get_all(&format!("{}/draft_notes", mr_path(key))).await?;
        Ok(drafts.into_iter().map(forge::Draft::from).collect())
    }

    pub async fn create_draft(&self, key: &MrKey, draft: &forge::NewDraft) -> Result<forge::Draft> {
        let body = serde_json::to_value(NewDraft::from(draft))?;
        self.post_json::<DraftNote>(&format!("{}/draft_notes", mr_path(key)), &body).await.map(forge::Draft::from)
    }

    /// The whole draft goes again: GitLab drops the position of a draft updated with its text alone.
    pub async fn update_draft(&self, key: &MrKey, id: u64, draft: &forge::NewDraft) -> Result<forge::Draft> {
        let body = serde_json::to_value(NewDraft::from(draft))?;
        self.put_json::<DraftNote>(&format!("{}/draft_notes/{id}", mr_path(key)), &body).await.map(forge::Draft::from)
    }

    pub async fn delete_draft(&self, key: &MrKey, id: u64) -> Result<()> {
        self.delete(&format!("{}/draft_notes/{id}", mr_path(key))).await
    }

    /// Every draft of mine on the MR becomes public at once, as one review; then the approval, when asked.
    pub async fn publish(&self, key: &MrKey, approve: bool) -> Result<()> {
        self.send_empty(Method::POST, &format!("{}/draft_notes/bulk_publish", mr_path(key)), None).await?;
        if approve {
            self.approve(key, true).await?;
        }
        Ok(())
    }

    pub async fn resolve(&self, key: &MrKey, discussion: &str, resolved: bool) -> Result<()> {
        let path = format!("{}/discussions/{discussion}", mr_path(key));
        self.put_json::<Discussion>(&path, &json!({"resolved": resolved})).await.map(|_| ())
    }

    /// A public new thread, on a line when `position` is given.
    pub async fn comment(&self, key: &MrKey, body: &str, position: Option<&forge::Position>) -> Result<forge::Discussion> {
        let payload = match position {
            Some(position) => json!({"body": body, "position": wire::Position::from_model(position)}),
            None => json!({"body": body}),
        };
        self.post_json::<Discussion>(&format!("{}/discussions", mr_path(key)), &payload).await.map(forge::Discussion::from)
    }

    /// Approves, or takes my approval back.
    /// GitLab applies the project's merge method; revu passes the squash and branch choices it
    /// showed, and `sha` so a push made after the reader looked is refused.
    pub async fn merge(&self, key: &MrKey, head: &str, plan: forge::MergePlan) -> Result<()> {
        let body = json!({
            "sha": head,
            "squash": plan.method == forge::MergeMethod::Squash,
            "should_remove_source_branch": plan.remove_branch,
        });
        self.put_json::<serde_json::Value>(&format!("{}/merge", mr_path(key)), &body).await.map(|_| ()).map_err(merge_refused)
    }

    pub async fn approve(&self, key: &MrKey, approve: bool) -> Result<()> {
        let verb = if approve { "approve" } else { "unapprove" };
        self.send_empty(Method::POST, &format!("{}/{verb}", mr_path(key)), None).await.map_err(cannot_approve)
    }
}

fn with_awards(discussion: forge::Discussion, awards: &super::award::Awards) -> forge::Discussion {
    let notes = discussion
        .notes
        .into_iter()
        .map(|note| match awards.get(&note.id) {
            Some((node, reactions)) => forge::Note { node: Some(node.clone()), reactions: reactions.clone(), ..note },
            None => note,
        })
        .collect();
    forge::Discussion { notes, ..discussion }
}

/// The answers GitLab gives a merge it will not do, in words the reader can act on.
fn merge_refused(err: anyhow::Error) -> anyhow::Error {
    let text = err.to_string();
    match () {
        () if text.contains("HTTP 409") => anyhow!("the branch moved since you read it: refresh with r and look at the new commits"),
        () if text.contains("HTTP 405") || text.contains("HTTP 406") => {
            anyhow!("GitLab will not merge it yet: {}", text.rsplit_once(": HTTP ").map_or(text.as_str(), |(_, rest)| rest))
        }
        () if text.contains("HTTP 401") || text.contains("HTTP 403") => anyhow!("you are not allowed to merge into this branch"),
        () => err,
    }
}

/// GitLab answers 401 to an approval the token owner is not allowed to give (own MR, approval rules).
fn cannot_approve(err: anyhow::Error) -> anyhow::Error {
    if err.to_string().contains("HTTP 401") { anyhow!("you cannot approve this MR (own MR or approval rules)") } else { err }
}

fn url_encode(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::auth::Credentials;
    use crate::forge::gitlab::fixture::{key, parse};
    use crate::forge::{LineRef, Refs};
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(server: &MockServer) -> Client {
        Client::with_base(&Credentials { host: "x".into(), token: "glpat-xxxx".into() }, &format!("{}/api/v4/", server.uri())).unwrap()
    }

    fn at_line(new_line: u32) -> forge::Position {
        let refs = Refs { base: "a".into(), start: "a".into(), head: "b".into() };
        let line = LineRef { old: None, new: Some(new_line) };
        forge::Position { refs, old_path: "src/a.rs".into(), new_path: "src/a.rs".into(), line, start: None }
    }

    fn mr_json() -> serde_json::Value {
        json!({
            "id": 1042, "iid": 42, "project_id": 7, "title": "feat: charge cards at checkout", "description": "## Why\n…",
            "state": "opened", "draft": false,
            "author": {"id": 5, "username": "omar", "name": "Omar", "avatar_url": null},
            "source_branch": "feat/checkout", "target_branch": "main",
            "web_url": "https://gitlab.com/acme/widgets/-/merge_requests/42",
            "updated_at": "2026-09-22T09:12:00Z", "sha": "bbbb",
            "diff_refs": {"base_sha": "aaaa", "head_sha": "bbbb", "start_sha": "aaaa"},
            "head_pipeline": {"id": 1, "status": "success", "web_url": "https://gitlab.com/acme/widgets/-/pipelines/1"},
            "changes_count": "9", "has_conflicts": false, "blocking_discussions_resolved": false,
            "reviewers": [{"id": 2, "username": "nina", "name": "Nina", "avatar_url": null}],
            "labels": ["payments"]
        })
    }

    #[tokio::test]
    async fn project_ids_resolve_to_paths_and_branches_are_looked_up_by_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/7"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 7, "path_with_namespace": "acme/widgets"})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/acme%2Fwidgets/merge_requests"))
            .and(query_param("source_branch", "feat/checkout"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"iid": 42, "title": "x"}])))
            .mount(&server)
            .await;
        let client = client(&server);
        assert_eq!(client.project_path(7).await.unwrap(), "acme/widgets");
        assert_eq!(client.mr_for_branch("acme/widgets", "feat/checkout").await.unwrap(), Some(42));
    }

    #[tokio::test]
    async fn merge_sends_the_head_and_the_choices_shown_and_words_the_refusals() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v4/projects/acme%2Fwidgets/merge_requests/42/merge"))
            .and(body_partial_json(json!({"sha": "bbbb", "squash": true, "should_remove_source_branch": true})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"iid": 42, "state": "merged"})))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/api/v4/projects/acme%2Fwidgets/merge_requests/43/merge"))
            .respond_with(ResponseTemplate::new(409).set_body_json(json!({"message": "SHA does not match HEAD of source branch"})))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/api/v4/projects/acme%2Fwidgets/merge_requests/44/merge"))
            .respond_with(ResponseTemplate::new(405).set_body_json(json!({"message": "405 Method Not Allowed"})))
            .mount(&server)
            .await;
        let client = client(&server);
        let plan = forge::MergePlan { method: forge::MergeMethod::Squash, remove_branch: true };
        client.merge(&key(), "bbbb", plan).await.unwrap();
        let moved = client.merge(&MrKey::new("acme/widgets", 43), "bbbb", plan).await.unwrap_err().to_string();
        assert!(moved.contains("the branch moved"), "{moved}");
        let early = client.merge(&MrKey::new("acme/widgets", 44), "bbbb", plan).await.unwrap_err().to_string();
        assert!(early.starts_with("GitLab will not merge it yet"), "{early}");
    }

    #[test]
    fn the_squash_and_branch_settings_become_the_merge_plan() {
        let mut mr = mr_json();
        mr["squash_on_merge"] = json!(true);
        mr["force_remove_source_branch"] = json!(true);
        let plan = parse::<wire::Mr>(&mr.to_string()).into_model("acme/widgets").merge;
        assert_eq!(plan, forge::MergePlan { method: forge::MergeMethod::Squash, remove_branch: true });
        let plain = parse::<wire::Mr>(&mr_json().to_string()).into_model("acme/widgets").merge;
        assert_eq!(plain, forge::MergePlan::default());
    }

    #[tokio::test]
    async fn an_mr_still_loads_when_who_i_am_cannot_be_asked() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/acme%2Fwidgets/merge_requests/42"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mr_json()))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/acme%2Fwidgets/merge_requests/42/approvals"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"approved": false, "approvals_left": 1})))
            .mount(&server)
            .await;
        Mock::given(method("GET")).and(path("/api/v4/user")).respond_with(ResponseTemplate::new(500)).mount(&server).await;
        let mr = client(&server).mr(&key()).await.unwrap();
        assert!(!mr.mine, "unknown owner: merge is simply not offered");
    }

    #[tokio::test]
    async fn mr_folds_the_approvals_in() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/acme%2Fwidgets/merge_requests/42"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mr_json()))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/acme%2Fwidgets/merge_requests/42/approvals"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "approved": true, "approvals_left": 0, "user_has_approved": false, "user_can_approve": true,
                "approved_by": [{"user": {"id": 3, "username": "lea", "name": "Léa"}}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v4/user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 9, "username": "vivien", "name": "Vivien"})))
            .expect(1)
            .mount(&server)
            .await;
        let client = client(&server);
        let mr = client.mr(&key()).await.unwrap();
        client.mr(&key()).await.unwrap();
        assert_eq!((mr.project.as_str(), mr.number), ("acme/widgets", 42));
        assert!(!mr.mine, "omar wrote it, not vivien; /user is asked once for both reads");
        assert_eq!((mr.changes_count.as_deref(), mr.labels.as_slice()), (Some("9"), &["payments".to_owned()][..]));
        assert_eq!(mr.pipeline.as_ref().map(|p| p.status.as_str()), Some("success"));
        assert_eq!(mr.refs, Refs { base: "aaaa".into(), start: "aaaa".into(), head: "bbbb".into() });
        assert!(mr.approvals.approved && mr.approvals.user_can_approve);
        assert_eq!(mr.approvals.approved_by[0].username, "lea");
    }

    #[tokio::test]
    async fn diffs_follow_the_next_link_across_pages() {
        let server = MockServer::start().await;
        let page = |n: u32| {
            json!([{"diff": format!("@@ -0,0 +1 @@\n+page {n}\n"), "old_path": format!("f{n}"), "new_path": format!("f{n}"),
                    "a_mode": "0", "b_mode": "100644", "new_file": true, "renamed_file": false, "deleted_file": false,
                    "generated_file": false, "too_large": false, "collapsed": false}])
        };
        let next = format!("<{}/api/v4/projects/acme%2Fwidgets/merge_requests/42/diffs?page=2&per_page=100>; rel=\"next\"", server.uri());
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/acme%2Fwidgets/merge_requests/42/diffs"))
            .and(query_param("page", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page(2)))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/acme%2Fwidgets/merge_requests/42/diffs"))
            .and(query_param("per_page", "100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page(1)).insert_header("Link", next.as_str()))
            .mount(&server)
            .await;
        let files = client(&server).diffs(&key()).await.unwrap();
        assert_eq!(files.iter().map(|f| f.new_path.as_str()).collect::<Vec<_>>(), ["f1", "f2"]);
        assert!(files[0].new_file);
    }

    fn draft_json(id: u64) -> serde_json::Value {
        json!({"id": id, "author_id": 2, "merge_request_id": 1042, "note": "nit", "discussion_id": null,
               "resolve_discussion": false, "line_code": null, "position": null})
    }

    #[tokio::test]
    async fn drafts_are_listed_created_updated_deleted_and_published() {
        let server = MockServer::start().await;
        let base = "/api/v4/projects/acme%2Fwidgets/merge_requests/42/draft_notes";
        Mock::given(method("GET"))
            .and(path(base))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([draft_json(1)])))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(base))
            .and(body_partial_json(json!({"note": "nit", "position": {"new_line": 13, "position_type": "text"}})))
            .respond_with(ResponseTemplate::new(201).set_body_json(draft_json(2)))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path(format!("{base}/2")))
            .and(body_partial_json(json!({"note": "nit: renamed", "position": {"new_line": 13, "head_sha": "b"}})))
            .respond_with(ResponseTemplate::new(200).set_body_json(draft_json(2)))
            .mount(&server)
            .await;
        Mock::given(method("DELETE")).and(path(format!("{base}/2"))).respond_with(ResponseTemplate::new(204)).mount(&server).await;
        Mock::given(method("POST")).and(path(format!("{base}/bulk_publish"))).respond_with(ResponseTemplate::new(204)).mount(&server).await;
        let client = client(&server);
        let draft = forge::NewDraft { body: "nit".into(), position: Some(at_line(13)), ..forge::NewDraft::default() };
        assert_eq!(client.drafts(&key()).await.unwrap()[0].id, 1);
        assert_eq!(client.create_draft(&key(), &draft).await.unwrap().id, 2);
        let renamed = forge::NewDraft { body: "nit: renamed".into(), ..draft.clone() };
        assert_eq!(client.update_draft(&key(), 2, &renamed).await.unwrap().id, 2);
        client.delete_draft(&key(), 2).await.unwrap();
        client.publish(&key(), false).await.unwrap();
        assert_eq!(server.received_requests().await.unwrap().len(), 5);
    }

    #[tokio::test]
    async fn threads_are_resolved_and_opened() {
        let server = MockServer::start().await;
        let base = "/api/v4/projects/acme%2Fwidgets/merge_requests/42/discussions";
        let discussion: serde_json::Value = serde_json::from_str(include_str!("fixtures/discussions.json")).unwrap();
        Mock::given(method("PUT"))
            .and(path(format!("{base}/6a9c1750")))
            .and(body_partial_json(json!({"resolved": true})))
            .respond_with(ResponseTemplate::new(200).set_body_json(&discussion))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(base))
            .and(body_partial_json(json!({"body": "why?", "position": {"old_path": "x", "new_path": "x", "new_line": 3}})))
            .respond_with(ResponseTemplate::new(201).set_body_json(&discussion))
            .mount(&server)
            .await;
        let client = client(&server);
        client.resolve(&key(), "6a9c1750", true).await.unwrap();
        let position = forge::Position { old_path: "x".into(), new_path: "x".into(), ..at_line(3) };
        assert_eq!(client.comment(&key(), "why?", Some(&position)).await.unwrap().id, discussion["id"]);
    }

    #[tokio::test]
    async fn approve_explains_a_401_and_unapprove_passes_other_errors_through() {
        let server = MockServer::start().await;
        let base = "/api/v4/projects/acme%2Fwidgets/merge_requests/42";
        Mock::given(method("POST")).and(path(format!("{base}/approve"))).respond_with(ResponseTemplate::new(401)).mount(&server).await;
        Mock::given(method("POST"))
            .and(path(format!("{base}/unapprove")))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "404 Not found"})))
            .mount(&server)
            .await;
        let client = client(&server);
        let err = client.approve(&key(), true).await.unwrap_err().to_string();
        assert!(err.contains("cannot approve"), "{err}");
        let err = client.approve(&key(), false).await.unwrap_err().to_string();
        assert!(err.contains("HTTP 404"), "{err}");
    }

    #[tokio::test]
    async fn discussions_parse_diff_notes_and_plain_notes() {
        let server = MockServer::start().await;
        let body = format!("[{},{}]", include_str!("fixtures/discussions.json"), include_str!("fixtures/diff_note.json"));
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/acme%2Fwidgets/merge_requests/42/discussions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body).insert_header("Content-Type", "application/json"))
            .mount(&server)
            .await;
        let discussions = client(&server).discussions(&key()).await.unwrap();
        let expected = forge::Discussion::from(parse::<Discussion>(include_str!("fixtures/diff_note.json")));
        assert_eq!(discussions.len(), 2);
        assert_eq!(discussions[1], expected);
    }
    #[tokio::test]
    async fn a_file_is_read_raw_at_a_commit() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/acme%2Fwidgets/repository/files/src%2Fpay%2Fcharge.rs/raw"))
            .and(query_param("ref", "abc123"))
            .respond_with(ResponseTemplate::new(200).set_body_string("fn main() {}\n"))
            .mount(&server)
            .await;
        assert_eq!(client(&server).file("acme/widgets", "src/pay/charge.rs", "abc123").await.unwrap(), "fn main() {}\n");
    }
    #[tokio::test]
    async fn checks_read_the_newest_pipeline_and_group_its_jobs_by_stage() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/acme%2Fwidgets/merge_requests/42/pipelines"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"id": 900, "status": "failed", "web_url": "https://x/p/900"}])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/acme%2Fwidgets/pipelines/900/jobs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"id": 3, "name": "flaky", "stage": "test", "status": "failed", "duration": 7.3, "web_url": "https://x/j/3", "allow_failure": false},
                {"id": 2, "name": "unit", "stage": "test", "status": "success", "duration": 12.2, "web_url": "https://x/j/2"},
                {"id": 1, "name": "lint", "stage": "check", "status": "success", "duration": 7.1, "web_url": "https://x/j/1"}
            ])))
            .mount(&server)
            .await;
        let checks = client(&server).checks(&key()).await.unwrap().unwrap();
        assert_eq!(checks.web_url.as_deref(), Some("https://x/p/900"));
        let jobs: Vec<String> = checks
            .stages
            .iter()
            .flat_map(|s| s.jobs.iter().map(move |j| format!("{} {} {:?} {:?}", s.name, j.name, j.state, j.seconds)))
            .collect();
        assert_eq!(jobs, ["check lint Passed Some(7)", "test flaky Failed Some(7)", "test unit Passed Some(12)"]);
    }

    #[tokio::test]
    async fn an_mr_without_a_pipeline_has_no_checks() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/acme%2Fwidgets/merge_requests/42/pipelines"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;
        assert_eq!(client(&server).checks(&key()).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_suggestion_is_applied_by_its_id() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v4/suggestions/77/apply"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 77})))
            .expect(1)
            .mount(&server)
            .await;
        let suggestion = forge::Suggestion { id: Some(77), path: "src/a.rs".into(), line: 3, above: 0, below: 0, text: "x".into() };
        client(&server).apply(&suggestion).await.unwrap();
        let unlisted = forge::Suggestion { id: None, ..suggestion };
        assert!(client(&server).apply(&unlisted).await.unwrap_err().to_string().contains("appliable"));
    }
}
