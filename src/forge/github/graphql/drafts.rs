//! My pending review: the drafts it holds, written and published, and replies and resolution.
use super::threads::{Comment, Target, Thread};
use crate::forge::github::{Client, wire};
use crate::forge::{self, Draft, MrKey, NewDraft, ThreadId};
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::{Value, json};

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

/// The pending review's text with `note` last: every draft on the PR itself shares it, so they
/// read back as one. A retry that already put `note` last changes nothing.
fn with_note(text: &str, note: &str) -> String {
    let text = text.trim();
    if text == note || text.ends_with(&format!("\n\n{note}")) {
        return text.to_owned();
    }
    [text, note].iter().filter(|t| !t.is_empty()).copied().collect::<Vec<_>>().join("\n\n")
}

const COMMENT_FIELDS: &str = "id fullDatabaseId body createdAt updatedAt state author { login ... on User { name databaseId } }";

impl Client {
    /// Adds the draft to my pending review, opening one first when there is none.
    pub(crate) async fn create_draft(&self, key: &MrKey, draft: &NewDraft) -> Result<Draft> {
        let prs = self.threads(key).await?;
        let review = match prs.pending_review() {
            Some(review) => review.clone(),
            None => self.open_review(&prs.id).await?,
        };
        match (&draft.position, &draft.reply_to) {
            (_, Some(thread)) => self.reply_draft(&review.id, thread, &draft.body).await,
            (Some(position), None) => self.thread_draft(&review.id, position, &draft.body).await,
            (None, None) => self.summary_draft(&review.id, &with_note(&review.body, &draft.body)).await,
        }
    }

    /// GitHub keeps a comment where it was written: only the text changes.
    pub(crate) async fn update_draft(&self, key: &MrKey, id: u64, draft: &NewDraft) -> Result<Draft> {
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

    pub(crate) async fn delete_draft(&self, key: &MrKey, id: u64) -> Result<()> {
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
    pub(crate) async fn publish(&self, key: &MrKey, kind: forge::ReviewKind) -> Result<forge::Approval> {
        let prs = self.threads(key).await?;
        let (event, approval) = match kind {
            forge::ReviewKind::Approve => ("APPROVE", forge::Approval::Given),
            forge::ReviewKind::Comment => ("COMMENT", forge::Approval::NotAsked),
        };
        let Some(review) = prs.pending_review() else {
            ensure!(kind == forge::ReviewKind::Approve, "nothing to publish: no pending review on {}#{}", key.project, key.number);
            self.approve(key, forge::ApprovalChange::Approve).await?;
            return Ok(forge::Approval::Given);
        };
        let query = "mutation($id: ID!, $event: PullRequestReviewEvent!) { submitPullRequestReview(input: {pullRequestReviewId: $id, event: $event}) { clientMutationId } }";
        self.graphql::<Value>(query, json!({"id": review.id, "event": event})).await?;
        Ok(approval)
    }

    pub(crate) async fn resolve(&self, discussion: &ThreadId, resolved: bool) -> Result<()> {
        if !discussion.as_str().starts_with("PRRT_") {
            bail!("only review threads resolve on GitHub; this is a comment on the pull request");
        }
        let verb = if resolved { "resolveReviewThread" } else { "unresolveReviewThread" };
        let query = format!("mutation($id: ID!) {{ {verb}(input: {{threadId: $id}}) {{ clientMutationId }} }}");
        self.graphql::<Value>(&query, json!({"id": discussion})).await.map(|_| ())
    }

    /// A public reply: on a review thread, in it; GitHub has no thread for any other comment, so
    /// the reply is a new comment on the PR.
    pub(crate) async fn reply(&self, key: &MrKey, thread: &ThreadId, body: &str) -> Result<()> {
        if !thread.as_str().starts_with("PRRT_") {
            return self.comment(key, body, None).await.map(|_| ());
        }
        let query = "mutation($thread: ID!, $body: String!) { addPullRequestReviewThreadReply(input: {pullRequestReviewThreadId: $thread, body: $body}) { clientMutationId } }";
        self.graphql::<Value>(query, json!({"thread": thread, "body": body})).await.map(|_| ())
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

    async fn reply_draft(&self, review: &str, thread: &ThreadId, body: &str) -> Result<Draft> {
        let query = format!(
            "mutation($review: ID!, $thread: ID!, $body: String!) {{ addPullRequestReviewThreadReply(input: {{pullRequestReviewId: $review, pullRequestReviewThreadId: $thread, body: $body}}) {{ comment {{ {COMMENT_FIELDS} }} }} }}"
        );
        let created: Created = self.graphql(&query, json!({"review": review, "thread": thread, "body": body})).await?;
        let comment = created.add_pull_request_review_thread_reply.context("GitHub answered without the reply")?.comment()?;
        Ok(Draft { id: comment.number(), body: comment.body, position: None, reply_to: Some(thread.clone()), resolve: false })
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

#[cfg(test)]
mod tests {
    use super::super::test_support::{client, comment_json, key, threads_json};
    use super::*;
    use crate::forge::{LineRef, Refs};
    use wiremock::matchers::{body_partial_json, body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn threads_mock(body: &str) -> Mock {
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("query Threads"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body.to_owned()))
    }

    fn no_pending_review() -> String {
        let mut value = threads_json();
        let reviews = value["data"]["repository"]["pullRequest"]["reviews"]["nodes"].as_array_mut().unwrap();
        reviews.retain(|r| r["state"] != "PENDING");
        value.to_string()
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
        threads_mock(include_str!("../fixtures/threads.json")).mount(&server).await;
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
        assert_eq!((reply.id, reply.reply_to), (107, Some("PRRT_1".into())));
        let summary = client.create_draft(&key(), &NewDraft { body: "Also: docs".into(), ..NewDraft::default() }).await.unwrap();
        assert_eq!(summary.id, 209);
    }

    fn with_review_text(body: &str) -> String {
        let mut value = threads_json();
        let reviews = value["data"]["repository"]["pullRequest"]["reviews"]["nodes"].as_array_mut().unwrap();
        reviews.iter_mut().filter(|r| r["state"] == "PENDING").for_each(|r| r["body"] = json!(body));
        value.to_string()
    }

    #[tokio::test]
    async fn a_pr_note_retried_after_a_lost_answer_is_not_added_twice() {
        let server = MockServer::start().await;
        threads_mock(&with_review_text("Two nits.\n\nAlso: docs")).mount(&server).await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("updatePullRequestReview("))
            .and(body_partial_json(json!({"variables": {"body": "Two nits.\n\nAlso: docs"}})))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"data": {"updatePullRequestReview": {"pullRequestReview": comment_json("PRR_9", 209, "Two nits.\n\nAlso: docs")}}}),
            ))
            .expect(1)
            .mount(&server)
            .await;
        let saved = client(&server).create_draft(&key(), &NewDraft { body: "Also: docs".into(), ..NewDraft::default() }).await.unwrap();
        assert_eq!((saved.id, saved.body.as_str()), (209, "Two nits.\n\nAlso: docs"), "the whole text comes back as the one draft");
    }

    #[tokio::test]
    async fn the_pr_draft_is_rewritten_whole_and_a_second_run_changes_nothing() {
        let server = MockServer::start().await;
        threads_mock(include_str!("../fixtures/threads.json")).mount(&server).await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("updatePullRequestReview("))
            .and(body_partial_json(json!({"variables": {"id": "PRR_9", "body": "Two nits.\n\nAlso: tests"}})))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"data": {"updatePullRequestReview": {"pullRequestReview": comment_json("PRR_9", 209, "Two nits.\n\nAlso: tests")}}}),
            ))
            .expect(2)
            .mount(&server)
            .await;
        let client = client(&server);
        let edit = NewDraft { body: "Two nits.\n\nAlso: tests".into(), ..NewDraft::default() };
        let first = client.update_draft(&key(), 209, &edit).await.unwrap();
        let again = client.update_draft(&key(), 209, &edit).await.unwrap();
        assert_eq!(first, again, "a second run changes nothing");
    }

    #[tokio::test]
    async fn drafts_are_edited_and_deleted_by_their_node_and_published_as_one_review() {
        let server = MockServer::start().await;
        threads_mock(include_str!("../fixtures/threads.json")).mount(&server).await;
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
        assert_eq!(
            client.publish(&key(), forge::ReviewKind::Approve).await.unwrap(),
            forge::Approval::Given,
            "one review carries the notes and the approval"
        );
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
        client.resolve(&"PRRT_1".into(), false).await.unwrap();
        assert!(client.resolve(&"IC_1".into(), true).await.unwrap_err().to_string().contains("only review threads"));
    }

    #[tokio::test]
    async fn a_reply_goes_in_its_review_thread_and_on_the_pr_for_a_loose_comment() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("addPullRequestReviewThreadReply"))
            .and(body_partial_json(json!({"variables": {"thread": "PRRT_1", "body": "agreed"}})))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"data": {"addPullRequestReviewThreadReply": {"clientMutationId": null}}})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/repos/acme/widgets/issues/42/comments"))
            .and(body_partial_json(json!({"body": "agreed"})))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "id": 8, "node_id": "IC_8", "body": "agreed", "user": {"id": 2, "login": "nina"},
                "created_at": "2026-09-22T10:00:00Z", "updated_at": "2026-09-22T10:00:00Z"
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = client(&server);
        client.reply(&key(), &"PRRT_1".into(), "agreed").await.unwrap();
        client.reply(&key(), &"IC_1".into(), "agreed").await.unwrap();
    }
}
