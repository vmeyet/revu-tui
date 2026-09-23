//! The queue in two calls, three when scoped to a project, all in parallel. Wire shapes stay
//! private; callers get `Queue` and its `Sections`.
//! GitLab refuses a query scoring over 250. With the fields the "needs me" rules read, the MRs
//! asking me score 185, mine 87 (the rules skip them, so they skip the fields) and a project's
//! open MRs 124: each call stays under the limit, one query for all would not.
use super::Client;
use crate::forge::{Queue, QueueMr, ReviewState, ReviewerState};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;

const ASKING: &str = r"
  currentUser {
    username
    reviewRequested: reviewRequestedMergeRequests(state: opened, first: 50, sort: UPDATED_DESC) { nodes { FIELDS RULES } }
    assigned: assignedMergeRequests(state: opened, first: 50, sort: UPDATED_DESC) { nodes { FIELDS RULES } }
  }";

const AUTHORED: &str = r"
  currentUser {
    username
    authored: authoredMergeRequests(state: opened, first: 50, sort: UPDATED_DESC) { nodes { FIELDS } }
  }";

const PROJECT: &str = r"
  project(fullPath: $project) {
    mergeRequests(state: opened, first: 100, sort: UPDATED_DESC) { nodes { FIELDS RULES } }
  }";

const FIELDS: &str = "id iid title description draft webUrl updatedAt createdAt
    sourceBranch targetBranch conflicts
    project { id fullPath }
    author { username name avatarUrl }
    approved approvedBy { nodes { username } }
    reviewers { nodes { username mergeRequestInteraction { reviewState } } }
    headPipeline { status detailedStatus { label } }
    diffStatsSummary { additions deletions fileCount }
    resolvableDiscussionsCount resolvedDiscussionsCount
    labels { nodes { title } }
    userNotesCount";

/// What only the "needs me" rules read: approvals still missing, and who commented.
const RULES: &str = "approvalsLeft commenters { nodes { username } }";

fn filled(body: &str) -> String {
    body.replace("FIELDS", FIELDS).replace("RULES", RULES)
}

fn asking_query() -> String {
    format!("query Asking {{{}\n}}", filled(ASKING))
}

fn authored_query() -> String {
    format!("query Authored {{{}\n}}", filled(AUTHORED))
}

fn project_query() -> String {
    format!("query Open($project: ID!) {{{}\n}}", filled(PROJECT))
}

impl Client {
    /// Every MR waiting on me; with `project`, only that project's, plus all its other open MRs.
    pub async fn queue(&self, project: Option<&str>) -> Result<Queue> {
        let (asking_body, authored_body) = (json!({"query": asking_query()}), json!({"query": authored_query()}));
        let asking = self.post_json::<Answer>("graphql", &asking_body);
        let authored = self.post_json::<Answer>("graphql", &authored_body);
        let Some(path) = project else {
            let (asking, authored) = tokio::try_join!(asking, authored)?;
            return queue_from(asking, authored, None);
        };
        let open_body = json!({"query": project_query(), "variables": {"project": path}});
        let (asking, authored, open) = tokio::try_join!(asking, authored, self.post_json::<Answer>("graphql", &open_body))?;
        queue_from(asking, authored, Some((open, path)))
    }
}

/// `open` is the project answer and the path it was asked for, when the queue is scoped.
fn queue_from(asking: Answer, authored: Answer, open: Option<(Answer, &str)>) -> Result<Queue> {
    let user = data_of(asking)?.current_user.context("GraphQL answered without currentUser")?;
    let mine = data_of(authored)?.current_user.context("GraphQL answered without currentUser")?;
    let (project, open) = match open {
        Some((answer, path)) => {
            let found = data_of(answer)?.project.with_context(|| format!("project {path} not found, or not visible with this token"))?;
            (Some(path.to_owned()), convert(found.merge_requests)?)
        }
        None => (None, vec![]),
    };
    Ok(Queue {
        me: user.username,
        project,
        review_requested: convert(user.review_requested)?,
        authored: convert(mine.authored)?,
        assigned: convert(user.assigned)?,
        open,
    })
}

/// A queue straight from a GraphQL answer body; the one body stands for every call's answer.
#[cfg(test)]
pub(super) fn queue_from_json(body: &str, project: Option<&str>) -> Result<Queue> {
    let open = match project {
        Some(path) => Some((serde_json::from_str(body)?, path)),
        None => None,
    };
    queue_from(serde_json::from_str(body)?, serde_json::from_str(body)?, open)
}

fn data_of(answer: Answer) -> Result<Data> {
    if let Some(errors) = answer.errors.filter(|e| !e.is_empty()) {
        bail!("GraphQL: {}", errors.iter().map(|e| e.message.as_str()).collect::<Vec<_>>().join("; "));
    }
    answer.data.context("GraphQL answered without data")
}

fn convert(connection: Connection<WireMr>) -> Result<Vec<QueueMr>> {
    connection.nodes.into_iter().map(QueueMr::try_from).collect()
}

impl TryFrom<WireMr> for QueueMr {
    type Error = anyhow::Error;

    fn try_from(w: WireMr) -> Result<Self> {
        let stats = w.diff_stats_summary.unwrap_or_default();
        Ok(Self {
            host: None,
            number: w.iid.parse().with_context(|| format!("iid {:?}", w.iid))?,
            project: w.project.full_path,
            title: w.title,
            description: w.description.unwrap_or_default(),
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
            approvals_left: w.approvals_left,
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
            commenters: w.commenters.map(|c| c.nodes.into_iter().map(|u| u.username).collect()).unwrap_or_default(),
            reason: None,
        })
    }
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
    project: Option<WireProjectMrs>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireProjectMrs {
    merge_requests: Connection<WireMr>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireUser {
    #[serde(default)]
    review_requested: Connection<WireMr>,
    #[serde(default)]
    authored: Connection<WireMr>,
    #[serde(default)]
    assigned: Connection<WireMr>,
    username: String,
}

#[derive(Deserialize)]
struct Connection<T> {
    nodes: Vec<T>,
}

impl<T> Default for Connection<T> {
    fn default() -> Self {
        Self { nodes: vec![] }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireMr {
    iid: String,
    title: String,
    description: Option<String>,
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
    #[serde(default)]
    approvals_left: Option<u32>,
    #[serde(default)]
    commenters: Option<Connection<WireUsername>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireProject {
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
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::auth::Credentials;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const FIXTURE: &str = include_str!("fixtures/queue.json");

    fn queue() -> Queue {
        queue_from_json(FIXTURE, None).unwrap()
    }

    fn iids(mrs: &[QueueMr]) -> Vec<u64> {
        mrs.iter().map(|m| m.number).collect()
    }

    #[tokio::test]
    async fn queue_posts_the_query_and_parses_the_answer() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .and(body_partial_json(json!({"query": asking_query()})))
            .respond_with(ResponseTemplate::new(200).set_body_string(FIXTURE))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .and(body_partial_json(json!({"query": authored_query()})))
            .respond_with(ResponseTemplate::new(200).set_body_string(FIXTURE))
            .mount(&server)
            .await;
        let client =
            Client::with_base(&Credentials { host: "x".into(), token: "glpat-xxxx".into() }, &format!("{}/api/v4/", server.uri())).unwrap();
        let queue = client.queue(None).await.unwrap();
        assert_eq!(queue.me, "nina");
        assert_eq!(iids(&queue.review_requested), [42, 40]);
        let first = &queue.review_requested[0];
        assert_eq!(first.key(), crate::forge::MrKey::new("acme/widgets", 42));
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
        let err = client.queue(None).await.unwrap_err().to_string();
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

    const SCOPED: &str = include_str!("fixtures/queue_scoped.json");

    #[tokio::test]
    async fn a_scoped_queue_sends_the_project_and_reads_its_open_mrs() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .and(body_partial_json(json!({"query": project_query(), "variables": {"project": "acme/widgets"}})))
            .respond_with(ResponseTemplate::new(200).set_body_string(SCOPED))
            .expect(1)
            .mount(&server)
            .await;
        for query in [asking_query(), authored_query()] {
            Mock::given(method("POST"))
                .and(path("/api/graphql"))
                .and(body_partial_json(json!({"query": query})))
                .respond_with(ResponseTemplate::new(200).set_body_string(FIXTURE))
                .expect(1)
                .mount(&server)
                .await;
        }
        let client =
            Client::with_base(&Credentials { host: "x".into(), token: "glpat-xxxx".into() }, &format!("{}/api/v4/", server.uri())).unwrap();
        let queue = client.queue(Some("acme/widgets")).await.unwrap();
        assert_eq!(queue.project.as_deref(), Some("acme/widgets"));
        assert_eq!(iids(&queue.open), [42, 50, 51]);
        assert_eq!(queue.open[1].description, "Adds the refund flow.");
    }

    #[test]
    fn a_scoped_queue_keeps_its_project_and_lists_the_rest_as_open() {
        let sections = queue_from_json(SCOPED, Some("acme/widgets")).unwrap().sections(&[]);
        assert_eq!(iids(&sections.to_review), [42]);
        assert_eq!(iids(&sections.mine), [41]);
        assert_eq!(iids(&sections.done), [40]);
        assert_eq!(iids(&sections.watching), [] as [u64; 0], "the assigned MR lives in another project");
        assert_eq!(iids(&sections.open), [51], "42 is already in To review");
        assert_eq!(iids(&sections.drafts), [50], "someone else's draft waits apart");
        assert_eq!(iids(&sections.mine), [41], "my own draft stays with mine");
    }

    #[test]
    fn an_unknown_project_is_an_error_not_an_empty_queue() {
        let body = FIXTURE.replacen("\"data\": {", "\"data\": {\"project\": null, ", 1);
        let err = queue_from_json(&body, Some("acme/gone")).unwrap_err().to_string();
        assert!(err.contains("acme/gone"), "{err}");
    }
}
