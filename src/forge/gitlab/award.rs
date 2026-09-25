//! Reactions on GitLab notes: its award emoji, read for every note of an MR in one GraphQL query
//! and changed by the note's GraphQL id, so removing mine needs no award id.
use super::Client;
use crate::forge::{Emoji, MrKey, Reaction, tally};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;

const NOTES: &str = "query Awards($project: ID!, $iid: String!, $after: String) {
  project(fullPath: $project) {
    mergeRequest(iid: $iid) {
      notes(first: 100, after: $after) {
        pageInfo { hasNextPage endCursor }
        nodes { id awardEmoji(first: 50) { nodes { name user { username } } } }
      }
    }
  }
}";

/// More pages than a real MR has; a guard against a cursor that never ends.
const MAX_PAGES: usize = 20;

/// A note's GraphQL id and its reactions, by the note's REST id.
pub type Awards = HashMap<u64, (String, Vec<Reaction>)>;

#[derive(Deserialize)]
struct Answer<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Vec<Message>,
}

#[derive(Deserialize)]
struct Message {
    message: String,
}

fn data<T>(answer: Answer<T>) -> Result<T> {
    if !answer.errors.is_empty() {
        bail!("GraphQL: {}", answer.errors.iter().map(|e| e.message.as_str()).collect::<Vec<_>>().join("; "));
    }
    answer.data.context("GraphQL answered without data")
}

#[derive(Deserialize)]
struct NotesData {
    project: Option<Project>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Project {
    merge_request: Option<MergeRequest>,
}

#[derive(Deserialize)]
struct MergeRequest {
    notes: Notes,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Notes {
    page_info: PageInfo,
    nodes: Vec<NoteAwards>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NoteAwards {
    id: String,
    award_emoji: Nodes<Award>,
}

#[derive(Deserialize)]
struct Nodes<T> {
    nodes: Vec<T>,
}

#[derive(Deserialize)]
struct Award {
    name: String,
    user: Option<Username>,
}

#[derive(Deserialize)]
struct Username {
    username: String,
}

impl NoteAwards {
    /// `gid://gitlab/DiffNote/123` is REST note 123.
    fn rest_id(&self) -> Option<u64> {
        self.id.rsplit('/').next()?.parse().ok()
    }

    fn reactions(&self, me: &str) -> Vec<Reaction> {
        tally(self.award_emoji.nodes.iter().filter_map(|a| {
            let emoji = Emoji::from_gitlab(&a.name)?;
            Some((emoji, a.user.as_ref().is_some_and(|u| u.username == me)))
        }))
    }
}

impl Client {
    /// Every note's reactions and GraphQL id, across pages.
    pub async fn awards(&self, key: &MrKey) -> Result<Awards> {
        let me = self.my_name().await?;
        let mut awards = Awards::new();
        let mut after: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let variables = json!({"project": key.project, "iid": key.number.to_string(), "after": after});
            let answer: Answer<NotesData> = self.post_json("graphql", &json!({"query": NOTES, "variables": variables})).await?;
            let notes = data(answer)?
                .project
                .and_then(|p| p.merge_request)
                .with_context(|| format!("{}!{} not found", key.project, key.number))?
                .notes;
            for note in &notes.nodes {
                if let Some(id) = note.rest_id() {
                    awards.insert(id, (note.id.clone(), note.reactions(&me)));
                }
            }
            match notes.page_info {
                PageInfo { has_next_page: true, end_cursor: Some(cursor) } => after = Some(cursor),
                _ => break,
            }
        }
        Ok(awards)
    }

    /// Adds my `emoji` to the note `node`, or takes it off.
    pub async fn react(&self, node: &str, emoji: Emoji, on: bool) -> Result<()> {
        let mutation = if on { "awardEmojiAdd" } else { "awardEmojiRemove" };
        let query =
            format!("mutation($id: AwardableID!, $name: String!) {{ {mutation}(input: {{awardableId: $id, name: $name}}) {{ errors }} }}");
        let answer: Answer<serde_json::Value> =
            self.post_json("graphql", &json!({"query": query, "variables": {"id": node, "name": emoji.gitlab()}})).await?;
        let errors = data(answer)?[mutation]["errors"].as_array().cloned().unwrap_or_default();
        if let Some(first) = errors.first() {
            bail!("GitLab refused the reaction: {}", first.as_str().unwrap_or("unknown error"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::auth::Credentials;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(server: &MockServer) -> Client {
        Client::with_base(&Credentials { host: "gitlab.com".into(), token: "glpat-xxxx".into() }, &format!("{}/api/v4/", server.uri()))
            .unwrap()
    }

    async fn me(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/api/v4/user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 9, "username": "nina", "name": "Nina"})))
            .mount(server)
            .await;
    }

    fn page(nodes: &serde_json::Value, next: Option<&str>) -> serde_json::Value {
        json!({"data": {"project": {"mergeRequest": {"notes": {
            "pageInfo": {"hasNextPage": next.is_some(), "endCursor": next},
            "nodes": nodes
        }}}}})
    }

    #[tokio::test]
    async fn awards_follow_every_page_and_tell_mine_apart() {
        let server = MockServer::start().await;
        me(&server).await;
        let first = json!([{"id": "gid://gitlab/DiffNote/11", "awardEmoji": {"nodes": [
            {"name": "thumbsup", "user": {"username": "nina"}},
            {"name": "thumbsup", "user": {"username": "lea"}},
            {"name": "100", "user": {"username": "lea"}},
            {"name": "my_team_logo", "user": {"username": "lea"}}
        ]}}]);
        let second = json!([{"id": "gid://gitlab/Note/12", "awardEmoji": {"nodes": [{"name": "tada", "user": {"username": "lea"}}]}}]);
        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .and(body_string_contains("\"after\":\"c1\""))
            .respond_with(ResponseTemplate::new(200).set_body_json(page(&second, None)))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page(&first, Some("c1"))))
            .mount(&server)
            .await;
        let awards = client(&server).awards(&MrKey::new("acme/widgets", 42)).await.unwrap();
        let hundred = Emoji::from_gitlab("100").unwrap();
        let expected = vec![Reaction { emoji: Emoji::ThumbsUp, count: 2, mine: true }, Reaction { emoji: hundred, count: 1, mine: false }];
        assert_eq!(awards[&11], ("gid://gitlab/DiffNote/11".into(), expected), "any emoji but a custom one");
        assert_eq!(awards[&12].1, vec![Reaction { emoji: Emoji::Hooray, count: 1, mine: false }]);
    }

    #[tokio::test]
    async fn react_adds_or_removes_by_the_notes_graphql_id_and_says_why_it_failed() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .and(body_string_contains("awardEmojiAdd"))
            .and(body_string_contains("gid://gitlab/DiffNote/11"))
            .and(body_string_contains("\"name\":\"rocket\""))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": {"awardEmojiAdd": {"errors": []}}})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .and(body_string_contains("awardEmojiRemove"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": {"awardEmojiRemove": {"errors": ["Not allowed"]}}})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .and(body_string_contains("awardEmojiAdd"))
            .and(body_string_contains("\"name\":\"100\""))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": {"awardEmojiAdd": {"errors": []}}})))
            .mount(&server)
            .await;
        let client = client(&server);
        client.react("gid://gitlab/DiffNote/11", Emoji::Rocket, true).await.unwrap();
        client.react("gid://gitlab/DiffNote/11", Emoji::from_gitlab("100").unwrap(), true).await.unwrap();
        let err = client.react("gid://gitlab/DiffNote/11", Emoji::Rocket, false).await.unwrap_err().to_string();
        assert!(err.contains("Not allowed"), "{err}");
    }
}
