//! Backends and fixtures the backend's tests share.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::Backend;
use crate::cache::Cache;
use crate::diff::words::InlineRule;
use crate::forge::gitlab::Client;
use crate::forge::{Forge, Queue};
use crate::tui::lanes::Lanes;

fn test_forge() -> Forge {
    Forge::GitLab(Client::new(&crate::auth::Credentials { host: "gitlab.com".into(), token: "glpat-xxxx".into() }).unwrap())
}

pub(super) fn backend_on(server: &wiremock::MockServer) -> Backend {
    let creds = crate::auth::Credentials { host: "gitlab.com".into(), token: "glpat-xxxx".into() };
    let forge = Forge::GitLab(Client::with_base(&creds, &format!("{}/api/v4/", server.uri())).unwrap());
    let dir = tempfile::tempdir().unwrap();
    Backend { forge, cache: Cache::in_dir(dir.path()), ..backend_on_nothing() }
}

pub(super) fn backend_on_nothing() -> Backend {
    Backend {
        others: vec![],
        forge: test_forge(),
        cache: Cache::in_dir(std::path::Path::new("/nonexistent")),
        fold_globs: vec![],
        watch_labels: vec![],
        rules: crate::forge::rules::Rules::disabled(),
        ready_command: None,
        inline: InlineRule::default(),
        open: crate::config::Open::default(),
        checkout: None,
        jev: None,
        claude: None,
        agent: None,
        opening: std::sync::Arc::default(),
        saved: std::sync::Arc::default(),
        changed: std::sync::Arc::default(),
        draft_lanes: Lanes::default(),
    }
}

pub(super) fn queue() -> Queue {
    crate::forge::gitlab::fixture::queue(include_str!("../../forge/gitlab/fixtures/queue.json"))
}

/// One MR on the mock GitLab, changed at `updated`; its five requests (MR, approvals, diffs, discussions,
/// drafts) may come `times[i]` times.
pub(super) async fn mount_mr(server: &wiremock::MockServer, updated: &str, times: [u64; 5]) {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, ResponseTemplate};
    let base = "/api/v4/projects/acme%2Fwidgets/merge_requests/42";
    let mr = serde_json::json!({
        "id": 1042, "iid": 42, "project_id": 7, "title": "feat: charge cards", "description": "", "state": "opened", "draft": false,
        "author": {"id": 5, "username": "omar", "name": "Omar"}, "source_branch": "feat/checkout", "target_branch": "main",
        "web_url": "https://gitlab.com/acme/widgets/-/merge_requests/42", "updated_at": updated, "sha": "bbbb",
        "diff_refs": {"base_sha": "aaaa", "head_sha": "bbbb", "start_sha": "aaaa"}
    });
    let answers = [
        (base.to_owned(), mr),
        (format!("{base}/approvals"), serde_json::json!({"approved": false, "approvals_left": 1, "approved_by": []})),
        (format!("{base}/diffs"), serde_json::json!([{"diff": "@@ -1 +1 @@\n-a\n+b\n", "old_path": "a.rs", "new_path": "a.rs"}])),
        (format!("{base}/discussions"), serde_json::json!([])),
        (format!("{base}/draft_notes"), serde_json::json!([])),
    ];
    for ((route, body), times) in answers.into_iter().zip(times) {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(times)
            .mount(server)
            .await;
    }
}
