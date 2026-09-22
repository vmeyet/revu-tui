use super::Client;
use super::types::{Approvals, DiffFile, Discussion, Mr};
use anyhow::Result;

fn mr_path(project_id: u64, iid: u64) -> String {
    format!("projects/{project_id}/merge_requests/{iid}")
}

impl Client {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::from_fixture;
    use crate::auth::Credentials;
    use serde_json::json;
    use wiremock::matchers::{method, path, query_param};
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
