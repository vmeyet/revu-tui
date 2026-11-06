//! My mentions on GitLab: the to-do items it keeps when someone mentions me or writes to me on an MR.
use super::Client;
use super::wire::User;
use crate::forge::http;
use crate::forge::{Mention, MentionId, ProjectPath, QueueMr};
use anyhow::Result;
use chrono::{DateTime, Utc};
use reqwest::{Method, StatusCode};
use serde::Deserialize;

#[derive(Deserialize)]
struct Todo {
    id: u64,
    project: Project,
    target: Target,
    /// The MR's page, ending in `#note_<id>` when a note made it.
    target_url: String,
}

#[derive(Deserialize)]
struct Project {
    path_with_namespace: ProjectPath,
}

/// The MR as a to-do item carries it.
#[derive(Deserialize)]
struct Target {
    iid: u64,
    title: String,
    #[serde(default)]
    description: Option<String>,
    state: String,
    #[serde(default)]
    draft: bool,
    author: User,
    source_branch: String,
    target_branch: String,
    web_url: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    #[serde(default)]
    has_conflicts: bool,
    #[serde(default)]
    labels: Vec<String>,
}

impl Todo {
    fn into_mention(self) -> Mention {
        let note = note_of(&self.target_url);
        let Target {
            iid,
            title,
            description,
            state,
            draft,
            author,
            source_branch,
            target_branch,
            web_url,
            created_at,
            updated_at,
            has_conflicts,
            labels,
        } = self.target;
        let state = super::wire::mr_state(&state);
        let mr = QueueMr {
            host: None,
            number: iid,
            project: self.project.path_with_namespace,
            title,
            description: description.unwrap_or_default(),
            draft,
            web_url,
            updated_at,
            created_at,
            source_branch,
            target_branch,
            conflicts: has_conflicts,
            author: author.username,
            author_name: author.name,
            approved: false,
            approved_by: vec![],
            approvals_left: None,
            reviewers: vec![],
            pipeline: None,
            size: None,
            unresolved: 0,
            labels,
            notes: 0,
            commenters: vec![],
            reason: None,
            worktree: None,
        };
        Mention { id: MentionId(self.id), mr, state, note }
    }
}

/// `…/merge_requests/42#note_123` is note 123.
fn note_of(url: &str) -> Option<u64> {
    url.rsplit_once("#note_")?.1.parse().ok()
}

impl Client {
    /// My pending mentions on MRs, newest first. Only the two kinds a person makes by writing to me:
    /// the others (assigned, review asked, pipeline failed) the queue already shows.
    pub(crate) async fn mentions(&self) -> Result<Vec<Mention>> {
        let (mentioned, addressed) = tokio::try_join!(self.pending("mentioned"), self.pending("directly_addressed"))?;
        let mut mentions: Vec<Mention> = mentioned.into_iter().chain(addressed).map(Todo::into_mention).collect();
        mentions.sort_by_key(|mention| std::cmp::Reverse(mention.id.0));
        Ok(mentions)
    }

    async fn pending(&self, action: &str) -> Result<Vec<Todo>> {
        self.http.get(&format!("todos?state=pending&type=MergeRequest&action={action}&per_page=100")).await
    }

    /// GitLab answers 404 for a to-do item that is gone: as done as it gets.
    pub(crate) async fn mark_done(&self, ids: &[MentionId]) -> Result<()> {
        for id in ids {
            match self.http.send_empty(Method::POST, &format!("todos/{}/mark_as_done", id.0), None).await {
                Err(e) if http::status(&e) == Some(StatusCode::NOT_FOUND) => {}
                outcome => outcome?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Credentials;
    use crate::forge::MrState;
    use crate::forge::gitlab::fixture::todo;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(server: &MockServer) -> Client {
        let creds = Credentials { host: "gitlab.com".into(), token: "glpat-xxxx".into() };
        Client::with_base(&creds, &format!("{}/api/v4/", server.uri())).unwrap()
    }

    async fn answer(server: &MockServer, action: &str, todos: serde_json::Value) {
        Mock::given(method("GET"))
            .and(path("/api/v4/todos"))
            .and(query_param("state", "pending"))
            .and(query_param("type", "MergeRequest"))
            .and(query_param("action", action))
            .respond_with(ResponseTemplate::new(200).set_body_json(todos))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn mentions_ask_both_kinds_and_read_the_note_from_the_link() {
        let server = MockServer::start().await;
        answer(&server, "mentioned", serde_json::json!([todo(11, "acme/widgets", 42, "opened", "#note_900")])).await;
        answer(&server, "directly_addressed", serde_json::json!([todo(12, "acme/widgets", 43, "merged", "")])).await;
        let mentions = client(&server).mentions().await.unwrap();
        let read: Vec<(u64, u64, MrState, Option<u64>)> = mentions.iter().map(|m| (m.id.0, m.mr.number, m.state, m.note)).collect();
        assert_eq!(read, [(12, 43, MrState::Merged, None), (11, 42, MrState::Open, Some(900))], "newest first");
        assert_eq!((mentions[1].mr.author.as_str(), mentions[1].mr.labels.as_slice()), ("omar", ["backend".to_owned()].as_slice()));
    }

    #[tokio::test]
    async fn marking_done_posts_each_one_and_takes_a_gone_one_as_done() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v4/todos/11/mark_as_done"))
            .respond_with(ResponseTemplate::new(201).set_body_json(todo(11, "acme/widgets", 42, "opened", "")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v4/todos/12/mark_as_done"))
            .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({"message": "404 Not found"})))
            .mount(&server)
            .await;
        client(&server).mark_done(&[MentionId(11), MentionId(12)]).await.unwrap();
    }

    #[tokio::test]
    async fn a_refused_mark_is_an_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v4/todos/11/mark_as_done"))
            .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({"message": "403 Forbidden"})))
            .mount(&server)
            .await;
        assert!(client(&server).mark_done(&[MentionId(11)]).await.is_err());
    }

    #[test]
    fn a_link_without_a_note_points_at_the_mr() {
        assert_eq!(note_of("https://gitlab.com/acme/widgets/-/merge_requests/42#note_77"), Some(77));
        assert_eq!(note_of("https://gitlab.com/acme/widgets/-/merge_requests/42"), None);
        assert_eq!(note_of("https://gitlab.com/acme/widgets/-/merge_requests/42#note_x"), None);
    }
}
