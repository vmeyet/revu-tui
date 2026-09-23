//! GitHub's shapes, as REST and GraphQL send them, and their turn into the neutral model.
use crate::forge::{self, LineRef, Position, QueueMr, Refs, ReviewState, ReviewerState, Side};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::Deserialize;

/// A GraphQL answer: `data`, or the `errors` that stopped it.
#[derive(Deserialize)]
pub struct Answer<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Option<Vec<Message>>,
}

#[derive(Deserialize)]
struct Message {
    message: String,
}

impl<T> Answer<T> {
    pub fn into_data(self) -> Result<T> {
        if let Some(errors) = self.errors.filter(|e| !e.is_empty()) {
            bail!("GraphQL: {}", errors.iter().map(|e| e.message.as_str()).collect::<Vec<_>>().join("; "));
        }
        self.data.context("GraphQL answered without data")
    }
}

/// GitHub errors are `{"message": …, "errors": [{"message": …}]}`; anything else is shown as is.
pub fn error_message(body: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else { return body.trim().to_owned() };
    let head = value.get("message").and_then(serde_json::Value::as_str).unwrap_or_default();
    let details: Vec<&str> = value
        .get("errors")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|e| e.get("message").and_then(serde_json::Value::as_str).or_else(|| e.as_str()))
        .collect();
    match (head, details.is_empty()) {
        ("", true) => body.trim().to_owned(),
        (head, true) => head.to_owned(),
        (head, false) => format!("{head}: {}", details.join("; ")),
    }
}

/// A REST user: `/user`, a PR author.
#[derive(Deserialize)]
pub struct RestUser {
    pub id: u64,
    pub login: String,
    #[serde(default)]
    pub name: Option<String>,
}

impl From<RestUser> for forge::User {
    fn from(u: RestUser) -> Self {
        let name = u.name.filter(|n| !n.is_empty()).unwrap_or_else(|| u.login.clone());
        Self { id: u.id, username: u.login, name }
    }
}

/// A GraphQL actor: a user (with a name and an id) or a bot (without); `None` once deleted.
#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Actor {
    pub login: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub database_id: Option<u64>,
}

pub fn user_of(actor: Option<Actor>) -> forge::User {
    let actor = actor.unwrap_or_else(|| Actor { login: "ghost".into(), ..Actor::default() });
    let name = actor.name.filter(|n| !n.is_empty()).unwrap_or_else(|| actor.login.clone());
    forge::User { id: actor.database_id.unwrap_or_default(), username: actor.login, name }
}

#[derive(Deserialize)]
pub struct Nodes<T> {
    pub nodes: Vec<T>,
}

impl<T> Default for Nodes<T> {
    fn default() -> Self {
        Self { nodes: vec![] }
    }
}

#[derive(Deserialize)]
pub struct Count {
    #[serde(rename = "totalCount")]
    pub total: u32,
}

#[derive(Deserialize)]
pub struct Label {
    pub name: String,
}

#[derive(Deserialize)]
pub struct Review {
    #[serde(default)]
    pub author: Option<Actor>,
    pub state: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRequest {
    #[serde(default)]
    pub requested_reviewer: Option<Actor>,
}

#[derive(Deserialize)]
pub struct CommitNode {
    pub commit: Commit,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Commit {
    #[serde(default)]
    pub status_check_rollup: Option<Rollup>,
}

#[derive(Deserialize)]
pub struct Rollup {
    pub state: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedFlag {
    pub is_resolved: bool,
}

/// A pull request as the queue fragment asks for it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuePr {
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    pub is_draft: bool,
    pub url: String,
    pub updated_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub head_ref_name: String,
    pub base_ref_name: String,
    #[serde(default)]
    pub mergeable: Option<String>,
    pub repository: Repository,
    #[serde(default)]
    pub author: Option<Actor>,
    #[serde(default)]
    pub review_decision: Option<String>,
    #[serde(default)]
    pub latest_reviews: Nodes<Review>,
    #[serde(default)]
    pub review_requests: Nodes<ReviewRequest>,
    #[serde(default)]
    pub commits: Nodes<CommitNode>,
    pub additions: u32,
    pub deletions: u32,
    pub changed_files: u32,
    #[serde(default)]
    pub review_threads: Nodes<ResolvedFlag>,
    #[serde(default)]
    pub labels: Nodes<Label>,
    pub comments: Count,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repository {
    pub name_with_owner: String,
}

/// A review state as the queue sorts it: a pending request counts over any earlier review.
pub fn review_state(state: &str) -> ReviewState {
    match state {
        "APPROVED" => ReviewState::Approved,
        "CHANGES_REQUESTED" => ReviewState::RequestedChanges,
        "COMMENTED" => ReviewState::Reviewed,
        "DISMISSED" => ReviewState::Unapproved,
        "PENDING" => ReviewState::ReviewStarted,
        _ => ReviewState::Unreviewed,
    }
}

/// Every reviewer once: whoever is asked again reads as unreviewed, whatever they said before.
pub fn reviewers(requests: &Nodes<ReviewRequest>, reviews: &Nodes<Review>) -> Vec<ReviewerState> {
    let asked = requests.nodes.iter().filter_map(|r| r.requested_reviewer.as_ref()).map(|a| (a.login.clone(), ReviewState::Unreviewed));
    let answered = reviews.nodes.iter().filter_map(|r| r.author.as_ref().map(|a| (a.login.clone(), review_state(&r.state))));
    let mut states: Vec<ReviewerState> = vec![];
    for (username, state) in asked.chain(answered) {
        if !states.iter().any(|s| s.username == username) {
            states.push(ReviewerState { username, state });
        }
    }
    states
}

/// Check-run rollups in the spelling the queue badges read: `SUCCESS`, `FAILED`, `RUNNING`.
pub fn pipeline(commits: &Nodes<CommitNode>) -> Option<String> {
    let state = commits.nodes.last()?.commit.status_check_rollup.as_ref()?.state.as_str();
    Some(
        match state {
            "SUCCESS" => "SUCCESS",
            "FAILURE" | "ERROR" => "FAILED",
            _ => "RUNNING",
        }
        .to_owned(),
    )
}

impl From<QueuePr> for QueueMr {
    fn from(pr: QueuePr) -> Self {
        let author = user_of(pr.author);
        let approved_by: Vec<String> = pr
            .latest_reviews
            .nodes
            .iter()
            .filter(|r| r.state == "APPROVED")
            .filter_map(|r| r.author.as_ref().map(|a| a.login.clone()))
            .collect();
        Self {
            host: None,
            number: pr.number,
            project: pr.repository.name_with_owner,
            title: pr.title,
            description: pr.body.unwrap_or_default(),
            draft: pr.is_draft,
            web_url: pr.url,
            updated_at: pr.updated_at,
            created_at: pr.created_at,
            source_branch: pr.head_ref_name,
            target_branch: pr.base_ref_name,
            conflicts: pr.mergeable.as_deref() == Some("CONFLICTING"),
            author: author.username,
            author_name: author.name,
            approved: pr.review_decision.as_deref() == Some("APPROVED") || (pr.review_decision.is_none() && !approved_by.is_empty()),
            approved_by,
            reviewers: reviewers(&pr.review_requests, &pr.latest_reviews),
            pipeline: pipeline(&pr.commits),
            additions: pr.additions,
            deletions: pr.deletions,
            files: pr.changed_files,
            unresolved: pr.review_threads.nodes.iter().filter(|t| !t.is_resolved).count() as u32,
            labels: pr.labels.nodes.into_iter().map(|l| l.name).collect(),
            notes: pr.comments.total,
        }
    }
}

/// A diff line on one side, as GitHub names it (`LEFT` is the old file, `RIGHT` the new one).
pub fn line_on(side: &str, line: u32) -> LineRef {
    if side == "LEFT" { LineRef { old: Some(line), new: None } } else { LineRef { old: None, new: Some(line) } }
}

/// The side and number GitHub wants for a neutral line: the new file whenever the line is there.
pub fn side_of(line: LineRef) -> Option<(&'static str, u32)> {
    let number = line.number()?;
    Some((if line.side() == Side::New { "RIGHT" } else { "LEFT" }, number))
}

/// A review thread's anchor, on `refs`; outdated threads fall back to where they were written.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Anchor {
    pub path: String,
    #[serde(default)]
    pub line: Option<u32>,
    #[serde(default)]
    pub original_line: Option<u32>,
    #[serde(default)]
    pub start_line: Option<u32>,
    #[serde(default)]
    pub original_start_line: Option<u32>,
    #[serde(default)]
    pub diff_side: Option<String>,
    #[serde(default)]
    pub start_diff_side: Option<String>,
}

impl Anchor {
    pub fn position(&self, refs: &Refs) -> Option<Position> {
        let side = self.diff_side.as_deref().unwrap_or("RIGHT");
        let line = line_on(side, self.line.or(self.original_line)?);
        let start = self
            .start_line
            .or(self.original_start_line)
            .map(|n| line_on(self.start_diff_side.as_deref().unwrap_or(side), n))
            .filter(|start| *start != line);
        Some(Position { refs: refs.clone(), old_path: self.path.clone(), new_path: self.path.clone(), line, start })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn refs() -> Refs {
        Refs { base: "a".into(), start: "a".into(), head: "b".into() }
    }

    #[test]
    fn sides_map_both_ways() {
        let cases = [
            (LineRef { old: None, new: Some(14) }, ("RIGHT", 14)),
            (LineRef { old: Some(13), new: None }, ("LEFT", 13)),
            (LineRef { old: Some(12), new: Some(12) }, ("RIGHT", 12)),
        ];
        for (line, wire) in cases {
            assert_eq!(side_of(line), Some(wire), "{line:?}");
        }
        assert_eq!(line_on("LEFT", 3), LineRef { old: Some(3), new: None });
        assert_eq!(line_on("RIGHT", 3), LineRef { old: None, new: Some(3) });
    }

    #[test]
    fn anchors_become_positions_with_their_range_and_fall_back_when_outdated() {
        let one: Anchor = serde_json::from_value(serde_json::json!({"path": "src/a.rs", "line": 57, "diffSide": "RIGHT"})).unwrap();
        let position = one.position(&refs()).unwrap();
        assert_eq!((position.path(), position.line, position.start), ("src/a.rs", LineRef { old: None, new: Some(57) }, None));
        let range: Anchor = serde_json::from_value(
            serde_json::json!({"path": "src/a.rs", "line": 9, "startLine": 7, "diffSide": "LEFT", "startDiffSide": "LEFT"}),
        )
        .unwrap();
        assert_eq!(range.position(&refs()).unwrap().start, Some(LineRef { old: Some(7), new: None }));
        let outdated: Anchor = serde_json::from_value(serde_json::json!({"path": "x", "line": null, "originalLine": 4})).unwrap();
        assert_eq!(outdated.position(&refs()).unwrap().line.new, Some(4));
        let nowhere: Anchor = serde_json::from_value(serde_json::json!({"path": "x"})).unwrap();
        assert!(nowhere.position(&refs()).is_none());
    }

    #[test]
    fn a_new_request_outranks_an_earlier_review() {
        let requests: Nodes<ReviewRequest> =
            serde_json::from_value(serde_json::json!({"nodes": [{"requestedReviewer": {"login": "nina"}}, {"requestedReviewer": null}]}))
                .unwrap();
        let reviews: Nodes<Review> = serde_json::from_value(serde_json::json!({"nodes": [
            {"author": {"login": "nina"}, "state": "APPROVED"},
            {"author": {"login": "lea"}, "state": "CHANGES_REQUESTED"},
            {"author": {"login": "omar"}, "state": "COMMENTED"}
        ]}))
        .unwrap();
        let states: Vec<(String, ReviewState)> = reviewers(&requests, &reviews).into_iter().map(|r| (r.username, r.state)).collect();
        assert_eq!(
            states,
            [
                ("nina".into(), ReviewState::Unreviewed),
                ("lea".into(), ReviewState::RequestedChanges),
                ("omar".into(), ReviewState::Reviewed)
            ]
        );
    }

    #[test]
    fn rollups_read_like_pipelines() {
        let rollup = |state: &str| -> Nodes<CommitNode> {
            serde_json::from_value(serde_json::json!({"nodes": [{"commit": {"statusCheckRollup": {"state": state}}}]})).unwrap()
        };
        for (state, expected) in
            [("SUCCESS", "SUCCESS"), ("FAILURE", "FAILED"), ("ERROR", "FAILED"), ("PENDING", "RUNNING"), ("EXPECTED", "RUNNING")]
        {
            assert_eq!(pipeline(&rollup(state)).as_deref(), Some(expected), "{state}");
        }
        let none: Nodes<CommitNode> =
            serde_json::from_value(serde_json::json!({"nodes": [{"commit": {"statusCheckRollup": null}}]})).unwrap();
        assert_eq!(pipeline(&none), None);
    }

    #[test]
    fn error_messages_join_the_details() {
        assert_eq!(
            error_message(r#"{"message":"Validation Failed","errors":[{"message":"line must be part of the diff"}]}"#),
            "Validation Failed: line must be part of the diff"
        );
        assert_eq!(error_message(r#"{"message":"Not Found"}"#), "Not Found");
        assert_eq!(error_message("<html>"), "<html>");
    }
}
