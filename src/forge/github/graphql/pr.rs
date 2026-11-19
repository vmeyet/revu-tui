//! One PR read whole, and the calls that change it: readiness and reactions.
use crate::forge::github::wire::{self, Actor, Label, Nodes, Review, ReviewRequest, user_of};
use crate::forge::github::{Client, owner_and_name};
use crate::forge::http::Missing;
use crate::forge::{self, Approvals, MrKey, Pipeline, ProjectPath, Refs, Sha};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

const MR: &str = r"
query Mr($owner: String!, $name: String!, $number: Int!) {
  viewer { login }
  repository(owner: $owner, name: $name) {
    squashMergeAllowed mergeCommitAllowed rebaseMergeAllowed deleteBranchOnMerge viewerPermission
    pullRequest(number: $number) {
      number title body state isDraft url updatedAt baseRefName headRefName baseRefOid headRefOid changedFiles mergeable mergeStateStatus
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
    #[serde(default)]
    viewer_permission: Option<String>,
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
    base_ref_oid: Sha,
    head_ref_oid: Sha,
    changed_files: u32,
    #[serde(default)]
    mergeable: Option<String>,
    #[serde(default)]
    merge_state_status: Option<String>,
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
    /// GitHub's verdict on merging this PR, for me. `mergeStateStatus` leaves out my right to push to the
    /// repository, so `permission` decides first; GitHub may leave it out, and then revu cannot tell.
    fn merge_verdict(&self, permission: Option<&str>) -> forge::MergeVerdict {
        if self.state != "OPEN" {
            return forge::MergeVerdict::NotOpen;
        }
        match permission {
            Some("WRITE" | "MAINTAIN" | "ADMIN") => self.merge_state_verdict(),
            None => forge::MergeVerdict::Unknown,
            Some(_) => forge::MergeVerdict::NotAllowed,
        }
    }

    /// `mergeStateStatus` as a verdict; a status missing, or newer than this list, is unknown.
    fn merge_state_verdict(&self) -> forge::MergeVerdict {
        match self.merge_state_status.as_deref().unwrap_or_default() {
            "CLEAN" | "HAS_HOOKS" | "UNSTABLE" => forge::MergeVerdict::Mergeable,
            "UNKNOWN" => forge::MergeVerdict::Checking,
            "DIRTY" => forge::MergeVerdict::Conflicts,
            "BEHIND" => forge::MergeVerdict::Behind,
            "DRAFT" => forge::MergeVerdict::Draft,
            "BLOCKED" => self.blocked_by(),
            _ => forge::MergeVerdict::Unknown,
        }
    }

    /// GitHub's `BLOCKED` names no reason: the review decision, then the checks, say which one shows.
    /// Once approved, failing checks may not be the ones the branch requires, so the rule stays unnamed.
    fn blocked_by(&self) -> forge::MergeVerdict {
        match (self.review_decision.as_deref(), wire::pipeline(&self.commits)) {
            (Some("CHANGES_REQUESTED"), _) => forge::MergeVerdict::ChangesRequested,
            (Some("REVIEW_REQUIRED"), _) => forge::MergeVerdict::NotApproved,
            (Some("APPROVED"), _) => forge::MergeVerdict::Blocked,
            (_, Some(forge::PipelineStatus::Failed)) => forge::MergeVerdict::PipelineNotPassed,
            (_, Some(forge::PipelineStatus::Running)) => forge::MergeVerdict::PipelineRunning,
            _ => forge::MergeVerdict::Blocked,
        }
    }

    fn into_model(self, project: &ProjectPath, me: &str) -> forge::Mr {
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
        let pipeline = wire::pipeline(&self.commits).map(|status| Pipeline { status, web_url: Some(format!("{}/checks", self.url)) });
        forge::Mr {
            project: project.clone(),
            number: self.number,
            title: self.title,
            description: self.body.unwrap_or_default(),
            state: match self.state.as_str() {
                "OPEN" => forge::MrState::Open,
                "MERGED" => forge::MrState::Merged,
                _ => forge::MrState::Closed,
            },
            draft: self.is_draft,
            mine: author.username == me,
            uncommitted: 0,
            no_diff: None,
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
            merge_verdict: forge::MergeVerdict::Unknown,
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

impl Client {
    pub(crate) async fn mr(&self, key: &MrKey) -> Result<forge::Mr> {
        let (owner, name) = owner_and_name(&key.project)?;
        let data: MrData = self.graphql(MR, json!({"owner": owner, "name": name, "number": key.number})).await?;
        let repository = data.repository.ok_or_else(|| Missing(format!("{} not found", key.project)))?;
        let merge = repository.merge_plan();
        let pr = repository.pull_request.ok_or_else(|| Missing(format!("{}#{} not found", key.project, key.number)))?;
        let merge_verdict = pr.merge_verdict(repository.viewer_permission.as_deref());
        Ok(forge::Mr { merge, merge_verdict, ..pr.into_model(&key.project, &data.viewer.login) })
    }

    /// Adds my `emoji` to the comment `node`, or takes it off; GitHub needs no reaction id for either.
    pub(crate) async fn react(&self, node: &str, emoji: forge::Emoji, change: forge::ReactionChange) -> Result<()> {
        let Some(content) = emoji.github() else { bail!("GitHub reacts with eight emoji only") };
        let mutation = match change {
            forge::ReactionChange::Add => "addReaction",
            forge::ReactionChange::Remove => "removeReaction",
        };
        let query = format!(
            "mutation($id: ID!, $content: ReactionContent!) {{ {mutation}(input: {{subjectId: $id, content: $content}}) {{ clientMutationId }} }}"
        );
        self.graphql::<Value>(&query, json!({"id": node, "content": content})).await.map(|_| ())
    }

    /// Marks the PR a draft, or ready for review; asked first, so a PR already there is left alone.
    pub(crate) async fn set_readiness(&self, key: &MrKey, readiness: forge::Readiness) -> Result<()> {
        let draft = readiness == forge::Readiness::Draft;
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
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{client, key};
    use super::*;
    use wiremock::matchers::{body_partial_json, body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

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
        client.react("PRRC_kw1", forge::Emoji::Hooray, forge::ReactionChange::Add).await.unwrap();
        let other =
            client.react("PRRC_kw1", forge::Emoji::from_gitlab("100").unwrap(), forge::ReactionChange::Add).await.unwrap_err().to_string();
        assert!(other.contains("eight"), "GitHub is not asked: {other}");
        let err = client.react("PRRC_kw1", forge::Emoji::Hooray, forge::ReactionChange::Remove).await.unwrap_err().to_string();
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
            viewer_permission: None,
        };
        let method = |r: MrRepository| r.merge_plan().method;
        assert_eq!(method(repo(true, true, true, false)), forge::MergeMethod::Squash);
        assert_eq!(method(repo(false, true, true, false)), forge::MergeMethod::Merge);
        assert_eq!(method(repo(false, false, true, false)), forge::MergeMethod::Rebase);
        assert!(repo(true, false, false, true).merge_plan().remove_branch);
    }

    #[tokio::test]
    async fn one_mr_reads_approvals_conflicts_and_checks() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("pullRequest(number: $number)"))
            .respond_with(ResponseTemplate::new(200).set_body_string(include_str!("../fixtures/mr.json")))
            .mount(&server)
            .await;
        let mr = client(&server).mr(&key()).await.unwrap();
        assert_eq!((mr.project.as_str(), mr.number, mr.state, mr.description.as_str()), ("acme/widgets", 42, forge::MrState::Open, ""));
        assert_eq!(mr.refs, Refs { base: "aaaa".into(), start: "aaaa".into(), head: "bbbb".into() });
        assert!(mr.conflicts);
        assert_eq!(mr.pipeline.as_ref().map(|p| p.status), Some(forge::PipelineStatus::Success));
        assert_eq!(mr.approvals.approved_by.iter().map(|u| u.username.as_str()).collect::<Vec<_>>(), ["lea"]);
        assert!(
            !mr.approvals.approved && mr.approvals.approvals_left == 1 && mr.approvals.user_can_approve && !mr.approvals.user_has_approved
        );
        assert_eq!(mr.reviewers.iter().map(|u| u.username.as_str()).collect::<Vec<_>>(), ["nina", "lea"]);
    }

    /// My own PR with green checks and no review asked, as GitHub answers it with `fields` on top, me holding `permission`.
    async fn my_pr(fields: Value, permission: &str) -> forge::Mr {
        let mut answer: Value = serde_json::from_str(include_str!("../fixtures/mr.json")).unwrap();
        answer["data"]["viewer"]["login"] = json!("omar");
        let repository = &mut answer["data"]["repository"];
        repository["viewerPermission"] = json!(permission);
        let pr = repository["pullRequest"].as_object_mut().unwrap();
        pr.extend(json!({"mergeable": "MERGEABLE", "reviewDecision": null, "latestReviews": {"nodes": []}}).as_object().unwrap().clone());
        pr.extend(fields.as_object().unwrap().clone());
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(answer))
            .mount(&server)
            .await;
        client(&server).mr(&key()).await.unwrap()
    }

    #[tokio::test]
    async fn the_merge_state_and_my_rights_github_answers_read_as_a_verdict() {
        let mr = my_pr(json!({"mergeStateStatus": "CLEAN"}), "TRIAGE").await;
        assert_eq!(mr.merge_verdict, forge::MergeVerdict::NotAllowed);
    }

    /// An open PR with green checks and no review asked, as GitHub answers it with `fields` on top.
    fn pr(fields: &Value) -> Pr {
        let answer: Value = serde_json::from_str(include_str!("../fixtures/mr.json")).unwrap();
        let mut pr = answer["data"]["repository"]["pullRequest"].clone();
        let object = pr.as_object_mut().unwrap();
        object
            .extend(json!({"mergeable": "MERGEABLE", "reviewDecision": null, "latestReviews": {"nodes": []}}).as_object().unwrap().clone());
        object.extend(fields.as_object().unwrap().clone());
        serde_json::from_value(pr).unwrap()
    }

    fn verdict(fields: &Value) -> forge::MergeVerdict {
        pr(fields).merge_verdict(Some("WRITE"))
    }

    fn checks(state: &str) -> Value {
        json!({"nodes": [{"commit": {"statusCheckRollup": {"state": state}}}]})
    }

    #[test]
    fn each_merge_state_of_github_reads_as_its_verdict() {
        let cases: [(&str, forge::MergeVerdict); 8] = [
            ("CLEAN", forge::MergeVerdict::Mergeable),
            ("HAS_HOOKS", forge::MergeVerdict::Mergeable),
            ("UNSTABLE", forge::MergeVerdict::Mergeable),
            ("UNKNOWN", forge::MergeVerdict::Checking),
            ("DIRTY", forge::MergeVerdict::Conflicts),
            ("BEHIND", forge::MergeVerdict::Behind),
            ("DRAFT", forge::MergeVerdict::Draft),
            ("BLOCKED", forge::MergeVerdict::Blocked),
        ];
        for (state, expected) in cases {
            assert_eq!(verdict(&json!({"mergeStateStatus": state})), expected, "{state}");
        }
    }

    #[test]
    fn a_merge_state_github_adds_later_is_unknown() {
        assert_eq!(verdict(&json!({"mergeStateStatus": "QUEUED"})), forge::MergeVerdict::Unknown);
    }

    #[test]
    fn a_pr_github_answers_without_a_merge_state_is_unknown() {
        assert_eq!(verdict(&json!({"mergeStateStatus": null})), forge::MergeVerdict::Unknown);
    }

    #[test]
    fn a_closed_or_merged_pr_is_not_open_whatever_its_merge_state_or_my_rights() {
        for state in ["CLOSED", "MERGED"] {
            let pr = pr(&json!({"state": state, "mergeStateStatus": "CLEAN"}));
            assert_eq!(pr.merge_verdict(Some("WRITE")), forge::MergeVerdict::NotOpen, "{state}");
            assert_eq!(pr.merge_verdict(Some("READ")), forge::MergeVerdict::NotOpen, "{state}");
        }
    }

    #[test]
    fn a_blocked_pr_names_the_review_answer_behind_it() {
        let cases = [("CHANGES_REQUESTED", forge::MergeVerdict::ChangesRequested), ("REVIEW_REQUIRED", forge::MergeVerdict::NotApproved)];
        for (decision, expected) in cases {
            assert_eq!(
                verdict(&json!({"mergeStateStatus": "BLOCKED", "reviewDecision": decision, "commits": checks("FAILURE")})),
                expected
            );
        }
    }

    #[test]
    fn a_blocked_pr_with_no_review_asked_names_its_checks() {
        let cases = [("FAILURE", forge::MergeVerdict::PipelineNotPassed), ("PENDING", forge::MergeVerdict::PipelineRunning)];
        for (state, expected) in cases {
            assert_eq!(verdict(&json!({"mergeStateStatus": "BLOCKED", "commits": checks(state)})), expected, "{state}");
        }
    }

    #[test]
    fn a_blocked_pr_already_approved_names_no_reason_though_its_checks_fail() {
        let approved = json!({"mergeStateStatus": "BLOCKED", "reviewDecision": "APPROVED", "commits": checks("FAILURE")});
        assert_eq!(verdict(&approved), forge::MergeVerdict::Blocked);
    }

    #[test]
    fn only_write_maintain_and_admin_may_merge_a_clean_pr() {
        let clean = pr(&json!({"mergeStateStatus": "CLEAN"}));
        for permission in ["WRITE", "MAINTAIN", "ADMIN"] {
            assert_eq!(clean.merge_verdict(Some(permission)), forge::MergeVerdict::Mergeable, "{permission}");
        }
        for permission in ["READ", "TRIAGE", "MODERATE", "RELEASE_MANAGER"] {
            assert_eq!(clean.merge_verdict(Some(permission)), forge::MergeVerdict::NotAllowed, "{permission}");
        }
    }

    #[test]
    fn a_pr_github_answers_without_my_rights_is_unknown() {
        assert_eq!(pr(&json!({"mergeStateStatus": "CLEAN"})).merge_verdict(None), forge::MergeVerdict::Unknown);
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
        client(&server).set_readiness(&key(), forge::Readiness::Draft).await.unwrap();
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
        client(&server).set_readiness(&key(), forge::Readiness::Ready).await.unwrap();
    }
}
