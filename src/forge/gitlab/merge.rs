//! Merging on GitLab: its verdict on an MR, the merge itself, and its refusals in words.
use super::Client;
use super::rest::mr_path;
use crate::forge::http::HttpError;
use crate::forge::{self, MergeVerdict, MrKey};
use anyhow::{Result, anyhow};
use reqwest::StatusCode;
use serde_json::json;

/// GitLab's `detailed_merge_status` as a verdict. The status is the same for everyone, so my right to
/// merge comes from `user.can_merge` and wins over it.
/// A status missing, or newer than this list, is unknown; the old names of renamed ones still read.
pub(super) fn verdict(status: Option<&str>, can_merge: Option<bool>) -> MergeVerdict {
    use MergeVerdict as Verdict;
    let verdict = match status.unwrap_or_default() {
        "mergeable" => Verdict::Ready,
        "checking" | "unchecked" | "preparing" | "approvals_syncing" => Verdict::Checking,
        "conflict" | "broken_status" => Verdict::Conflicts,
        "need_rebase" => Verdict::Behind,
        "ci_must_pass" | "security_policy_pipeline_check" => Verdict::PipelineNotPassed,
        "ci_still_running" => Verdict::PipelineRunning,
        "not_approved" => Verdict::NotApproved,
        "requested_changes" => Verdict::ChangesRequested,
        "discussions_not_resolved" => Verdict::UnresolvedThreads,
        "draft_status" => Verdict::Draft,
        "not_open" => Verdict::NotOpen,
        "merge_request_blocked"
        | "blocked_status"
        | "status_checks_must_pass"
        | "external_status_checks"
        | "jira_association_missing"
        | "merge_time"
        | "security_policy_violations"
        | "policies_denied"
        | "locked_paths"
        | "locked_lfs_files"
        | "title_regex"
        | "commits_status" => Verdict::Blocked,
        _ => Verdict::Unknown,
    };
    if can_merge == Some(false) { Verdict::NotAllowed } else { verdict }
}

impl Client {
    /// GitLab applies the project's merge method; revu passes the squash and branch choices it
    /// showed, and `sha` so a push made after the reader looked is refused.
    pub(crate) async fn merge(&self, key: &MrKey, head: &str, plan: forge::MergePlan) -> Result<()> {
        let body = json!({
            "sha": head,
            "squash": plan.method == forge::MergeMethod::Squash,
            "should_remove_source_branch": plan.remove_branch,
        });
        self.http.put_json::<serde_json::Value>(&format!("{}/merge", mr_path(key)), &body).await.map(|_| ()).map_err(merge_refused)
    }
}

/// The answers GitLab gives a merge it will not do, in words the reader can act on.
fn merge_refused(err: anyhow::Error) -> anyhow::Error {
    let Some(refused) = err.downcast_ref::<HttpError>() else { return err };
    match refused.status {
        StatusCode::CONFLICT => anyhow!("the branch moved since you read it: refresh with r and look at the new commits"),
        StatusCode::METHOD_NOT_ALLOWED | StatusCode::NOT_ACCEPTABLE => anyhow!("GitLab will not merge it yet: {}", refused.message),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => anyhow!("you are not allowed to merge into this branch"),
        _ => err,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Credentials;
    use crate::forge::gitlab::fixture::key;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(server: &MockServer) -> Client {
        Client::with_base(&Credentials { host: "gitlab.com".into(), token: "glpat-xxxx".into() }, &format!("{}/api/v4/", server.uri()))
            .unwrap()
    }

    fn mr_json(fields: &serde_json::Value) -> serde_json::Value {
        let mut mr = json!({
            "iid": 42, "title": "feat: charge cards at checkout", "state": "opened", "draft": false,
            "author": {"id": 5, "username": "nina", "name": "Nina"},
            "source_branch": "feat/checkout", "target_branch": "main",
            "web_url": "https://gitlab.com/acme/widgets/-/merge_requests/42",
            "updated_at": "2026-09-22T09:12:00Z",
            "diff_refs": {"base_sha": "aaaa", "head_sha": "bbbb", "start_sha": "aaaa"},
            "head_pipeline": {"id": 1, "status": "failed", "web_url": null},
            "user": {"can_merge": true}
        });
        mr.as_object_mut().unwrap().extend(fields.as_object().unwrap().clone());
        mr
    }

    /// My own MR, unapproved and with a failed pipeline, as GitLab answers it with `fields` on top.
    async fn my_mr(fields: serde_json::Value) -> forge::Mr {
        let server = MockServer::start().await;
        let answers = [
            ("/api/v4/projects/acme%2Fwidgets/merge_requests/42", mr_json(&fields)),
            ("/api/v4/projects/acme%2Fwidgets/merge_requests/42/approvals", json!({"approved": false, "approvals_left": 0})),
            ("/api/v4/user", json!({"id": 5, "username": "nina", "name": "Nina"})),
        ];
        for (at, answer) in answers {
            Mock::given(method("GET")).and(path(at)).respond_with(ResponseTemplate::new(200).set_body_json(answer)).mount(&server).await;
        }
        client(&server).mr(&key()).await.unwrap()
    }

    #[tokio::test]
    async fn each_merge_status_of_gitlab_reads_as_its_verdict() {
        let cases: [(&str, MergeVerdict); 28] = [
            ("mergeable", MergeVerdict::Ready),
            ("checking", MergeVerdict::Checking),
            ("unchecked", MergeVerdict::Checking),
            ("preparing", MergeVerdict::Checking),
            ("approvals_syncing", MergeVerdict::Checking),
            ("conflict", MergeVerdict::Conflicts),
            ("broken_status", MergeVerdict::Conflicts),
            ("need_rebase", MergeVerdict::Behind),
            ("ci_must_pass", MergeVerdict::PipelineNotPassed),
            ("security_policy_pipeline_check", MergeVerdict::PipelineNotPassed),
            ("ci_still_running", MergeVerdict::PipelineRunning),
            ("not_approved", MergeVerdict::NotApproved),
            ("requested_changes", MergeVerdict::ChangesRequested),
            ("discussions_not_resolved", MergeVerdict::UnresolvedThreads),
            ("draft_status", MergeVerdict::Draft),
            ("not_open", MergeVerdict::NotOpen),
            ("merge_request_blocked", MergeVerdict::Blocked),
            ("blocked_status", MergeVerdict::Blocked),
            ("status_checks_must_pass", MergeVerdict::Blocked),
            ("external_status_checks", MergeVerdict::Blocked),
            ("jira_association_missing", MergeVerdict::Blocked),
            ("merge_time", MergeVerdict::Blocked),
            ("security_policy_violations", MergeVerdict::Blocked),
            ("policies_denied", MergeVerdict::Blocked),
            ("locked_paths", MergeVerdict::Blocked),
            ("locked_lfs_files", MergeVerdict::Blocked),
            ("title_regex", MergeVerdict::Blocked),
            ("commits_status", MergeVerdict::Blocked),
        ];
        for (status, verdict) in cases {
            assert_eq!(my_mr(json!({"detailed_merge_status": status})).await.merge_verdict, verdict, "{status}");
        }
    }

    #[tokio::test]
    async fn a_merge_status_gitlab_adds_later_is_unknown() {
        let mr = my_mr(json!({"detailed_merge_status": "waiting_in_a_merge_train"})).await;
        assert_eq!(mr.merge_verdict, MergeVerdict::Unknown);
    }

    #[tokio::test]
    async fn an_mr_gitlab_answers_without_a_merge_status_is_unknown() {
        assert_eq!(my_mr(json!({})).await.merge_verdict, MergeVerdict::Unknown);
    }

    #[tokio::test]
    async fn an_mr_i_may_not_merge_is_not_allowed_though_its_status_is_mergeable() {
        let mr = my_mr(json!({"detailed_merge_status": "mergeable", "user": {"can_merge": false}})).await;
        assert_eq!(mr.merge_verdict, MergeVerdict::NotAllowed);
    }

    #[tokio::test]
    async fn an_mr_gitlab_answers_without_my_rights_keeps_its_status() {
        let mr = my_mr(json!({"detailed_merge_status": "mergeable", "user": null})).await;
        assert_eq!(mr.merge_verdict, MergeVerdict::Ready);
    }

    #[tokio::test]
    async fn merge_sends_the_head_and_the_choices_shown_and_words_the_refusals() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v4/projects/acme%2Fwidgets/merge_requests/42/merge"))
            .and(body_partial_json(json!({"sha": "bbbb", "squash": true, "should_remove_source_branch": true})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"iid": 42, "state": "merged"})))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/api/v4/projects/acme%2Fwidgets/merge_requests/43/merge"))
            .respond_with(ResponseTemplate::new(409).set_body_json(json!({"message": "SHA does not match HEAD of source branch"})))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/api/v4/projects/acme%2Fwidgets/merge_requests/44/merge"))
            .respond_with(ResponseTemplate::new(405).set_body_json(json!({"message": "405 Method Not Allowed"})))
            .mount(&server)
            .await;
        let client = client(&server);
        let plan = forge::MergePlan { method: forge::MergeMethod::Squash, remove_branch: true };
        client.merge(&key(), "bbbb", plan).await.unwrap();
        let moved = client.merge(&MrKey::new("acme/widgets", 43), "bbbb", plan).await.unwrap_err().to_string();
        assert!(moved.contains("the branch moved"), "{moved}");
        let early = client.merge(&MrKey::new("acme/widgets", 44), "bbbb", plan).await.unwrap_err().to_string();
        assert_eq!(early, "GitLab will not merge it yet: 405 Method Not Allowed");
    }
}
