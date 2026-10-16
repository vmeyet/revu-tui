//! The queue: every PR waiting on me in four searches, and which PRs are done.
use crate::forge::github::wire::{Actor, Nodes, QueuePr};
use crate::forge::github::{Client, owner_and_name};
use crate::forge::{Fetched, ProjectPath, Queue, QueueMr};
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{Value, json};

const PR_FIELDS: &str = r"
fragment pr on PullRequest {
  number title body isDraft url updatedAt createdAt headRefName baseRefName mergeable
  repository { nameWithOwner }
  author { login ... on User { name databaseId } }
  reviewDecision
  latestReviews(first: 20) { nodes { state author { login } } }
  reviewRequests(first: 20) { nodes { requestedReviewer { ... on User { login } } } }
  commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
  additions deletions changedFiles
  reviewThreads(first: 100) { nodes { isResolved } }
  labels(first: 20) { nodes { name } }
  comments { totalCount }
  participants(first: 30) { nodes { login } }
}";

const QUEUE: &str = r"
query Queue($requested: String!, $authored: String!, $assigned: String!, $reviewed: String!) {
  viewer { login }
  requested: search(type: ISSUE, first: 50, query: $requested) { nodes { ...pr } }
  authored: search(type: ISSUE, first: 50, query: $authored) { nodes { ...pr } }
  assigned: search(type: ISSUE, first: 50, query: $assigned) { nodes { ...pr } }
  reviewed: search(type: ISSUE, first: 50, query: $reviewed) { nodes { ...pr } }
}";

const OPEN: &str = r"
query Open($owner: String!, $name: String!) {
  repository(owner: $owner, name: $name) {
    pullRequests(states: OPEN, first: 100, orderBy: {field: UPDATED_AT, direction: DESC}) { nodes { ...pr } }
  }
}";

/// The four searches of the queue, scoped to one repository when asked.
fn searches(project: Option<&ProjectPath>) -> Value {
    let scope = project.map(|p| format!(" repo:{p}")).unwrap_or_default();
    let query = |filter: &str| format!("is:pr is:open archived:false {filter}{scope}");
    json!({
        "requested": query("review-requested:@me"),
        "authored": query("author:@me"),
        "assigned": query("assignee:@me"),
        "reviewed": query("reviewed-by:@me -author:@me"),
    })
}

#[derive(Deserialize)]
struct QueueData {
    viewer: Actor,
    requested: Nodes<Value>,
    authored: Nodes<Value>,
    assigned: Nodes<Value>,
    reviewed: Nodes<Value>,
}

#[derive(Deserialize)]
struct OpenData {
    repository: Option<OpenRepository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenRepository {
    pull_requests: Nodes<Value>,
}

/// Search answers every issue type; only pull requests carry the fragment's fields.
fn prs(nodes: Nodes<Value>) -> Vec<QueueMr> {
    nodes.nodes.into_iter().filter_map(|node| serde_json::from_value::<QueuePr>(node).ok()).map(QueueMr::from).collect()
}

/// Reviewed and requested once more both land in `review_requested`, each MR once: the queue
/// sorts them into To review or Done by my state.
fn queue_from(data: QueueData, project: Option<&ProjectPath>, open: Vec<QueueMr>) -> Queue {
    let mut review_requested = prs(data.requested);
    for mr in prs(data.reviewed) {
        if !review_requested.iter().any(|r| r.key() == mr.key()) {
            review_requested.push(mr);
        }
    }
    Queue {
        me: data.viewer.login,
        project: project.cloned(),
        review_requested,
        authored: prs(data.authored),
        assigned: prs(data.assigned),
        open,
    }
}

impl Client {
    /// Every PR waiting on me; with `project`, only that repository's, plus all its other open PRs.
    /// Two calls in parallel keep each query small; when the repository's fails, my searches still answer.
    pub(crate) async fn queue(&self, project: Option<&ProjectPath>) -> Result<Fetched> {
        let mine_query = format!("{QUEUE}{PR_FIELDS}");
        let mine = self.graphql::<QueueData>(&mine_query, searches(project));
        let Some(project) = project else { return Ok(Fetched { queue: queue_from(mine.await?, None, vec![]), missing: None }) };
        let (mine, open) = tokio::join!(mine, self.open_prs(project));
        Ok(Fetched::scoped(queue_from(mine?, Some(project), vec![]), project, open))
    }

    async fn open_prs(&self, project: &ProjectPath) -> Result<Vec<QueueMr>> {
        let (owner, name) = owner_and_name(project)?;
        let open: OpenData = self.graphql(&format!("{OPEN}{PR_FIELDS}"), json!({"owner": owner, "name": name})).await?;
        let repository = open.repository.with_context(|| format!("repository {project} not found, or not visible with this token"))?;
        Ok(prs(repository.pull_requests))
    }

    /// Which of `numbers` in `project` are merged or closed: one query per fifty, each PR an alias.
    pub(crate) async fn finished(&self, project: &ProjectPath, numbers: &[u64]) -> Result<Vec<u64>> {
        let (owner, name) = owner_and_name(project)?;
        let mut done = vec![];
        for chunk in numbers.chunks(50) {
            let fields: String = chunk.iter().map(|n| format!(" p{n}: pullRequest(number: {n}) {{ state }}")).collect();
            let query = format!("query($owner: String!, $name: String!) {{ repository(owner: $owner, name: $name) {{{fields} }} }}");
            let data: Value = self.graphql(&query, json!({"owner": owner, "name": name})).await?;
            let state = |n: &u64| data["repository"][format!("p{n}")]["state"].as_str().map(str::to_owned);
            done.extend(chunk.iter().filter(|n| state(n).is_some_and(|s| s != "OPEN")));
        }
        Ok(done)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::client;
    use super::*;
    use crate::forge::github::wire;
    use crate::forge::{self, ReviewState};
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// A queue straight from GraphQL answer bodies, for tests.
    fn queue_from_json(mine: &str, open: Option<(&str, &str)>) -> Result<Queue> {
        let mine: QueueData = serde_json::from_str::<wire::Answer<QueueData>>(mine)?.into_data()?;
        let Some((body, project)) = open else { return Ok(queue_from(mine, None, vec![])) };
        let open: OpenData = serde_json::from_str::<wire::Answer<OpenData>>(body)?.into_data()?;
        Ok(queue_from(mine, Some(&project.into()), prs(open.repository.context("no repository")?.pull_requests)))
    }

    fn numbers(mrs: &[QueueMr]) -> Vec<u64> {
        mrs.iter().map(|m| m.number).collect()
    }

    #[test]
    fn the_queue_sorts_searches_into_sections_and_skips_non_prs() {
        let queue = queue_from_json(include_str!("../fixtures/queue.json"), None).unwrap();
        let sections = queue.sections(&[]);
        assert_eq!(queue.me, "nina");
        assert_eq!(numbers(&sections.to_review), [42, 40], "asked again after approving counts as asked");
        assert_eq!(numbers(&sections.done), [39]);
        assert_eq!(numbers(&sections.mine), [41]);
        assert_eq!(numbers(&sections.watching), [35]);
        let failing = &sections.to_review[0];
        assert_eq!(
            (failing.pipeline, failing.unresolved, failing.project.as_str()),
            (Some(forge::PipelineStatus::Failed), 1, "acme/widgets")
        );
        assert!(sections.done[0].conflicts);
        assert_eq!(sections.done[0].my_state("nina"), Some(ReviewState::Reviewed));
        assert!(sections.mine[0].approved && sections.mine[0].approved_by == ["lea"]);
    }

    #[test]
    fn a_scoped_queue_adds_the_other_open_prs_of_the_repository() {
        let queue =
            queue_from_json(include_str!("../fixtures/queue.json"), Some((include_str!("../fixtures/open.json"), "acme/widgets"))).unwrap();
        let sections = queue.sections(&[]);
        assert_eq!(sections.open, [] as [QueueMr; 0]);
        assert_eq!(numbers(&sections.drafts), [44], "a draft PR waits apart from the open ones");
        assert!(sections.watching.is_empty(), "acme/infra is out of scope");
    }

    #[tokio::test]
    async fn a_repository_whose_open_prs_fail_leaves_my_searches_and_says_so() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("query Queue("))
            .respond_with(ResponseTemplate::new(200).set_body_string(include_str!("../fixtures/queue.json")))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("query Open("))
            .respond_with(ResponseTemplate::new(502))
            .mount(&server)
            .await;
        let Fetched { queue, missing } = client(&server).queue(Some(&"acme/widgets".into())).await.unwrap();
        assert_eq!(numbers(&queue.review_requested), [42, 40, 39]);
        assert_eq!(queue.open, [] as [QueueMr; 0]);
        let missing = missing.expect("the missing part is named");
        assert!(missing.starts_with("the open MRs of acme/widgets did not load"), "{missing}");
    }

    #[test]
    fn searches_are_scoped_with_repo() {
        let scoped = searches(Some(&"acme/widgets".into()));
        assert_eq!(scoped["requested"], "is:pr is:open archived:false review-requested:@me repo:acme/widgets");
        assert_eq!(searches(None)["reviewed"], "is:pr is:open archived:false reviewed-by:@me -author:@me");
    }

    #[tokio::test]
    async fn finished_asks_every_pr_in_one_query_and_names_the_closed_ones() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("p7: pullRequest(number: 7)"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": {"repository": {
                "p7": {"state": "MERGED"}, "p8": {"state": "OPEN"}, "p9": {"state": "CLOSED"}
            }}})))
            .mount(&server)
            .await;
        assert_eq!(client(&server).finished(&"acme/widgets".into(), &[7, 8, 9]).await.unwrap(), [7, 9]);
    }
}
