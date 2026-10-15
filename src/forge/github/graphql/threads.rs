//! A PR's threads, reviews and comments, read page after page, as discussions and drafts.
use crate::forge::github::wire::{Actor, Anchor, user_of};
use crate::forge::github::{Client, owner_and_name};
use crate::forge::{self, Discussion, Draft, MrKey, Note, Refs, Sha, ThreadId};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;

const THREADS: &str = r"
query Threads($owner: String!, $name: String!, $number: Int!, $threads: String, $reviews: String, $comments: String) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      id baseRefOid headRefOid
      reviewThreads(first: 100, after: $threads) {
        nodes {
          id isResolved path line originalLine startLine originalStartLine diffSide startDiffSide
          comments(first: 100) { nodes { ...comment state } pageInfo { ...page } }
        }
        pageInfo { ...page }
      }
      reviews(first: 100, after: $reviews) { nodes { ...comment state } pageInfo { ...page } }
      comments(first: 100, after: $comments) { nodes { ...comment } pageInfo { ...page } }
    }
  }
}";

const THREAD_COMMENTS: &str = r"
query ThreadComments($id: ID!, $after: String) {
  node(id: $id) { ... on PullRequestReviewThread { comments(first: 100, after: $after) { nodes { ...comment state } pageInfo { ...page } } } }
}";

const COMMENT_FRAGMENTS: &str = r"
fragment page on PageInfo { hasNextPage endCursor }
fragment comment on Comment {
  id body createdAt updatedAt author { login ... on User { name databaseId } }
  ... on Reactable { reactionGroups { content viewerHasReacted reactors { totalCount } } }
  ... on PullRequestReviewComment { fullDatabaseId }
  ... on PullRequestReview { fullDatabaseId }
  ... on IssueComment { fullDatabaseId }
}";

/// Enough for any PR a person reviews; the bound only stops a server that never ends a list.
const MAX_PAGES: usize = 50;

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
    pub(super) id: String,
    base_ref_oid: Sha,
    head_ref_oid: Sha,
    review_threads: Page<Thread>,
    reviews: Page<Comment>,
    comments: Page<Comment>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Thread {
    id: ThreadId,
    is_resolved: bool,
    #[serde(flatten)]
    anchor: Anchor,
    pub(super) comments: Page<Comment>,
}

/// One page of a GraphQL list; an answer without `pageInfo` holds the whole list.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Page<T> {
    pub(super) nodes: Vec<T>,
    #[serde(default)]
    page_info: PageInfo,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

impl<T> Page<T> {
    /// The cursor the next page starts after, when there is a next page.
    fn next(&self) -> Option<&str> {
        self.page_info.end_cursor.as_deref().filter(|_| self.page_info.has_next_page)
    }

    /// An empty page past the end keeps the cursor where it was: a null one would start the list over.
    fn followed_by(self, next: Page<T>) -> Page<T> {
        let end_cursor = next.page_info.end_cursor.or(self.page_info.end_cursor);
        Page {
            nodes: self.nodes.into_iter().chain(next.nodes).collect(),
            page_info: PageInfo { has_next_page: next.page_info.has_next_page, end_cursor },
        }
    }
}

/// The cursor each list of a PR resumes after; none starts it from the top.
#[derive(Default)]
struct After {
    threads: Option<String>,
    reviews: Option<String>,
    comments: Option<String>,
}

#[derive(Deserialize)]
struct ThreadCommentsData {
    node: Option<ThreadComments>,
}

#[derive(Deserialize)]
struct ThreadComments {
    comments: Page<Comment>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Comment {
    pub(super) id: String,
    #[serde(default)]
    full_database_id: Option<String>,
    pub(super) body: String,
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

    pub(super) fn number(&self) -> u64 {
        self.full_database_id.as_deref().and_then(|id| id.parse().ok()).unwrap_or_default()
    }

    /// A note in a review thread, `position` on its first one.
    fn thread_note(&self, resolved: bool, position: Option<forge::Position>) -> Note {
        Note { resolvable: true, resolved, position, ..self.loose_note() }
    }

    /// A note outside any review thread: a review summary or a PR comment.
    fn loose_note(&self) -> Note {
        Note {
            id: self.number(),
            body: self.body.clone(),
            author: user_of(self.author.clone()),
            created_at: self.created_at,
            updated_at: self.updated_at,
            system: false,
            resolvable: false,
            resolved: false,
            position: None,
            suggestions: vec![],
            reactions: self
                .reaction_groups
                .iter()
                .filter(|g| g.reactors.total_count > 0)
                .filter_map(|g| {
                    let emoji = forge::Emoji::from_github(&g.content)?;
                    Some(forge::Reaction { emoji, count: g.reactors.total_count, mine: g.viewer_has_reacted })
                })
                .collect(),
            node: Some(self.id.clone()),
        }
    }
}

impl PrThreads {
    /// Where each list picks up, while one of them has more; a list read whole answers an empty page.
    fn resume(&self) -> Option<After> {
        let lists = [&self.review_threads.page_info, &self.reviews.page_info, &self.comments.page_info];
        lists.iter().any(|p| p.has_next_page).then(|| After {
            threads: self.review_threads.page_info.end_cursor.clone(),
            reviews: self.reviews.page_info.end_cursor.clone(),
            comments: self.comments.page_info.end_cursor.clone(),
        })
    }

    fn followed_by(self, next: PrThreads) -> PrThreads {
        PrThreads {
            review_threads: self.review_threads.followed_by(next.review_threads),
            reviews: self.reviews.followed_by(next.reviews),
            comments: self.comments.followed_by(next.comments),
            ..self
        }
    }

    fn refs(&self) -> Refs {
        Refs { base: self.base_ref_oid.clone(), start: self.base_ref_oid.clone(), head: self.head_ref_oid.clone() }
    }

    /// My pending review, which holds every draft: its node id and its database id.
    pub(super) fn pending_review(&self) -> Option<&Comment> {
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
                .map(|(i, c)| c.thread_note(thread.is_resolved, if i == 0 { position.clone() } else { None }))
                .collect();
            (!notes.is_empty()).then(|| Discussion { id: thread.id.clone(), notes })
        });
        let summaries = self.reviews.nodes.iter().filter(|r| !r.pending() && !r.body.trim().is_empty());
        let loose = summaries.chain(&self.comments.nodes).map(|c| Discussion { id: c.id.as_str().into(), notes: vec![c.loose_note()] });
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
    pub(super) fn node_of(&self, id: u64) -> Option<Target> {
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
pub(super) enum Target {
    Comment(String),
    Summary(String),
}

impl Client {
    /// Every thread, review and comment of the PR, page after page.
    pub(super) async fn threads(&self, key: &MrKey) -> Result<PrThreads> {
        let mut prs = self.threads_page(key, After::default()).await?;
        for _ in 0..MAX_PAGES {
            let Some(resume) = prs.resume() else { break };
            prs = prs.followed_by(self.threads_page(key, resume).await?);
        }
        let threads = futures_util::future::try_join_all(prs.review_threads.nodes.into_iter().map(|t| self.whole_thread(t))).await?;
        Ok(PrThreads { review_threads: Page { nodes: threads, page_info: PageInfo::default() }, ..prs })
    }

    async fn threads_page(&self, key: &MrKey, after: After) -> Result<PrThreads> {
        let (owner, name) = owner_and_name(&key.project)?;
        let variables = json!({
            "owner": owner, "name": name, "number": key.number,
            "threads": after.threads, "reviews": after.reviews, "comments": after.comments,
        });
        let data: ThreadsData = self.graphql(&format!("{THREADS}{COMMENT_FRAGMENTS}"), variables).await?;
        data.repository.and_then(|r| r.pull_request).with_context(|| format!("{}#{} not found", key.project, key.number))
    }

    /// A thread with the comments past its first page.
    async fn whole_thread(&self, thread: Thread) -> Result<Thread> {
        let query = format!("{THREAD_COMMENTS}{COMMENT_FRAGMENTS}");
        let mut comments = thread.comments;
        for _ in 0..MAX_PAGES {
            let Some(after) = comments.next().map(str::to_owned) else { break };
            let data: ThreadCommentsData = self.graphql(&query, json!({"id": thread.id, "after": after})).await?;
            comments = comments.followed_by(data.node.with_context(|| format!("thread {} is gone", thread.id))?.comments);
        }
        Ok(Thread { comments, ..thread })
    }

    pub(crate) async fn discussions(&self, key: &MrKey) -> Result<Vec<Discussion>> {
        Ok(self.threads(key).await?.discussions())
    }

    pub(crate) async fn drafts(&self, key: &MrKey) -> Result<Vec<Draft>> {
        Ok(self.threads(key).await?.drafts())
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{client, comment_json, key, threads_json};
    use super::*;
    use crate::forge::LineRef;
    use crate::forge::github::wire;
    use serde_json::Value;
    use wiremock::matchers::{body_partial_json, body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn threads() -> PrThreads {
        let answer: wire::Answer<ThreadsData> = serde_json::from_str(include_str!("../fixtures/threads.json")).unwrap();
        answer.into_data().unwrap().repository.unwrap().pull_request.unwrap()
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
        let note = comment.loose_note();
        assert_eq!((note.id, note.node.as_deref()), (77, Some("PRRC_kw1")));
        assert_eq!(note.reactions, [forge::Reaction { emoji: forge::Emoji::ThumbsUp, count: 2, mine: true }], "empty groups are left out");
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
            (position.path(), position.line, position.refs.head.as_str()),
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
        let shape: Vec<(u64, Option<ThreadId>, bool)> = drafts.iter().map(|d| (d.id, d.reply_to.clone(), d.position.is_some())).collect();
        assert_eq!(shape, [(103, Some("PRRT_1".into()), false), (105, None, true), (209, None, false)]);
        assert_eq!(drafts[1].position.as_ref().unwrap().line, LineRef { old: None, new: Some(4) });
    }

    #[test]
    fn draft_ids_name_their_node() {
        let prs = threads();
        assert!(matches!(prs.node_of(105), Some(Target::Comment(id)) if id == "PRRC_5"));
        assert!(matches!(prs.node_of(209), Some(Target::Summary(id)) if id == "PRR_9"));
        assert!(prs.node_of(101).is_none(), "a published comment is no draft");
    }

    fn page_info(next: Option<&str>) -> Value {
        json!({"hasNextPage": next.is_some(), "endCursor": next})
    }

    #[tokio::test]
    async fn threads_reviews_comments_and_long_threads_are_read_past_the_first_page() {
        let server = MockServer::start().await;
        let mut first = threads_json();
        let pr = &mut first["data"]["repository"]["pullRequest"];
        pr["reviewThreads"]["pageInfo"] = page_info(Some("t1"));
        pr["reviewThreads"]["nodes"][0]["comments"]["pageInfo"] = page_info(Some("r1"));
        pr["reviews"]["pageInfo"] = page_info(None);
        pr["comments"]["pageInfo"] = page_info(Some("c1"));
        let thread = json!({"id": "PRRT_4", "isResolved": false, "path": "src/pay/mod.rs", "line": 9, "diffSide": "RIGHT",
            "comments": {"nodes": [comment_json("PRRC_8", 108, "and here")], "pageInfo": page_info(None)}});
        let loose = json!({"id": "IC_2", "fullDatabaseId": "302", "body": "ping", "createdAt": "2026-09-23T10:00:00Z", "updatedAt": "2026-09-23T10:00:00Z", "author": {"login": "lea"}});
        let second = json!({"data": {"repository": {"pullRequest": {"id": "PR_42", "baseRefOid": "aaaa", "headRefOid": "bbbb",
            "reviewThreads": {"nodes": [thread], "pageInfo": page_info(None)},
            "reviews": {"nodes": [], "pageInfo": page_info(None)},
            "comments": {"nodes": [loose], "pageInfo": page_info(None)}}}}});
        let page = |after: Value, answer: Value| {
            Mock::given(method("POST"))
                .and(path("/graphql"))
                .and(body_string_contains("query Threads"))
                .and(body_partial_json(json!({"variables": after})))
                .respond_with(ResponseTemplate::new(200).set_body_json(answer))
                .expect(1)
        };
        page(json!({"threads": null}), first).mount(&server).await;
        page(json!({"threads": "t1", "comments": "c1"}), second).mount(&server).await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("query ThreadComments"))
            .and(body_partial_json(json!({"variables": {"id": "PRRT_1", "after": "r1"}})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": {"node": {"comments": {
                "nodes": [comment_json("PRRC_6", 106, "and a doc line")], "pageInfo": page_info(None)
            }}}})))
            .expect(1)
            .mount(&server)
            .await;
        let prs = client(&server).threads(&key()).await.unwrap();
        let drafts: Vec<u64> = prs.drafts().iter().map(|d| d.id).collect();
        assert_eq!(drafts, [103, 106, 105, 108, 209]);
        assert!(prs.discussions().iter().any(|d| d.id == "IC_2".into()));
    }
}
