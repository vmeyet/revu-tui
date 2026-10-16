//! The queue in two calls, three when scoped to a project, all in parallel. Wire shapes stay
//! private; callers get `Fetched`, its `Queue` and their `Sections`.
//! GitLab refuses a query scoring over 250. With the fields the "needs me" rules read, the MRs
//! asking me scored 185, mine 87 (the rules skip them, so they skip the fields) and a project's
//! open MRs 124, sizes included: each call stays under the limit, one query for all would not.
//! GitLab can take a minute to count an MR's changed lines, or give up on it, so the sizes come
//! from their own queries over the same lists: the queue waits on them a few seconds at most.
use super::Client;
use crate::forge::{Fetched, MrKey, PipelineStatus, ProjectPath, Queue, QueueMr, Readiness, ReviewState, ReviewerState, Size};
use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::json;
use std::collections::HashMap;
use std::time::Duration;

const REQUESTED: &str = "reviewRequested: reviewRequestedMergeRequests(state: opened, first: 50, sort: UPDATED_DESC)";
const ASSIGNED: &str = "assigned: assignedMergeRequests(state: opened, first: 50, sort: UPDATED_DESC)";
const AUTHORED: &str = "authored: authoredMergeRequests(state: opened, first: 50, sort: UPDATED_DESC)";
const OPEN: &str = "mergeRequests(state: opened, first: 100, sort: UPDATED_DESC)";

const FIELDS: &str = "id iid title description draft webUrl updatedAt createdAt
    sourceBranch targetBranch conflicts
    project { id fullPath }
    author { username name avatarUrl }
    approved approvedBy { nodes { username } }
    reviewers { nodes { username mergeRequestInteraction { reviewState } } }
    headPipeline { status detailedStatus { label } }
    resolvableDiscussionsCount resolvedDiscussionsCount
    labels { nodes { title } }
    userNotesCount";

/// What only the "needs me" rules read: approvals still missing, and who commented.
const RULES: &str = "approvalsLeft commenters { nodes { username } }";

const SIZE: &str = "iid project { fullPath } diffStatsSummary { additions deletions fileCount }";

const SET_DRAFT: &str = r"
mutation SetDraft($project: ID!, $iid: String!, $draft: Boolean!) {
  mergeRequestSetDraft(input: {projectPath: $project, iid: $iid, draft: $draft}) { errors }
}";

/// How long the queue waits on the parts it can do without.
#[derive(Clone, Copy)]
struct Patience {
    /// The project's open MRs, before my own lists come back without them.
    open: Duration,
    /// The sizes, before the rows come back without one.
    sizes: Duration,
}

const PATIENCE: Patience = Patience { open: Duration::from_secs(20), sizes: Duration::from_secs(4) };

fn nodes(list: &str, fields: &str) -> String {
    format!("{list} {{ nodes {{ {fields} }} }}")
}

fn mine_query(name: &str, lists: &[String]) -> String {
    format!("query {name} {{ currentUser {{ username {} }} }}", lists.join(" "))
}

fn open_query(name: &str, fields: &str) -> String {
    format!("query {name}($project: ID!) {{ project(fullPath: $project) {{ {} }} }}", nodes(OPEN, fields))
}

fn asking_query() -> String {
    let fields = format!("{FIELDS} {RULES}");
    mine_query("Asking", &[nodes(REQUESTED, &fields), nodes(ASSIGNED, &fields)])
}

fn authored_query() -> String {
    mine_query("Authored", &[nodes(AUTHORED, FIELDS)])
}

fn project_query() -> String {
    open_query("Open", &format!("{FIELDS} {RULES}"))
}

fn my_sizes_query() -> String {
    mine_query("MySizes", &[nodes(REQUESTED, SIZE), nodes(ASSIGNED, SIZE), nodes(AUTHORED, SIZE)])
}

fn open_sizes_query() -> String {
    open_query("OpenSizes", SIZE)
}

impl Client {
    /// Every MR waiting on me; with `project`, only that project's, plus all its other open MRs.
    pub(crate) async fn queue(&self, project: Option<&ProjectPath>) -> Result<Fetched> {
        self.queue_within(project, PATIENCE).await
    }

    async fn queue_within(&self, project: Option<&ProjectPath>, patience: Patience) -> Result<Fetched> {
        let (lists, sizes) = tokio::join!(Box::pin(self.lists(project, patience.open)), Box::pin(self.sizes(project, patience.sizes)));
        let Fetched { queue, missing } = lists?;
        Ok(Fetched { queue: sized(queue, &sizes), missing })
    }

    /// My lists, which the queue cannot do without, and the project's open MRs, which it can.
    async fn lists(&self, project: Option<&ProjectPath>, patience: Duration) -> Result<Fetched> {
        let mine = async { tokio::try_join!(self.ask(asking_query(), None), self.ask(authored_query(), None)) };
        let Some(path) = project else {
            let (asking, authored) = mine.await?;
            return Ok(Fetched { queue: queue_from(asking, authored)?, missing: None });
        };
        let (mine, open) = tokio::join!(mine, within(patience, self.ask(project_query(), Some(path))));
        let (asking, authored) = mine?;
        Ok(Fetched::scoped(queue_from(asking, authored)?, path, open.and_then(|answer| open_mrs(answer, path))))
    }

    /// The size of every MR the lists hold, by MR; a list that fails or takes longer than `patience` is logged and left out.
    async fn sizes(&self, project: Option<&ProjectPath>, patience: Duration) -> HashMap<MrKey, Size> {
        let mine = within(patience, self.ask(my_sizes_query(), None));
        let open = futures_util::future::OptionFuture::from(project.map(|path| within(patience, self.ask(open_sizes_query(), Some(path)))));
        let (mine, open) = tokio::join!(mine, open);
        let mut sizes = HashMap::new();
        for answer in std::iter::once(mine).chain(open) {
            match answer {
                Ok(lists) => sizes.extend(sizes_of(lists)),
                Err(e) => tracing::warn!("the queue goes without some sizes: {e:#}"),
            }
        }
        sizes
    }

    /// One GraphQL query, about `project` when given; what GitLab answered is kept even when part of it failed.
    async fn ask<T: DeserializeOwned>(&self, query: String, project: Option<&ProjectPath>) -> Result<T> {
        self.http.post_json::<Answer<T>>(super::GRAPHQL, &request(&query, project)).await?.into_partial_data()
    }
}

fn request(query: &str, project: Option<&ProjectPath>) -> serde_json::Value {
    match project {
        Some(path) => json!({"query": query, "variables": {"project": path}}),
        None => json!({"query": query}),
    }
}

impl Client {
    /// Marks the MR a draft, or ready; GitLab leaves an MR already there as it is.
    pub(crate) async fn set_readiness(&self, key: &MrKey, readiness: Readiness) -> Result<()> {
        let draft = readiness == Readiness::Draft;
        let variables = json!({"project": key.project, "iid": key.number.to_string(), "draft": draft});
        let answer: Answer<SetDraftData> =
            self.http.post_json(super::GRAPHQL, &json!({"query": SET_DRAFT, "variables": variables})).await?;
        let payload = answer.into_data()?.merge_request_set_draft.context("GraphQL answered without the merge request")?;
        if !payload.errors.is_empty() {
            bail!("GitLab: {}", payload.errors.join("; "));
        }
        Ok(())
    }
}

async fn within<T>(patience: Duration, ask: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(patience, ask).await.unwrap_or_else(|_| Err(anyhow!("no answer within {} s", patience.as_secs_f32())))
}

/// The queue from my two lists' answers, scoped to no project and without its open MRs.
fn queue_from(asking: Lists<WireMr>, authored: Lists<WireMr>) -> Result<Queue> {
    let user = asking.current_user.context("GraphQL answered without currentUser")?;
    let mine = authored.current_user.context("GraphQL answered without currentUser")?;
    Ok(Queue {
        me: user.username,
        project: None,
        review_requested: convert(user.review_requested)?,
        authored: convert(mine.authored)?,
        assigned: convert(user.assigned)?,
        open: vec![],
    })
}

fn open_mrs(answer: Lists<WireMr>, path: &ProjectPath) -> Result<Vec<QueueMr>> {
    let found = answer.project.with_context(|| format!("project {path} not found, or not visible with this token"))?;
    convert(found.merge_requests)
}

fn sized(queue: Queue, sizes: &HashMap<MrKey, Size>) -> Queue {
    queue.map_rows(|mr| QueueMr { size: sizes.get(&mr.key()).copied(), ..mr })
}

/// A row with an iid GitLab spelled wrong, or no count, is left without a size.
fn sizes_of(lists: Lists<WireSized>) -> impl Iterator<Item = (MrKey, Size)> {
    let mine = lists.current_user.into_iter().flat_map(|user| [user.review_requested, user.assigned, user.authored]);
    let open = lists.project.map(|project| project.merge_requests);
    mine.chain(open).flat_map(|list| list.nodes).filter_map(|node| {
        let stats = node.diff_stats_summary?;
        let key = MrKey::new(node.project.full_path, node.iid.parse().ok()?);
        Some((key, Size { additions: stats.additions, deletions: stats.deletions, files: stats.file_count }))
    })
}

/// A queue straight from a GraphQL answer body; the one body stands for every call's answer, sizes included.
#[cfg(test)]
pub(super) fn queue_from_json(body: &str, project: Option<&str>) -> Result<Queue> {
    let lists = || serde_json::from_str::<Answer<Lists<WireMr>>>(body)?.into_partial_data();
    let queue = queue_from(lists()?, lists()?)?;
    let queue = match project {
        Some(path) => Queue { project: Some(path.into()), open: open_mrs(lists()?, &path.into())?, ..queue },
        None => queue,
    };
    let sizes = sizes_of(serde_json::from_str::<Answer<Lists<WireSized>>>(body)?.into_partial_data()?).collect();
    Ok(sized(queue, &sizes))
}

fn convert(connection: Connection<WireMr>) -> Result<Vec<QueueMr>> {
    connection.nodes.into_iter().map(QueueMr::try_from).collect()
}

impl TryFrom<WireMr> for QueueMr {
    type Error = anyhow::Error;

    fn try_from(w: WireMr) -> Result<Self> {
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
            size: None,
            unresolved: w.resolvable_discussions_count.saturating_sub(w.resolved_discussions_count),
            labels: w.labels.nodes.into_iter().map(|l| l.title).collect(),
            notes: w.user_notes_count,
            commenters: w.commenters.map(|c| c.nodes.into_iter().map(|u| u.username).collect()).unwrap_or_default(),
            reason: None,
        })
    }
}

#[derive(Deserialize)]
struct Answer<T> {
    data: Option<T>,
    errors: Option<Vec<GraphqlError>>,
}

impl<T> Answer<T> {
    /// The data, unless GitLab reported any error.
    fn into_data(self) -> Result<T> {
        if let Some(message) = self.error_message() {
            bail!("GraphQL: {message}");
        }
        self.data.context("GraphQL answered without data")
    }

    /// The data GitLab answered, its errors logged: a field that timed out costs that field, not the rest.
    fn into_partial_data(self) -> Result<T> {
        if let Some(message) = self.error_message() {
            if self.data.is_none() {
                bail!("GraphQL: {message}");
            }
            tracing::warn!("GraphQL answered in part: {message}");
        }
        self.data.context("GraphQL answered without data")
    }

    fn error_message(&self) -> Option<String> {
        let errors = self.errors.as_ref().filter(|e| !e.is_empty())?;
        Some(errors.iter().map(|e| e.message.as_str()).collect::<Vec<_>>().join("; "))
    }
}

#[derive(Deserialize)]
struct GraphqlError {
    message: String,
}

/// What a queue query answers: my lists, or a project's open MRs; `N` is the shape of one MR.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", bound(deserialize = "N: Deserialize<'de>"))]
struct Lists<N> {
    current_user: Option<WireUser<N>>,
    project: Option<WireProjectMrs<N>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetDraftData {
    merge_request_set_draft: Option<MutationPayload>,
}

#[derive(Deserialize)]
struct MutationPayload {
    errors: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", bound(deserialize = "N: Deserialize<'de>"))]
struct WireProjectMrs<N> {
    merge_requests: Connection<N>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", bound(deserialize = "N: Deserialize<'de>"))]
struct WireUser<N> {
    #[serde(default)]
    review_requested: Connection<N>,
    #[serde(default)]
    authored: Connection<N>,
    #[serde(default)]
    assigned: Connection<N>,
    username: String,
}

#[derive(Deserialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
struct Connection<T> {
    #[serde(deserialize_with = "present")]
    nodes: Vec<T>,
}

impl<T> Default for Connection<T> {
    fn default() -> Self {
        Self { nodes: vec![] }
    }
}

/// A list GitLab answered in part holds `null` where one node failed: the others still count.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(deserializer: D) -> Result<Vec<T>, D::Error> {
    Ok(Vec::<Option<T>>::deserialize(deserializer)?.into_iter().flatten().collect())
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
    resolvable_discussions_count: u32,
    resolved_discussions_count: u32,
    labels: Connection<WireLabel>,
    user_notes_count: u32,
    #[serde(default)]
    approvals_left: Option<u32>,
    #[serde(default)]
    commenters: Option<Connection<WireUsername>>,
}

/// One MR as the size queries answer it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireSized {
    iid: String,
    project: WireProject,
    diff_stats_summary: Option<WireStats>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireProject {
    full_path: ProjectPath,
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
    status: PipelineStatus,
}

#[derive(Deserialize)]
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
    const SCOPED: &str = include_str!("fixtures/queue_scoped.json");
    /// Patience for the parts a test makes late; my own lists wait as long as they take.
    const QUICK: Patience = Patience { open: Duration::from_millis(100), sizes: Duration::from_millis(100) };
    /// An answer that never comes within `QUICK`.
    const LATE: Duration = Duration::from_secs(30);

    fn queue() -> Queue {
        queue_from_json(FIXTURE, None).unwrap()
    }

    fn iids(mrs: &[QueueMr]) -> Vec<u64> {
        mrs.iter().map(|m| m.number).collect()
    }

    fn client(server: &MockServer) -> Client {
        Client::with_base(&Credentials { host: "x".into(), token: "glpat-xxxx".into() }, &format!("{}/api/v4/", server.uri())).unwrap()
    }

    /// `query` answered with `answer`; with `project`, only when asked about it.
    async fn answer(server: &MockServer, query: String, project: Option<&ProjectPath>, answer: ResponseTemplate) {
        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .and(body_partial_json(request(&query, project)))
            .respond_with(answer)
            .mount(server)
            .await;
    }

    fn ok(body: &str) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_string(body)
    }

    async fn mount_mine(server: &MockServer, body: &str) {
        for query in [asking_query(), authored_query()] {
            answer(server, query, None, ok(body)).await;
        }
    }

    #[tokio::test]
    async fn queue_posts_the_query_and_parses_the_answer() {
        let server = MockServer::start().await;
        mount_mine(&server, FIXTURE).await;
        answer(&server, my_sizes_query(), None, ok(FIXTURE)).await;
        let Fetched { queue, missing } = client(&server).queue(None).await.unwrap();
        assert_eq!((queue.me.as_str(), missing), ("nina", None));
        assert_eq!(iids(&queue.review_requested), [42, 40]);
        let first = &queue.review_requested[0];
        assert_eq!(first.key(), MrKey::new("acme/widgets", 42));
        assert_eq!(first.my_state("nina"), Some(ReviewState::Unreviewed));
        assert_eq!(first.size, Some(Size { additions: 412, deletions: 38, files: 9 }));
        assert_eq!(first.unresolved, 1);
        assert_eq!(first.pipeline, Some(PipelineStatus::Success));
        assert_eq!(queue.assigned[0].size, None, "an MR GitLab did not count has no size");
    }

    #[tokio::test]
    async fn sizes_that_fail_leave_the_rows_without_one() {
        let server = MockServer::start().await;
        mount_mine(&server, FIXTURE).await;
        answer(&server, my_sizes_query(), None, ResponseTemplate::new(500)).await;
        let queue = client(&server).queue(None).await.unwrap().queue;
        assert_eq!(iids(&queue.review_requested), [42, 40]);
        assert!(queue.review_requested.iter().chain(&queue.authored).chain(&queue.assigned).all(|mr| mr.size.is_none()));
    }

    #[tokio::test]
    async fn sizes_that_come_too_late_leave_the_rows_without_one() {
        let server = MockServer::start().await;
        mount_mine(&server, FIXTURE).await;
        answer(&server, my_sizes_query(), None, ok(FIXTURE).set_delay(LATE)).await;
        let queue = client(&server).queue_within(None, QUICK).await.unwrap().queue;
        assert_eq!(iids(&queue.review_requested), [42, 40]);
        assert_eq!(queue.review_requested[0].size, None);
    }

    #[tokio::test]
    async fn graphql_errors_without_data_surface_as_messages() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"errors": [{"message": "Field x doesn't exist"}]})))
            .mount(&server)
            .await;
        let err = client(&server).queue(None).await.unwrap_err().to_string();
        assert!(err.contains("doesn't exist"), "{err}");
    }

    #[tokio::test]
    async fn an_answer_with_errors_keeps_the_data_it_holds() {
        let server = MockServer::start().await;
        let partial = FIXTURE
            .replacen("{\n  \"data\"", "{\"errors\": [{\"message\": \"Timeout on DiffStatsSummary.additions\"}], \"data\"", 1)
            .replacen("\"nodes\": [", "\"nodes\": [null, ", 1);
        mount_mine(&server, &partial).await;
        answer(&server, my_sizes_query(), None, ok(&partial)).await;
        let queue = client(&server).queue(None).await.unwrap().queue;
        assert_eq!(iids(&queue.review_requested), [42, 40], "the failed node is left out, the others stay");
        assert_eq!(queue.review_requested[0].size.map(Size::lines), Some(450));
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

    #[tokio::test]
    async fn a_scoped_queue_sends_the_project_and_reads_its_open_mrs() {
        let server = MockServer::start().await;
        let project = ProjectPath::from("acme/widgets");
        answer(&server, project_query(), Some(&project), ok(SCOPED)).await;
        answer(&server, open_sizes_query(), Some(&project), ok(SCOPED)).await;
        mount_mine(&server, FIXTURE).await;
        let Fetched { queue, missing } = client(&server).queue(Some(&"acme/widgets".into())).await.unwrap();
        assert_eq!((queue.project.as_ref().map(ProjectPath::as_str), missing), (Some("acme/widgets"), None));
        assert_eq!(iids(&queue.open), [42, 50, 51]);
        assert_eq!(queue.open[1].description, "Adds the refund flow.");
        assert!(queue.open[1].size.is_some(), "the project's sizes come from their own query");
    }

    #[tokio::test]
    async fn a_project_that_does_not_answer_in_time_leaves_my_lists_and_says_so() {
        let server = MockServer::start().await;
        answer(&server, project_query(), None, ok(SCOPED).set_delay(LATE)).await;
        mount_mine(&server, FIXTURE).await;
        let Fetched { queue, missing } = client(&server).queue_within(Some(&"acme/widgets".into()), QUICK).await.unwrap();
        assert_eq!(iids(&queue.review_requested), [42, 40]);
        assert_eq!(queue.open, [] as [QueueMr; 0]);
        assert_eq!(queue.project.as_ref().map(ProjectPath::as_str), Some("acme/widgets"), "still scoped to its project");
        let missing = missing.expect("the missing part is named");
        assert!(missing.starts_with("the open MRs of acme/widgets did not load"), "{missing}");
    }

    #[tokio::test]
    async fn an_unknown_project_is_named_not_an_empty_list() {
        let server = MockServer::start().await;
        answer(&server, project_query(), None, ResponseTemplate::new(200).set_body_json(json!({"data": {"project": null}}))).await;
        mount_mine(&server, FIXTURE).await;
        let missing = client(&server).queue(Some(&"acme/gone".into())).await.unwrap().missing.unwrap();
        assert!(missing.contains("project acme/gone not found"), "{missing}");
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

    #[tokio::test]
    async fn set_draft_sends_the_project_the_iid_and_the_state_and_reads_the_errors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .and(body_partial_json(json!({"variables": {"project": "acme/widgets", "iid": "42", "draft": true}})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": {"mergeRequestSetDraft": {"errors": []}}})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/graphql"))
            .and(body_partial_json(json!({"variables": {"iid": "43"}})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": {"mergeRequestSetDraft": {"errors": ["no can do"]}}})))
            .mount(&server)
            .await;
        let client = client(&server);
        client.set_readiness(&MrKey::new("acme/widgets", 42), Readiness::Draft).await.unwrap();
        let err = client.set_readiness(&MrKey::new("acme/widgets", 43), Readiness::Draft).await.unwrap_err().to_string();
        assert!(err.contains("no can do"), "{err}");
    }
}
