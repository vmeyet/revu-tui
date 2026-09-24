//! Everything GitHub answers best in GraphQL: the queue, one PR, its threads, and the pending
//! review that holds my drafts until `publish` submits it.
use super::wire::{self, Actor, Anchor, Label, Nodes, QueuePr, Review, ReviewRequest, user_of};
use super::{Client, owner_and_name};
use crate::forge::{self, Approvals, Discussion, Draft, MrKey, NewDraft, Note, Pipeline, Queue, QueueMr, Refs};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
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

const MR: &str = r"
query Mr($owner: String!, $name: String!, $number: Int!) {
  viewer { login }
  repository(owner: $owner, name: $name) {
    squashMergeAllowed mergeCommitAllowed rebaseMergeAllowed deleteBranchOnMerge
    pullRequest(number: $number) {
      number title body state isDraft url updatedAt baseRefName headRefName baseRefOid headRefOid changedFiles mergeable
      author { login ... on User { name databaseId } }
      reviewDecision
      reviewRequests(first: 20) { nodes { requestedReviewer { ... on User { login name databaseId } } } }
      latestReviews(first: 20) { nodes { state author { login ... on User { name databaseId } } } }
      labels(first: 20) { nodes { name } }
      commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
    }
  }
}";

const DRAFT_STATE: &str = r"
query DraftState($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) { pullRequest(number: $number) { id isDraft } }
}";

const THREADS: &str = r"
query Threads($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      id baseRefOid headRefOid
      reviewThreads(first: 100) {
        nodes {
          id isResolved path line originalLine startLine originalStartLine diffSide startDiffSide
          comments(first: 100) { nodes { ...comment state } }
        }
      }
      reviews(first: 100) { nodes { ...comment state } }
      comments(first: 100) { nodes { ...comment } }
    }
  }
}
fragment comment on Comment {
  id body createdAt updatedAt author { login ... on User { name databaseId } }
  ... on Reactable { reactionGroups { content viewerHasReacted reactors { totalCount } } }
  ... on PullRequestReviewComment { fullDatabaseId }
  ... on PullRequestReview { fullDatabaseId }
  ... on IssueComment { fullDatabaseId }
}";

/// The four searches of the queue, scoped to one repository when asked.
fn searches(project: Option<&str>) -> Value {
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
fn queue_from(data: QueueData, project: Option<&str>, open: Vec<QueueMr>) -> Queue {
    let mut review_requested = prs(data.requested);
    for mr in prs(data.reviewed) {
        if !review_requested.iter().any(|r| r.key() == mr.key()) {
            review_requested.push(mr);
        }
    }
    Queue {
        me: data.viewer.login,
        project: project.map(str::to_owned),
        review_requested,
        authored: prs(data.authored),
        assigned: prs(data.assigned),
        open,
    }
}

#[derive(Deserialize)]
struct MrData {
    viewer: Actor,
    repository: Option<MrRepository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MrRepository {
    pull_request: Option<Pr>,
    #[serde(default)]
    squash_merge_allowed: bool,
    #[serde(default)]
    merge_commit_allowed: bool,
    #[serde(default)]
    rebase_merge_allowed: bool,
    #[serde(default)]
    delete_branch_on_merge: bool,
}

impl MrRepository {
    /// Squash when the repo allows it, else a merge commit, else rebase: the tidiest history on offer.
    fn merge_plan(&self) -> forge::MergePlan {
        let method = match (self.squash_merge_allowed, self.merge_commit_allowed, self.rebase_merge_allowed) {
            (true, _, _) => forge::MergeMethod::Squash,
            (false, true, _) | (false, false, false) => forge::MergeMethod::Merge,
            (false, false, true) => forge::MergeMethod::Rebase,
        };
        forge::MergePlan { method, remove_branch: self.delete_branch_on_merge }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pr {
    number: u64,
    title: String,
    #[serde(default)]
    body: Option<String>,
    state: String,
    is_draft: bool,
    url: String,
    updated_at: DateTime<Utc>,
    base_ref_name: String,
    head_ref_name: String,
    base_ref_oid: String,
    head_ref_oid: String,
    changed_files: u32,
    #[serde(default)]
    mergeable: Option<String>,
    #[serde(default)]
    author: Option<Actor>,
    #[serde(default)]
    review_decision: Option<String>,
    review_requests: Nodes<ReviewRequest>,
    latest_reviews: Nodes<Review>,
    labels: Nodes<Label>,
    commits: Nodes<wire::CommitNode>,
}

impl Pr {
    fn into_model(self, project: &str, me: &str) -> forge::Mr {
        let author = user_of(self.author.clone());
        let approvers: Vec<forge::User> =
            self.latest_reviews.nodes.iter().filter(|r| r.state == "APPROVED").map(|r| user_of(r.author.clone())).collect();
        let reviewers = self
            .review_requests
            .nodes
            .iter()
            .filter_map(|r| r.requested_reviewer.clone())
            .chain(self.latest_reviews.nodes.iter().filter_map(|r| r.author.clone()))
            .map(|a| user_of(Some(a)))
            .fold(Vec::<forge::User>::new(), |mut all, user| {
                if !all.iter().any(|u| u.username == user.username) {
                    all.push(user);
                }
                all
            });
        let approved = self.review_decision.as_deref() == Some("APPROVED") || (self.review_decision.is_none() && !approvers.is_empty());
        let approvals = Approvals {
            approved,
            approvals_left: u32::from(self.review_decision.as_deref() == Some("REVIEW_REQUIRED")),
            user_has_approved: approvers.iter().any(|u| u.username == me),
            user_can_approve: author.username != me,
            approved_by: approvers,
        };
        let pipeline = wire::pipeline(&self.commits)
            .map(|status| Pipeline { status: status.to_lowercase(), web_url: Some(format!("{}/checks", self.url)) });
        forge::Mr {
            project: project.to_owned(),
            number: self.number,
            title: self.title,
            description: self.body.unwrap_or_default(),
            state: match self.state.as_str() {
                "OPEN" => "opened",
                "MERGED" => "merged",
                _ => "closed",
            }
            .to_owned(),
            draft: self.is_draft,
            mine: author.username == me,
            author,
            source_branch: self.head_ref_name,
            target_branch: self.base_ref_name,
            web_url: self.url,
            updated_at: self.updated_at,
            refs: Refs { base: self.base_ref_oid.clone(), start: self.base_ref_oid, head: self.head_ref_oid },
            pipeline,
            changes_count: Some(self.changed_files.to_string()),
            conflicts: self.mergeable.as_deref() == Some("CONFLICTING"),
            reviewers,
            labels: self.labels.nodes.into_iter().map(|l| l.name).collect(),
            approvals,
            merge: forge::MergePlan::default(),
        }
    }
}

#[derive(Deserialize)]
struct DraftStateData {
    repository: Option<DraftStateRepository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DraftStateRepository {
    pull_request: Option<DraftState>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DraftState {
    id: String,
    is_draft: bool,
}

#[derive(Deserialize)]
struct ThreadsData {
    repository: Option<ThreadsRepository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadsRepository {
    pull_request: Option<PrThreads>,
}

/// One PR's threads, reviews and comments: the published ones make discussions, the pending
/// ones (only mine are visible) make drafts.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PrThreads {
    id: String,
    base_ref_oid: String,
    head_ref_oid: String,
    review_threads: Nodes<Thread>,
    reviews: Nodes<Comment>,
    comments: Nodes<Comment>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Thread {
    id: String,
    is_resolved: bool,
    #[serde(flatten)]
    anchor: Anchor,
    comments: Nodes<Comment>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Comment {
    id: String,
    #[serde(default)]
    full_database_id: Option<String>,
    body: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    #[serde(default)]
    author: Option<Actor>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    reaction_groups: Vec<ReactionGroup>,
}

/// GitHub's count of one reaction on a comment, and whether I am among them.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReactionGroup {
    content: String,
    viewer_has_reacted: bool,
    reactors: Total,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Total {
    total_count: u32,
}

impl Comment {
    fn pending(&self) -> bool {
        self.state.as_deref() == Some("PENDING")
    }

    fn number(&self) -> u64 {
        self.full_database_id.as_deref().and_then(|id| id.parse().ok()).unwrap_or_default()
    }

    fn note(&self, resolvable: bool, resolved: bool, position: Option<forge::Position>) -> Note {
        Note {
            id: self.number(),
            body: self.body.clone(),
            author: user_of(self.author.clone()),
            created_at: self.created_at,
            updated_at: self.updated_at,
            system: false,
            resolvable,
            resolved,
            position,
            suggestions: vec![],
            reactions: self
                .reaction_groups
                .iter()
                .filter(|g| g.reactors.total_count > 0)
                .filter_map(|g| {
                    let emoji = forge::Emoji::named(&g.content)?;
                    Some(forge::Reaction { emoji, count: g.reactors.total_count, mine: g.viewer_has_reacted })
                })
                .collect(),
            node: Some(self.id.clone()),
        }
    }
}

impl PrThreads {
    fn refs(&self) -> Refs {
        Refs { base: self.base_ref_oid.clone(), start: self.base_ref_oid.clone(), head: self.head_ref_oid.clone() }
    }

    /// My pending review, which holds every draft: its node id and its database id.
    fn pending_review(&self) -> Option<&Comment> {
        self.reviews.nodes.iter().find(|r| r.pending())
    }

    /// Published threads with their published notes, then review summaries and PR comments as
    /// unanchored threads, oldest first.
    pub(super) fn discussions(&self) -> Vec<Discussion> {
        let refs = self.refs();
        let threads = self.review_threads.nodes.iter().filter_map(|thread| {
            let published: Vec<&Comment> = thread.comments.nodes.iter().filter(|c| !c.pending()).collect();
            let position = thread.anchor.position(&refs);
            let notes: Vec<Note> = published
                .iter()
                .enumerate()
                .map(|(i, c)| c.note(true, thread.is_resolved, if i == 0 { position.clone() } else { None }))
                .collect();
            (!notes.is_empty()).then(|| Discussion { id: thread.id.clone(), notes })
        });
        let summaries = self.reviews.nodes.iter().filter(|r| !r.pending() && !r.body.trim().is_empty());
        let loose = summaries.chain(&self.comments.nodes).map(|c| Discussion { id: c.id.clone(), notes: vec![c.note(false, false, None)] });
        let mut all: Vec<Discussion> = threads.chain(loose).collect();
        all.sort_by_key(|d| d.notes.first().map(|n| n.created_at));
        all
    }

    /// My pending comments: a thread's first comment hangs on its line, a later one answers the
    /// thread; the pending review's own text is the one draft on the MR itself.
    pub(super) fn drafts(&self) -> Vec<Draft> {
        let refs = self.refs();
        let in_threads = self.review_threads.nodes.iter().flat_map(|thread| {
            let position = thread.anchor.position(&refs);
            thread.comments.nodes.iter().enumerate().filter(|(_, c)| c.pending()).map(move |(i, c)| Draft {
                id: c.number(),
                body: c.body.clone(),
                position: if i == 0 { position.clone() } else { None },
                reply_to: (i > 0).then(|| thread.id.clone()),
                resolve: false,
            })
        });
        let summary = self.pending_review().filter(|r| !r.body.trim().is_empty()).map(|r| Draft {
            id: r.number(),
            body: r.body.clone(),
            position: None,
            reply_to: None,
            resolve: false,
        });
        in_threads.chain(summary).collect()
    }

    /// The node id of my pending comment `id`, or of the pending review when `id` is its own.
    fn node_of(&self, id: u64) -> Option<Target> {
        if let Some(review) = self.pending_review().filter(|r| r.number() == id) {
            return Some(Target::Summary(review.id.clone()));
        }
        self.review_threads
            .nodes
            .iter()
            .flat_map(|t| &t.comments.nodes)
            .find(|c| c.pending() && c.number() == id)
            .map(|c| Target::Comment(c.id.clone()))
    }
}

/// What a draft id names on GitHub: a pending comment, or the pending review's own text.
enum Target {
    Comment(String),
    Summary(String),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Created {
    #[serde(default)]
    add_pull_request_review: Option<ReviewPayload>,
    #[serde(default)]
    update_pull_request_review: Option<ReviewPayload>,
    #[serde(default)]
    add_pull_request_review_thread: Option<ThreadPayload>,
    #[serde(default)]
    add_pull_request_review_thread_reply: Option<CommentPayload>,
    #[serde(default)]
    update_pull_request_review_comment: Option<CommentPayload>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewPayload {
    pull_request_review: Comment,
}

#[derive(Deserialize)]
struct ThreadPayload {
    thread: Thread,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommentPayload {
    comment: Option<Comment>,
    #[serde(default)]
    pull_request_review_comment: Option<Comment>,
}

impl CommentPayload {
    fn comment(self) -> Result<Comment> {
        self.comment.or(self.pull_request_review_comment).context("GitHub answered without the comment")
    }
}

const COMMENT_FIELDS: &str = "id fullDatabaseId body createdAt updatedAt state author { login ... on User { name databaseId } }";

impl Client {
    /// Every PR waiting on me; with `project`, only that repository's, plus all its other open PRs.
    /// Two calls in parallel keep each query small.
    pub async fn queue(&self, project: Option<&str>) -> Result<Queue> {
        let (mine_query, open_query) = (format!("{QUEUE}{PR_FIELDS}"), format!("{OPEN}{PR_FIELDS}"));
        let mine = self.graphql::<QueueData>(&mine_query, searches(project));
        let Some(project) = project else { return Ok(queue_from(mine.await?, None, vec![])) };
        let (owner, name) = owner_and_name(project)?;
        let open = self.graphql::<OpenData>(&open_query, json!({"owner": owner, "name": name}));
        let (mine, open) = tokio::try_join!(mine, open)?;
        let repository = open.repository.with_context(|| format!("repository {project} not found, or not visible with this token"))?;
        Ok(queue_from(mine, Some(project), prs(repository.pull_requests)))
    }

    pub async fn mr(&self, key: &MrKey) -> Result<forge::Mr> {
        let (owner, name) = owner_and_name(&key.project)?;
        let data: MrData = self.graphql(MR, json!({"owner": owner, "name": name, "number": key.number})).await?;
        let repository = data.repository.with_context(|| format!("{} not found", key.project))?;
        let plan = repository.merge_plan();
        let pr = repository.pull_request.with_context(|| format!("{}#{} not found", key.project, key.number))?;
        Ok(forge::Mr { merge: plan, ..pr.into_model(&key.project, &data.viewer.login) })
    }

    /// Adds my `emoji` to the comment `node`, or takes it off; GitHub needs no reaction id for either.
    pub async fn react(&self, node: &str, emoji: forge::Emoji, on: bool) -> Result<()> {
        let mutation = if on { "addReaction" } else { "removeReaction" };
        let query = format!(
            "mutation($id: ID!, $content: ReactionContent!) {{ {mutation}(input: {{subjectId: $id, content: $content}}) {{ clientMutationId }} }}"
        );
        self.graphql::<serde_json::Value>(&query, json!({"id": node, "content": emoji.github()})).await.map(|_| ())
    }

    /// Marks the PR a draft, or ready for review; asked first, so a PR already there is left alone.
    pub async fn set_draft(&self, key: &MrKey, draft: bool) -> Result<()> {
        let (owner, name) = owner_and_name(&key.project)?;
        let data: DraftStateData = self.graphql(DRAFT_STATE, json!({"owner": owner, "name": name, "number": key.number})).await?;
        let pr = data.repository.and_then(|r| r.pull_request).with_context(|| format!("{}#{} not found", key.project, key.number))?;
        if pr.is_draft == draft {
            return Ok(());
        }
        let verb = if draft { "convertPullRequestToDraft" } else { "markPullRequestReadyForReview" };
        let query = format!("mutation($id: ID!) {{ {verb}(input: {{pullRequestId: $id}}) {{ clientMutationId }} }}");
        self.graphql::<Value>(&query, json!({"id": pr.id})).await.map(|_| ())
    }

    pub(super) async fn threads(&self, key: &MrKey) -> Result<PrThreads> {
        let (owner, name) = owner_and_name(&key.project)?;
        let data: ThreadsData = self.graphql(THREADS, json!({"owner": owner, "name": name, "number": key.number})).await?;
        data.repository.and_then(|r| r.pull_request).with_context(|| format!("{}#{} not found", key.project, key.number))
    }

    pub async fn discussions(&self, key: &MrKey) -> Result<Vec<Discussion>> {
        Ok(self.threads(key).await?.discussions())
    }

    pub async fn drafts(&self, key: &MrKey) -> Result<Vec<Draft>> {
        Ok(self.threads(key).await?.drafts())
    }

    /// Adds the draft to my pending review, opening one first when there is none.
    pub async fn create_draft(&self, key: &MrKey, draft: &NewDraft) -> Result<Draft> {
        let prs = self.threads(key).await?;
        let review = match prs.pending_review() {
            Some(review) => review.clone(),
            None => self.open_review(&prs.id).await?,
        };
        match (&draft.position, &draft.reply_to) {
            (_, Some(thread)) => self.reply_draft(&review.id, thread, &draft.body).await,
            (Some(position), None) => self.thread_draft(&review.id, position, &draft.body).await,
            (None, None) => {
                let body =
                    [review.body.trim(), draft.body.as_str()].iter().filter(|t| !t.is_empty()).copied().collect::<Vec<_>>().join("\n\n");
                self.summary_draft(&review.id, &body).await
            }
        }
    }

    /// GitHub keeps a comment where it was written: only the text changes.
    pub async fn update_draft(&self, key: &MrKey, id: u64, draft: &NewDraft) -> Result<Draft> {
        match self.threads(key).await?.node_of(id).with_context(|| format!("draft {id} is not in my pending review"))? {
            Target::Summary(review) => self.summary_draft(&review, &draft.body).await,
            Target::Comment(node) => {
                let query = format!(
                    "mutation($id: ID!, $body: String!) {{ updatePullRequestReviewComment(input: {{pullRequestReviewCommentId: $id, body: $body}}) {{ pullRequestReviewComment {{ {COMMENT_FIELDS} }} }} }}"
                );
                let created: Created = self.graphql(&query, json!({"id": node, "body": draft.body})).await?;
                let comment = created.update_pull_request_review_comment.context("GitHub answered without the comment")?.comment()?;
                Ok(Draft {
                    id: comment.number(),
                    body: comment.body,
                    position: draft.position.clone(),
                    reply_to: draft.reply_to.clone(),
                    resolve: false,
                })
            }
        }
    }

    pub async fn delete_draft(&self, key: &MrKey, id: u64) -> Result<()> {
        match self.threads(key).await?.node_of(id).with_context(|| format!("draft {id} is not in my pending review"))? {
            Target::Summary(review) => self.summary_draft(&review, "").await.map(|_| ()),
            Target::Comment(node) => {
                let query = "mutation($id: ID!) { deletePullRequestReviewComment(input: {id: $id}) { clientMutationId } }";
                self.graphql::<Value>(query, json!({"id": node})).await.map(|_| ())
            }
        }
    }

    /// Submits my pending review, as an approval when asked; with nothing pending, an approval
    /// alone is still a review.
    pub async fn publish(&self, key: &MrKey, approve: bool) -> Result<()> {
        let prs = self.threads(key).await?;
        let event = if approve { "APPROVE" } else { "COMMENT" };
        match prs.pending_review() {
            Some(review) => {
                let query = "mutation($id: ID!, $event: PullRequestReviewEvent!) { submitPullRequestReview(input: {pullRequestReviewId: $id, event: $event}) { clientMutationId } }";
                self.graphql::<Value>(query, json!({"id": review.id, "event": event})).await.map(|_| ())
            }
            None if approve => self.approve(key, true).await,
            None => bail!("nothing to publish: no pending review on {}#{}", key.project, key.number),
        }
    }

    pub async fn resolve(&self, discussion: &str, resolved: bool) -> Result<()> {
        if !discussion.starts_with("PRRT_") {
            bail!("only review threads resolve on GitHub; this is a comment on the pull request");
        }
        let verb = if resolved { "resolveReviewThread" } else { "unresolveReviewThread" };
        let query = format!("mutation($id: ID!) {{ {verb}(input: {{threadId: $id}}) {{ clientMutationId }} }}");
        self.graphql::<Value>(&query, json!({"id": discussion})).await.map(|_| ())
    }

    async fn open_review(&self, pull_request: &str) -> Result<Comment> {
        let query = format!(
            "mutation($pr: ID!) {{ addPullRequestReview(input: {{pullRequestId: $pr}}) {{ pullRequestReview {{ {COMMENT_FIELDS} }} }} }}"
        );
        let created: Created = self.graphql(&query, json!({"pr": pull_request})).await?;
        Ok(created.add_pull_request_review.context("GitHub answered without the review")?.pull_request_review)
    }

    async fn thread_draft(&self, review: &str, position: &forge::Position, body: &str) -> Result<Draft> {
        let (side, line) = wire::side_of(position.line).context("the line has no number")?;
        let mut input = json!({"pullRequestReviewId": review, "path": position.path(), "line": line, "side": side, "body": body});
        if let Some((start_side, start_line)) = position.start.and_then(wire::side_of) {
            input["startLine"] = json!(start_line);
            input["startSide"] = json!(start_side);
        }
        let query = format!(
            "mutation($input: AddPullRequestReviewThreadInput!) {{ addPullRequestReviewThread(input: $input) {{ thread {{ id isResolved path line originalLine startLine originalStartLine diffSide startDiffSide comments(first: 1) {{ nodes {{ {COMMENT_FIELDS} }} }} }} }} }}"
        );
        let created: Created = self.graphql(&query, json!({"input": input})).await?;
        let thread = created.add_pull_request_review_thread.context("GitHub answered without the thread")?.thread;
        let comment = thread.comments.nodes.first().context("GitHub answered a thread without its comment")?;
        Ok(Draft { id: comment.number(), body: comment.body.clone(), position: Some(position.clone()), reply_to: None, resolve: false })
    }

    async fn reply_draft(&self, review: &str, thread: &str, body: &str) -> Result<Draft> {
        let query = format!(
            "mutation($review: ID!, $thread: ID!, $body: String!) {{ addPullRequestReviewThreadReply(input: {{pullRequestReviewId: $review, pullRequestReviewThreadId: $thread, body: $body}}) {{ comment {{ {COMMENT_FIELDS} }} }} }}"
        );
        let created: Created = self.graphql(&query, json!({"review": review, "thread": thread, "body": body})).await?;
        let comment = created.add_pull_request_review_thread_reply.context("GitHub answered without the reply")?.comment()?;
        Ok(Draft { id: comment.number(), body: comment.body, position: None, reply_to: Some(thread.to_owned()), resolve: false })
    }

    /// The pending review's own text: the draft that sits on the PR itself.
    async fn summary_draft(&self, review: &str, body: &str) -> Result<Draft> {
        let query = format!(
            "mutation($id: ID!, $body: String!) {{ updatePullRequestReview(input: {{pullRequestReviewId: $id, body: $body}}) {{ pullRequestReview {{ {COMMENT_FIELDS} }} }} }}"
        );
        let created: Created = self.graphql(&query, json!({"id": review, "body": body})).await?;
        let review = created.update_pull_request_review.context("GitHub answered without the review")?.pull_request_review;
        Ok(Draft { id: review.number(), body: review.body, position: None, reply_to: None, resolve: false })
    }
}

/// A queue straight from GraphQL answer bodies, for tests.
#[cfg(test)]
pub(super) fn queue_from_json(mine: &str, open: Option<(&str, &str)>) -> Result<Queue> {
    let mine: QueueData = serde_json::from_str::<wire::Answer<QueueData>>(mine)?.into_data()?;
    let Some((body, project)) = open else { return Ok(queue_from(mine, None, vec![])) };
    let open: OpenData = serde_json::from_str::<wire::Answer<OpenData>>(body)?.into_data()?;
    Ok(queue_from(mine, Some(project), prs(open.repository.context("no repository")?.pull_requests)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::auth::Credentials;
    use crate::forge::{LineRef, ReviewState};
    use wiremock::matchers::{body_partial_json, body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn key() -> MrKey {
        MrKey::new("acme/widgets", 42)
    }

    fn client(server: &MockServer) -> Client {
        Client::with_base(&Credentials { host: "github.com".into(), token: "ghp_xxxx".into() }, &server.uri()).unwrap()
    }

    fn threads() -> PrThreads {
        let answer: wire::Answer<ThreadsData> = serde_json::from_str(include_str!("fixtures/threads.json")).unwrap();
        answer.into_data().unwrap().repository.unwrap().pull_request.unwrap()
    }

    fn numbers(mrs: &[QueueMr]) -> Vec<u64> {
        mrs.iter().map(|m| m.number).collect()
    }

    #[test]
    fn a_comment_brings_its_node_id_and_the_shared_reactions_only() {
        let comment: Comment = serde_json::from_value(json!({
            "id": "PRRC_kw1", "fullDatabaseId": "77", "body": "nice", "createdAt": "2026-09-22T10:00:00Z",
            "updatedAt": "2026-09-22T10:00:00Z", "author": {"login": "lea"},
            "reactionGroups": [
                {"content": "THUMBS_UP", "viewerHasReacted": true, "reactors": {"totalCount": 2}},
                {"content": "ROCKET", "viewerHasReacted": false, "reactors": {"totalCount": 0}}
            ]
        }))
        .unwrap();
        let note = comment.note(false, false, None);
        assert_eq!((note.id, note.node.as_deref()), (77, Some("PRRC_kw1")));
        assert_eq!(note.reactions, [forge::Reaction { emoji: forge::Emoji::ThumbsUp, count: 2, mine: true }], "empty groups are left out");
    }

    #[tokio::test]
    async fn react_adds_or_removes_by_node_id_and_content() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("addReaction"))
            .and(body_string_contains("\"content\":\"HOORAY\""))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": {"addReaction": {"clientMutationId": null}}})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("removeReaction"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"errors": [{"message": "Could not resolve to a node"}]})))
            .mount(&server)
            .await;
        let client = client(&server);
        client.react("PRRC_kw1", forge::Emoji::Hooray, true).await.unwrap();
        let err = client.react("PRRC_kw1", forge::Emoji::Hooray, false).await.unwrap_err().to_string();
        assert!(err.contains("Could not resolve"), "{err}");
    }
    #[test]
    fn the_merge_method_prefers_squash_then_a_merge_commit_then_rebase() {
        let repo = |squash, merge, rebase, delete| MrRepository {
            pull_request: None,
            squash_merge_allowed: squash,
            merge_commit_allowed: merge,
            rebase_merge_allowed: rebase,
            delete_branch_on_merge: delete,
        };
        let method = |r: MrRepository| r.merge_plan().method;
        assert_eq!(method(repo(true, true, true, false)), forge::MergeMethod::Squash);
        assert_eq!(method(repo(false, true, true, false)), forge::MergeMethod::Merge);
        assert_eq!(method(repo(false, false, true, false)), forge::MergeMethod::Rebase);
        assert!(repo(true, false, false, true).merge_plan().remove_branch);
    }
    #[test]
    fn the_queue_sorts_searches_into_sections_and_skips_non_prs() {
        let queue = queue_from_json(include_str!("fixtures/queue.json"), None).unwrap();
        let sections = queue.sections(&[]);
        assert_eq!(queue.me, "nina");
        assert_eq!(numbers(&sections.to_review), [42, 40], "asked again after approving counts as asked");
        assert_eq!(numbers(&sections.done), [39]);
        assert_eq!(numbers(&sections.mine), [41]);
        assert_eq!(numbers(&sections.watching), [35]);
        let failing = &sections.to_review[0];
        assert_eq!((failing.pipeline.as_deref(), failing.unresolved, failing.project.as_str()), (Some("FAILED"), 1, "acme/widgets"));
        assert!(sections.done[0].conflicts);
        assert_eq!(sections.done[0].my_state("nina"), Some(ReviewState::Reviewed));
        assert!(sections.mine[0].approved && sections.mine[0].approved_by == ["lea"]);
    }

    #[test]
    fn a_scoped_queue_adds_the_other_open_prs_of_the_repository() {
        let queue =
            queue_from_json(include_str!("fixtures/queue.json"), Some((include_str!("fixtures/open.json"), "acme/widgets"))).unwrap();
        let sections = queue.sections(&[]);
        assert!(sections.open.is_empty());
        assert_eq!(numbers(&sections.drafts), [44], "a draft PR waits apart from the open ones");
        assert!(sections.watching.is_empty(), "acme/infra is out of scope");
    }

    #[test]
    fn searches_are_scoped_with_repo() {
        let scoped = searches(Some("acme/widgets"));
        assert_eq!(scoped["requested"], "is:pr is:open archived:false review-requested:@me repo:acme/widgets");
        assert_eq!(searches(None)["reviewed"], "is:pr is:open archived:false reviewed-by:@me -author:@me");
    }

    #[test]
    fn published_threads_carry_their_side_range_and_resolution_and_loose_notes_follow() {
        let discussions = threads().discussions();
        let ids: Vec<&str> = discussions.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["IC_1", "PRRT_2", "PRR_1", "PRRT_1"], "oldest first; empty reviews and pending-only threads are left out");
        let open = &discussions[3];
        assert_eq!(open.notes.len(), 2, "my pending reply is a draft, not a note");
        let position = open.notes[0].position.as_ref().unwrap();
        assert_eq!(
            (position.path(), position.line, &position.refs.head[..]),
            ("src/pay/charge.rs", LineRef { old: None, new: Some(57) }, "bbbb")
        );
        assert!(open.notes[1].position.is_none());
        let resolved = &discussions[1];
        assert!(resolved.notes[0].resolved);
        let range = resolved.notes[0].position.as_ref().unwrap();
        assert_eq!((range.line, range.start), (LineRef { old: Some(13), new: None }, Some(LineRef { old: Some(11), new: None })));
        assert!(!discussions[0].notes[0].resolvable);
    }

    #[test]
    fn drafts_are_my_pending_comments_and_the_pending_review_text() {
        let drafts = threads().drafts();
        let shape: Vec<(u64, Option<&str>, bool)> = drafts.iter().map(|d| (d.id, d.reply_to.as_deref(), d.position.is_some())).collect();
        assert_eq!(shape, [(103, Some("PRRT_1"), false), (105, None, true), (209, None, false)]);
        assert_eq!(drafts[1].position.as_ref().unwrap().line, LineRef { old: None, new: Some(4) });
    }

    #[test]
    fn draft_ids_name_their_node() {
        let prs = threads();
        assert!(matches!(prs.node_of(105), Some(Target::Comment(id)) if id == "PRRC_5"));
        assert!(matches!(prs.node_of(209), Some(Target::Summary(id)) if id == "PRR_9"));
        assert!(prs.node_of(101).is_none(), "a published comment is no draft");
    }

    #[tokio::test]
    async fn one_mr_reads_approvals_conflicts_and_checks() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("pullRequest(number: $number)"))
            .respond_with(ResponseTemplate::new(200).set_body_string(include_str!("fixtures/mr.json")))
            .mount(&server)
            .await;
        let mr = client(&server).mr(&key()).await.unwrap();
        assert_eq!((mr.project.as_str(), mr.number, mr.state.as_str(), mr.description.as_str()), ("acme/widgets", 42, "opened", ""));
        assert_eq!(mr.refs, Refs { base: "aaaa".into(), start: "aaaa".into(), head: "bbbb".into() });
        assert!(mr.conflicts);
        assert_eq!(mr.pipeline.as_ref().map(|p| p.status.as_str()), Some("success"));
        assert_eq!(mr.approvals.approved_by.iter().map(|u| u.username.as_str()).collect::<Vec<_>>(), ["lea"]);
        assert!(
            !mr.approvals.approved && mr.approvals.approvals_left == 1 && mr.approvals.user_can_approve && !mr.approvals.user_has_approved
        );
        assert_eq!(mr.reviewers.iter().map(|u| u.username.as_str()).collect::<Vec<_>>(), ["nina", "lea"]);
    }

    #[tokio::test]
    async fn graphql_errors_are_errors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data": null, "errors": [{"message": "Could not resolve to a Repository"}]})),
            )
            .mount(&server)
            .await;
        let err = client(&server).mr(&key()).await.unwrap_err().to_string();
        assert!(err.contains("Could not resolve"), "{err}");
    }

    fn threads_mock(body: &str) -> Mock {
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("query Threads"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body.to_owned()))
    }

    fn no_pending_review() -> String {
        let mut value: Value = serde_json::from_str(include_str!("fixtures/threads.json")).unwrap();
        let reviews = value["data"]["repository"]["pullRequest"]["reviews"]["nodes"].as_array_mut().unwrap();
        reviews.retain(|r| r["state"] != "PENDING");
        value.to_string()
    }

    fn comment_json(id: &str, db: u64, body: &str) -> Value {
        json!({"id": id, "fullDatabaseId": db.to_string(), "body": body, "createdAt": "2026-09-22T10:00:00Z", "updatedAt": "2026-09-22T10:00:00Z", "state": "PENDING", "author": {"login": "nina"}})
    }

    #[tokio::test]
    async fn a_line_draft_opens_a_pending_review_first_and_lands_on_its_side() {
        let server = MockServer::start().await;
        threads_mock(&no_pending_review()).mount(&server).await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("addPullRequestReview(input"))
            .and(body_string_contains("PR_42"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data": {"addPullRequestReview": {"pullRequestReview": comment_json("PRR_new", 210, "")}}})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("addPullRequestReviewThread"))
            .and(body_string_contains(r#""side":"LEFT""#))
            .and(body_string_contains(r#""line":13"#))
            .and(body_string_contains(r#""startLine":11"#))
            .and(body_string_contains("PRR_new"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": {"addPullRequestReviewThread": {"thread": {
                "id": "PRRT_9", "isResolved": false, "path": "src/pay/charge.rs", "line": 13, "diffSide": "LEFT",
                "comments": {"nodes": [comment_json("PRRC_9", 109, "why?")]}
            }}}})))
            .expect(1)
            .mount(&server)
            .await;
        let refs = Refs { base: "aaaa".into(), start: "aaaa".into(), head: "bbbb".into() };
        let position = forge::Position {
            refs,
            old_path: "src/pay/charge.rs".into(),
            new_path: "src/pay/charge.rs".into(),
            line: LineRef { old: Some(13), new: None },
            start: Some(LineRef { old: Some(11), new: None }),
        };
        let draft = client(&server)
            .create_draft(&key(), &NewDraft { body: "why?".into(), position: Some(position), ..NewDraft::default() })
            .await
            .unwrap();
        assert_eq!((draft.id, draft.body.as_str()), (109, "why?"));
    }

    #[tokio::test]
    async fn replies_join_the_pending_review_and_the_pr_note_extends_its_text() {
        let server = MockServer::start().await;
        threads_mock(include_str!("fixtures/threads.json")).mount(&server).await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("addPullRequestReviewThreadReply"))
            .and(body_string_contains("PRR_9"))
            .and(body_string_contains("PRRT_1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({"data": {"addPullRequestReviewThreadReply": {"comment": comment_json("PRRC_7", 107, "agreed")}}}),
                ),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("updatePullRequestReview("))
            .and(body_string_contains(r"Two nits.\n\nAlso: docs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"data": {"updatePullRequestReview": {"pullRequestReview": comment_json("PRR_9", 209, "Two nits.\n\nAlso: docs")}}}),
            ))
            .mount(&server)
            .await;
        let client = client(&server);
        let reply = client
            .create_draft(&key(), &NewDraft { body: "agreed".into(), reply_to: Some("PRRT_1".into()), ..NewDraft::default() })
            .await
            .unwrap();
        assert_eq!((reply.id, reply.reply_to.as_deref()), (107, Some("PRRT_1")));
        let summary = client.create_draft(&key(), &NewDraft { body: "Also: docs".into(), ..NewDraft::default() }).await.unwrap();
        assert_eq!(summary.id, 209);
    }

    #[tokio::test]
    async fn drafts_are_edited_and_deleted_by_their_node_and_published_as_one_review() {
        let server = MockServer::start().await;
        threads_mock(include_str!("fixtures/threads.json")).mount(&server).await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("updatePullRequestReviewComment"))
            .and(body_string_contains("PRRC_5"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": {"updatePullRequestReviewComment": {"pullRequestReviewComment": comment_json("PRRC_5", 105, "nit: unused")}}})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("deletePullRequestReviewComment"))
            .and(body_string_contains("PRRC_3"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"data": {"deletePullRequestReviewComment": {"clientMutationId": null}}})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("submitPullRequestReview"))
            .and(body_string_contains(r#""event":"APPROVE""#))
            .and(body_string_contains("PRR_9"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"data": {"submitPullRequestReview": {"clientMutationId": null}}})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let client = client(&server);
        let edited = client.update_draft(&key(), 105, &NewDraft { body: "nit: unused".into(), ..NewDraft::default() }).await.unwrap();
        assert_eq!(edited.body, "nit: unused");
        client.delete_draft(&key(), 103).await.unwrap();
        client.publish(&key(), true).await.unwrap();
        let err = client.delete_draft(&key(), 101).await.unwrap_err().to_string();
        assert!(err.contains("not in my pending review"), "{err}");
    }

    #[tokio::test]
    async fn threads_resolve_and_loose_comments_say_they_cannot() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("unresolveReviewThread"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": {"unresolveReviewThread": {"clientMutationId": null}}})))
            .expect(1)
            .mount(&server)
            .await;
        let client = client(&server);
        client.resolve("PRRT_1", false).await.unwrap();
        assert!(client.resolve("IC_1", true).await.unwrap_err().to_string().contains("only review threads"));
    }

    fn draft_state_mock(is_draft: bool) -> Mock {
        Mock::given(method("POST")).and(path("/graphql")).and(body_string_contains("query DraftState")).respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data": {"repository": {"pullRequest": {"id": "PR_42", "isDraft": is_draft}}}})),
        )
    }

    #[tokio::test]
    async fn set_draft_converts_a_ready_pr_by_its_node_id() {
        let server = MockServer::start().await;
        draft_state_mock(false).mount(&server).await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("convertPullRequestToDraft"))
            .and(body_partial_json(json!({"variables": {"id": "PR_42"}})))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"data": {"convertPullRequestToDraft": {"clientMutationId": null}}})),
            )
            .expect(1)
            .mount(&server)
            .await;
        client(&server).set_draft(&key(), true).await.unwrap();
    }

    #[tokio::test]
    async fn set_draft_leaves_a_pr_already_in_that_state_alone() {
        let server = MockServer::start().await;
        draft_state_mock(false).mount(&server).await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("mutation"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        client(&server).set_draft(&key(), false).await.unwrap();
    }
}
