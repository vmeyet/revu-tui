//! Hosts and fixtures the backend's tests share.
use super::Homes;
use crate::cache::Cache;
use crate::forge::gitlab::Client;
use crate::forge::{Forge, Queue};
use std::path::Path;

fn credentials() -> crate::auth::Credentials {
    crate::auth::Credentials { host: "gitlab.com".into(), token: "glpat-xxxx".into() }
}

/// One GitLab host that no request reaches, and no cache that can be written.
pub(super) fn homes_on_nothing() -> Homes {
    let forge = Forge::GitLab(Client::new(&credentials()).unwrap());
    Homes { forge, cache: Cache::in_dir(Path::new("/nonexistent")), others: vec![] }
}

/// One host, no request reaching it, with its cache in `dir`.
pub(super) fn homes_in(dir: &Path) -> Homes {
    Homes { cache: Cache::in_dir(dir), ..homes_on_nothing() }
}

/// One GitLab host answered by `server`, with its cache in `dir`.
pub(super) fn homes_on(server: &wiremock::MockServer, dir: &Path) -> Homes {
    let forge = Forge::GitLab(Client::with_base(&credentials(), &format!("{}/api/v4/", server.uri())).unwrap());
    Homes { forge, cache: Cache::in_dir(dir), others: vec![] }
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
