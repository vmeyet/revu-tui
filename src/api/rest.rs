use super::Client;
use super::types::{Approvals, DiffFile, Discussion, DraftNote, Mr, NewDraft, Note, Position};
use anyhow::{Context, Result, anyhow};
use reqwest::Method;
use serde::Deserialize;
use serde_json::json;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Project {
    pub id: u64,
    pub path_with_namespace: String,
}

#[derive(Deserialize)]
struct MrIid {
    iid: u64,
}

fn mr_path(project_id: u64, iid: u64) -> String {
    format!("projects/{project_id}/merge_requests/{iid}")
}

impl Client {
    /// A project by its `group/project` path.
    pub async fn project(&self, path: &str) -> Result<Project> {
        let encoded = path.replace('/', "%2F");
        self.get(&format!("projects/{encoded}")).await.with_context(|| format!("project {path}"))
    }

    /// The open MR whose source is `branch`, if any.
    pub async fn mr_for_branch(&self, project_id: u64, branch: &str) -> Result<Option<u64>> {
        let path = format!("projects/{project_id}/merge_requests?state=opened&source_branch={}", url_encode(branch));
        let found: Vec<MrIid> = self.get(&path).await?;
        Ok(found.first().map(|m| m.iid))
    }

    /// The MR with its approvals folded in; the two requests run together.
    pub async fn mr(&self, project_id: u64, iid: u64) -> Result<Mr> {
        let path = mr_path(project_id, iid);
        let (mr, approvals) = tokio::try_join!(self.get::<Mr>(&path), self.approvals(project_id, iid))?;
        Ok(Mr { approvals, ..mr })
    }

    pub async fn approvals(&self, project_id: u64, iid: u64) -> Result<Approvals> {
        self.get(&format!("{}/approvals", mr_path(project_id, iid))).await
    }

    pub async fn diffs(&self, project_id: u64, iid: u64) -> Result<Vec<DiffFile>> {
        self.get_all(&format!("{}/diffs", mr_path(project_id, iid))).await
    }

    pub async fn discussions(&self, project_id: u64, iid: u64) -> Result<Vec<Discussion>> {
        self.get_all(&format!("{}/discussions", mr_path(project_id, iid))).await
    }

    pub async fn draft_notes(&self, project_id: u64, iid: u64) -> Result<Vec<DraftNote>> {
        self.get_all(&format!("{}/draft_notes", mr_path(project_id, iid))).await
    }

    pub async fn create_draft(&self, project_id: u64, iid: u64, draft: &NewDraft) -> Result<DraftNote> {
        self.post_json(&format!("{}/draft_notes", mr_path(project_id, iid)), &serde_json::to_value(draft)?).await
    }

    /// The whole draft goes again: GitLab drops the position of a draft updated with its text alone.
    pub async fn update_draft(&self, project_id: u64, iid: u64, id: u64, draft: &NewDraft) -> Result<DraftNote> {
        self.put_json(&format!("{}/draft_notes/{id}", mr_path(project_id, iid)), &serde_json::to_value(draft)?).await
    }

    pub async fn delete_draft(&self, project_id: u64, iid: u64, id: u64) -> Result<()> {
        self.delete(&format!("{}/draft_notes/{id}", mr_path(project_id, iid))).await
    }

    pub async fn publish_draft(&self, project_id: u64, iid: u64, id: u64) -> Result<()> {
        self.send_empty(Method::PUT, &format!("{}/draft_notes/{id}/publish", mr_path(project_id, iid)), None).await
    }

    /// Every draft of mine on the MR becomes public at once, as one review.
    pub async fn publish_drafts(&self, project_id: u64, iid: u64) -> Result<()> {
        self.send_empty(Method::POST, &format!("{}/draft_notes/bulk_publish", mr_path(project_id, iid)), None).await
    }

    pub async fn resolve(&self, project_id: u64, iid: u64, discussion_id: &str, resolved: bool) -> Result<Discussion> {
        self.put_json(&format!("{}/discussions/{discussion_id}", mr_path(project_id, iid)), &json!({"resolved": resolved})).await
    }

    /// A public reply in an existing thread.
    pub async fn reply(&self, project_id: u64, iid: u64, discussion_id: &str, body: &str) -> Result<Note> {
        self.post_json(&format!("{}/discussions/{discussion_id}/notes", mr_path(project_id, iid)), &json!({"body": body})).await
    }

    /// A public new thread, on a line when `position` is given.
    pub async fn comment(&self, project_id: u64, iid: u64, body: &str, position: Option<&Position>) -> Result<Discussion> {
        let payload = match position {
            Some(position) => json!({"body": body, "position": position}),
            None => json!({"body": body}),
        };
        self.post_json(&format!("{}/discussions", mr_path(project_id, iid)), &payload).await
    }

    pub async fn approve(&self, project_id: u64, iid: u64) -> Result<()> {
        self.send_empty(Method::POST, &format!("{}/approve", mr_path(project_id, iid)), None).await.map_err(cannot_approve)
    }

    pub async fn unapprove(&self, project_id: u64, iid: u64) -> Result<()> {
        self.send_empty(Method::POST, &format!("{}/unapprove", mr_path(project_id, iid)), None).await.map_err(cannot_approve)
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
    use super::*;
    use crate::api::types::from_fixture;
    use crate::auth::Credentials;
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn client(server: &MockServer) -> Client {
        Client::with_base(&Credentials { host: "x".into(), token: "glpat-xxxx".into() }, &format!("{}/api/v4/", server.uri())).unwrap()
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
    async fn project_paths_are_encoded_and_branches_looked_up() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/acme%2Fwidgets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 7, "path_with_namespace": "acme/widgets"})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/7/merge_requests"))
            .and(query_param("source_branch", "feat/checkout"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"iid": 42, "title": "x"}])))
            .mount(&server)
            .await;
        let client = client(&server).await;
        assert_eq!(client.project("acme/widgets").await.unwrap().id, 7);
        assert_eq!(client.mr_for_branch(7, "feat/checkout").await.unwrap(), Some(42));
    }

    #[tokio::test]
    async fn mr_folds_the_approvals_in() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/7/merge_requests/42"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mr_json()))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/7/merge_requests/42/approvals"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "approved": true, "approvals_left": 0, "user_has_approved": false, "user_can_approve": true,
                "approved_by": [{"user": {"id": 3, "username": "lea", "name": "Léa"}}]
            })))
            .mount(&server)
            .await;
        let mr = client(&server).await.mr(7, 42).await.unwrap();
        assert_eq!((mr.iid, mr.changes_count.as_deref(), mr.labels.as_slice()), (42, Some("9"), &["payments".to_owned()][..]));
        assert_eq!(mr.head_pipeline.as_ref().map(|p| p.status.as_str()), Some("success"));
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
        let next = format!("<{}/api/v4/projects/7/merge_requests/42/diffs?page=2&per_page=100>; rel=\"next\"", server.uri());
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/7/merge_requests/42/diffs"))
            .and(query_param("page", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page(2)))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/7/merge_requests/42/diffs"))
            .and(query_param("per_page", "100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page(1)).insert_header("Link", next.as_str()))
            .mount(&server)
            .await;
        let files = client(&server).await.diffs(7, 42).await.unwrap();
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
        let base = "/api/v4/projects/7/merge_requests/42/draft_notes";
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
        Mock::given(method("PUT")).and(path(format!("{base}/1/publish"))).respond_with(ResponseTemplate::new(204)).mount(&server).await;
        Mock::given(method("POST")).and(path(format!("{base}/bulk_publish"))).respond_with(ResponseTemplate::new(204)).mount(&server).await;
        let client = client(&server).await;
        let refs = crate::api::DiffRefs { base_sha: "a".into(), head_sha: "b".into(), start_sha: "a".into() };
        let draft = NewDraft { note: "nit".into(), position: Some(Position::line(&refs, "x", "x", None, Some(13))), ..NewDraft::default() };
        assert_eq!(client.draft_notes(7, 42).await.unwrap()[0].id, 1);
        assert_eq!(client.create_draft(7, 42, &draft).await.unwrap().id, 2);
        let renamed = NewDraft { note: "nit: renamed".into(), ..draft.clone() };
        assert_eq!(client.update_draft(7, 42, 2, &renamed).await.unwrap().id, 2);
        client.delete_draft(7, 42, 2).await.unwrap();
        client.publish_draft(7, 42, 1).await.unwrap();
        client.publish_drafts(7, 42).await.unwrap();
        assert_eq!(server.received_requests().await.unwrap().len(), 6);
    }

    #[tokio::test]
    async fn threads_are_resolved_replied_to_and_opened() {
        let server = MockServer::start().await;
        let base = "/api/v4/projects/7/merge_requests/42/discussions";
        let discussion: serde_json::Value = serde_json::from_str(include_str!("fixtures/discussions.json")).unwrap();
        let note = discussion["notes"][0].clone();
        Mock::given(method("PUT"))
            .and(path(format!("{base}/6a9c1750")))
            .and(body_partial_json(json!({"resolved": true})))
            .respond_with(ResponseTemplate::new(200).set_body_json(&discussion))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(format!("{base}/6a9c1750/notes")))
            .and(body_partial_json(json!({"body": "done"})))
            .respond_with(ResponseTemplate::new(201).set_body_json(&note))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(base))
            .and(body_partial_json(json!({"body": "why?", "position": {"old_path": "x", "new_path": "x", "new_line": 3}})))
            .respond_with(ResponseTemplate::new(201).set_body_json(&discussion))
            .mount(&server)
            .await;
        let client = client(&server).await;
        let refs = crate::api::DiffRefs { base_sha: "a".into(), head_sha: "b".into(), start_sha: "a".into() };
        assert_eq!(client.resolve(7, 42, "6a9c1750", true).await.unwrap().id, discussion["id"]);
        assert_eq!(client.reply(7, 42, "6a9c1750", "done").await.unwrap().body, note["body"]);
        let position = Position::line(&refs, "x", "x", None, Some(3));
        assert_eq!(client.comment(7, 42, "why?", Some(&position)).await.unwrap().id, discussion["id"]);
    }

    #[tokio::test]
    async fn approve_explains_a_401_and_unapprove_passes_other_errors_through() {
        let server = MockServer::start().await;
        let base = "/api/v4/projects/7/merge_requests/42";
        Mock::given(method("POST")).and(path(format!("{base}/approve"))).respond_with(ResponseTemplate::new(401)).mount(&server).await;
        Mock::given(method("POST"))
            .and(path(format!("{base}/unapprove")))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "404 Not found"})))
            .mount(&server)
            .await;
        let client = client(&server).await;
        let err = client.approve(7, 42).await.unwrap_err().to_string();
        assert!(err.contains("cannot approve"), "{err}");
        let err = client.unapprove(7, 42).await.unwrap_err().to_string();
        assert!(err.contains("HTTP 404"), "{err}");
    }

    #[tokio::test]
    async fn discussions_parse_diff_notes_and_plain_notes() {
        let server = MockServer::start().await;
        let body = format!("[{},{}]", include_str!("fixtures/discussions.json"), include_str!("fixtures/diff_note.json"));
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/7/merge_requests/42/discussions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body).insert_header("Content-Type", "application/json"))
            .mount(&server)
            .await;
        let discussions = client(&server).await.discussions(7, 42).await.unwrap();
        let expected: Discussion = from_fixture(include_str!("fixtures/diff_note.json"));
        assert_eq!(discussions.len(), 2);
        assert_eq!(discussions[1], expected);
    }
}
