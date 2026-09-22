//! The queue in one call. Wire shapes stay private; callers get `Queue` and its `Sections`.
use super::Client;
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;

const QUERY: &str = r"
query Queue {
  currentUser {
    username
    reviewRequested: reviewRequestedMergeRequests(state: opened, first: 50, sort: UPDATED_DESC) { ...list }
    authored: authoredMergeRequests(state: opened, first: 50, sort: UPDATED_DESC) { ...list }
    assigned: assignedMergeRequests(state: opened, first: 50, sort: UPDATED_DESC) { ...list }
  }
}
fragment list on MergeRequestConnection {
  nodes {
    id iid title draft webUrl updatedAt createdAt
    sourceBranch targetBranch conflicts
    project { id fullPath }
    author { username name avatarUrl }
    approved approvedBy { nodes { username } }
    reviewers { nodes { username mergeRequestInteraction { reviewState } } }
    headPipeline { status detailedStatus { label } }
    diffStatsSummary { additions deletions fileCount }
    resolvableDiscussionsCount resolvedDiscussionsCount
    labels { nodes { title } }
    userNotesCount
  }
}";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Queue {
    pub me: String,
    pub review_requested: Vec<QueueMr>,
    pub authored: Vec<QueueMr>,
    pub assigned: Vec<QueueMr>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueMr {
    pub id: u64,
    pub iid: u64,
    pub project_id: u64,
    pub project: String,
    pub title: String,
    pub draft: bool,
    pub web_url: String,
    pub updated_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub source_branch: String,
    pub target_branch: String,
    pub conflicts: bool,
    pub author: String,
    pub author_name: String,
    pub approved: bool,
    pub approved_by: Vec<String>,
    pub reviewers: Vec<ReviewerState>,
    /// GraphQL status: `SUCCESS`, `FAILED`, `RUNNING`, `PENDING`, `CANCELED`, `SKIPPED`, `MANUAL`…
    pub pipeline: Option<String>,
    pub additions: u32,
    pub deletions: u32,
    pub files: u32,
    pub unresolved: u32,
    pub labels: Vec<String>,
    pub notes: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewerState {
    pub username: String,
    pub state: ReviewState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewState {
    Unreviewed,
    Reviewed,
    RequestedChanges,
    Approved,
    ReviewStarted,
    Unapproved,
}

/// The queue sorted into the sidebar sections, each MR in exactly one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sections {
    pub to_review: Vec<QueueMr>,
    pub mine: Vec<QueueMr>,
    pub watching: Vec<QueueMr>,
    pub done: Vec<QueueMr>,
}

impl QueueMr {
    pub fn my_state(&self, me: &str) -> Option<ReviewState> {
        self.reviewers.iter().find(|r| r.username == me).map(|r| r.state)
    }

    fn reviewed_by(&self, me: &str) -> bool {
        matches!(self.my_state(me), Some(ReviewState::Approved | ReviewState::Reviewed))
    }

    fn key(&self) -> (u64, u64) {
        (self.project_id, self.iid)
    }
}

impl Queue {
    pub fn sections(&self, watch_labels: &[String]) -> Sections {
        let me = self.me.as_str();
        let (done, to_review): (Vec<_>, Vec<_>) = self.review_requested.iter().cloned().partition(|mr| mr.reviewed_by(me));
        let mine = self.authored.clone();
        let placed: HashSet<(u64, u64)> = to_review.iter().chain(&mine).chain(&done).map(QueueMr::key).collect();
        let labelled = self.review_requested.iter().chain(&self.authored).filter(|mr| mr.labels.iter().any(|l| watch_labels.contains(l)));
        let mut seen = placed.clone();
        let watching = self.assigned.iter().chain(labelled).filter(|mr| seen.insert(mr.key())).cloned().collect();
        Sections { to_review, mine, watching, done }
    }
}

impl Client {
    pub async fn queue(&self) -> Result<Queue> {
        let answer: Answer = self.post_json("graphql", &json!({"query": QUERY})).await?;
        if let Some(errors) = answer.errors.filter(|e| !e.is_empty()) {
            bail!("GraphQL: {}", errors.iter().map(|e| e.message.as_str()).collect::<Vec<_>>().join("; "));
        }
        let user = answer.data.and_then(|d| d.current_user).context("GraphQL answered without currentUser")?;
        Ok(Queue {
            me: user.username,
            review_requested: convert(user.review_requested)?,
            authored: convert(user.authored)?,
            assigned: convert(user.assigned)?,
        })
    }
}

fn convert(connection: Connection<WireMr>) -> Result<Vec<QueueMr>> {
    connection.nodes.into_iter().map(QueueMr::try_from).collect()
}

impl TryFrom<WireMr> for QueueMr {
    type Error = anyhow::Error;

    fn try_from(w: WireMr) -> Result<Self> {
        let stats = w.diff_stats_summary.unwrap_or_default();
        Ok(Self {
            id: gid(&w.id)?,
            iid: w.iid.parse().with_context(|| format!("iid {:?}", w.iid))?,
            project_id: gid(&w.project.id)?,
            project: w.project.full_path,
            title: w.title,
            draft: w.draft,
            web_url: w.web_url,
            updated_at: w.updated_at,
            created_at: w.created_at,
            source_branch: w.source_branch,
            target_branch: w.target_branch,
            conflicts: w.conflicts,
            author: w.author.username,
            author_name: w.author.name,
            approved: w.approved,
            approved_by: w.approved_by.nodes.into_iter().map(|u| u.username).collect(),
            reviewers: w
                .reviewers
                .nodes
                .into_iter()
                .map(|r| ReviewerState { username: r.username, state: r.merge_request_interaction.review_state })
                .collect(),
            pipeline: w.head_pipeline.map(|p| p.status),
            additions: stats.additions,
            deletions: stats.deletions,
            files: stats.file_count,
            unresolved: w.resolvable_discussions_count.saturating_sub(w.resolved_discussions_count),
            labels: w.labels.nodes.into_iter().map(|l| l.title).collect(),
            notes: w.user_notes_count,
        })
    }
}

/// `gid://gitlab/MergeRequest/1042` → 1042
fn gid(id: &str) -> Result<u64> {
    id.rsplit('/').next().and_then(|n| n.parse().ok()).with_context(|| format!("not a global id: {id:?}"))
}

#[derive(Deserialize)]
struct Answer {
    data: Option<Data>,
    errors: Option<Vec<GraphqlError>>,
}

#[derive(Deserialize)]
struct GraphqlError {
    message: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Data {
    current_user: Option<WireUser>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireUser {
    username: String,
    review_requested: Connection<WireMr>,
    authored: Connection<WireMr>,
    assigned: Connection<WireMr>,
}

#[derive(Deserialize)]
struct Connection<T> {
    nodes: Vec<T>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireMr {
    id: String,
    iid: String,
    title: String,
    draft: bool,
    web_url: String,
    updated_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
    source_branch: String,
    target_branch: String,
    conflicts: bool,
    project: WireProject,
    author: WireAuthor,
    approved: bool,
    approved_by: Connection<WireUsername>,
    reviewers: Connection<WireReviewer>,
    head_pipeline: Option<WirePipeline>,
    diff_stats_summary: Option<WireStats>,
    resolvable_discussions_count: u32,
    resolved_discussions_count: u32,
    labels: Connection<WireLabel>,
    user_notes_count: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireProject {
    id: String,
    full_path: String,
}

#[derive(Deserialize)]
struct WireAuthor {
    username: String,
    name: String,
}

#[derive(Deserialize)]
struct WireUsername {
    username: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireReviewer {
    username: String,
    merge_request_interaction: WireInteraction,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireInteraction {
    review_state: ReviewState,
}

#[derive(Deserialize)]
struct WirePipeline {
    status: String,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireStats {
    additions: u32,
    deletions: u32,
    file_count: u32,
}

#[derive(Deserialize)]
struct WireLabel {
    title: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Credentials;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const FIXTURE: &str = include_str!("fixtures/queue.json");

    fn queue() -> Queue {
        let answer: Answer = serde_json::from_str(FIXTURE).unwrap();
        let user = answer.data.unwrap().current_user.unwrap();
        Queue {
            me: user.username,
            review_requested: convert(user.review_requested).unwrap(),
            authored: convert(user.authored).unwrap(),
            assigned: convert(user.assigned).unwrap(),
        }
    }

    fn iids(mrs: &[QueueMr]) -> Vec<u64> {
        mrs.iter().map(|m| m.iid).collect()
    }

    #[tokio::test]
    async fn queue_posts_the_query_and_parses_the_answer() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .and(body_partial_json(json!({"query": QUERY})))
            .respond_with(ResponseTemplate::new(200).set_body_string(FIXTURE))
            .mount(&server)
            .await;
        let client =
            Client::with_base(&Credentials { host: "x".into(), token: "glpat-xxxx".into() }, &format!("{}/api/v4/", server.uri())).unwrap();
        let queue = client.queue().await.unwrap();
        assert_eq!(queue.me, "nina");
        assert_eq!(iids(&queue.review_requested), [42, 40]);
        let first = &queue.review_requested[0];
        assert_eq!((first.id, first.project_id, first.project.as_str()), (1042, 7, "acme/widgets"));
        assert_eq!(first.my_state("nina"), Some(ReviewState::Unreviewed));
        assert_eq!((first.additions, first.deletions, first.files, first.unresolved), (412, 38, 9, 1));
        assert_eq!(first.pipeline.as_deref(), Some("SUCCESS"));
        assert_eq!(queue.assigned[0].files, 0, "missing stats read as zero");
    }

    #[tokio::test]
    async fn graphql_errors_surface_as_messages() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"errors": [{"message": "Field x doesn't exist"}]})))
            .mount(&server)
            .await;
        let client =
            Client::with_base(&Credentials { host: "x".into(), token: "glpat-xxxx".into() }, &format!("{}/api/v4/", server.uri())).unwrap();
        let err = client.queue().await.unwrap_err().to_string();
        assert!(err.contains("doesn't exist"), "{err}");
    }

    #[test]
    fn sections_follow_the_spec_table() {
        let sections = queue().sections(&[]);
        assert_eq!(iids(&sections.to_review), [42], "unreviewed request");
        assert_eq!(iids(&sections.done), [40], "already approved by me");
        assert_eq!(iids(&sections.mine), [41]);
        assert_eq!(iids(&sections.watching), [35], "assigned, and my own MR is not repeated");
    }

    #[test]
    fn watch_labels_pull_in_mrs_not_placed_elsewhere() {
        let mut q = queue();
        q.review_requested.clear();
        let sections = q.sections(&["payments".into(), "infra".into()]);
        assert_eq!(iids(&sections.watching), [35], "labelled MRs come from the lists the query returned");
        let sections = queue().sections(&["payments".into()]);
        assert_eq!(iids(&sections.to_review), [42], "a label never moves an MR out of To review");
        assert_eq!(iids(&sections.watching), [35]);
    }

    #[test]
    fn global_ids_parse_and_garbage_does_not() {
        assert_eq!(gid("gid://gitlab/MergeRequest/1042").unwrap(), 1042);
        assert!(gid("gid://gitlab/MergeRequest/").is_err());
    }
}
