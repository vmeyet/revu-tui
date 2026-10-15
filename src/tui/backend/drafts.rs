//! Draft writes and publishing, one at a time per MR.
use super::Backend;
use crate::cache::keys;
use crate::forge::{Draft as HeldDraft, MrKey};
use crate::review::Draft;
use crate::tui::app::Incoming;
use anyhow::Result;

impl Backend {
    /// Posts the draft unless the forge already lists it: a retry after a lost answer never doubles a note.
    pub(in crate::tui) async fn save_draft(&self, key: MrKey, draft: Box<Draft>) -> Result<Incoming> {
        let _lane = self.draft_lanes.enter(&key).await;
        let held = self.forge_of(&key).drafts(&key).await?;
        let saved = match held.into_iter().find(|note| draft.same_as(&Draft::from_held(note))) {
            Some(note) => note,
            None => self.forge_of(&key).create_draft(&key, &draft.payload()).await?,
        };
        Ok(Incoming::DraftSaved { key, draft, id: saved.id, body: saved.body })
    }

    pub(in crate::tui) async fn update_draft(&self, key: &MrKey, id: u64, draft: &Draft) -> Result<()> {
        let _lane = self.draft_lanes.enter(key).await;
        self.forge_of(key).update_draft(key, id, &draft.payload()).await.map(|_| ())
    }

    pub(in crate::tui) async fn delete_draft(&self, key: &MrKey, id: u64) -> Result<()> {
        let _lane = self.draft_lanes.enter(key).await;
        self.forge_of(key).delete_draft(key, id).await
    }

    pub(in crate::tui) async fn publish(&self, key: MrKey, approve: bool, count: usize) -> Result<Incoming> {
        let _lane = self.draft_lanes.enter(&key).await;
        let approval = self.forge_of(&key).publish(&key, approve).await?;
        let wanted = key.clone();
        self.write_cache("caching the drafts", move |b| b.cache_of(&wanted).write(&keys::drafts(&wanted), &Vec::<HeldDraft>::new())).await;
        Ok(Incoming::Published { key, approval, count })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::forge::gitlab::fixture::key;
    use crate::forge::{Forge, LineRef, Position, Refs};
    use crate::tui::backend::test_support::{backend_on, backend_on_nothing};
    use std::time::Duration;

    fn held_note(id: u64, note: &str, new_line: u32) -> serde_json::Value {
        serde_json::json!({
            "id": id, "author_id": 1, "merge_request_id": 42, "note": note,
            "position": {"base_sha": "a", "head_sha": "b", "start_sha": "a", "position_type": "text",
                         "old_path": "src/a.rs", "new_path": "src/a.rs", "old_line": null, "new_line": new_line}
        })
    }

    fn draft_at(new_line: u32, body: &str) -> Draft {
        let refs = Refs { base: "a".into(), start: "a".into(), head: "b".into() };
        let line = LineRef { old: None, new: Some(new_line) };
        Draft::on(Position { refs, old_path: "src/a.rs".into(), new_path: "src/a.rs".into(), line, start: None }, body)
    }

    /// A GitHub whose pending review text is read slowly and written whole, as `create_draft` does for a note on the PR.
    struct PendingReview(std::sync::Arc<std::sync::Mutex<String>>);

    impl wiremock::Respond for PendingReview {
        fn respond(&self, request: &wiremock::Request) -> wiremock::ResponseTemplate {
            let asked: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            let mut text = self.0.lock().unwrap();
            if let Some(body) = asked["variables"]["body"].as_str() {
                body.clone_into(&mut text);
                let review = serde_json::json!({"id": "PRR_9", "fullDatabaseId": "209", "body": *text, "createdAt": "2026-09-22T10:00:00Z",
                    "updatedAt": "2026-09-22T10:00:00Z", "state": "PENDING", "author": {"login": "nina"}});
                return wiremock::ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"data": {"updatePullRequestReview": {"pullRequestReview": review}}}));
            }
            let mut threads: serde_json::Value = serde_json::from_str(include_str!("../../forge/github/fixtures/threads.json")).unwrap();
            let reviews = threads["data"]["repository"]["pullRequest"]["reviews"]["nodes"].as_array_mut().unwrap();
            reviews.iter_mut().filter(|r| r["state"] == "PENDING").for_each(|r| r["body"] = serde_json::json!(*text));
            wiremock::ResponseTemplate::new(200).set_body_json(threads).set_delay(Duration::from_millis(30))
        }
    }

    #[tokio::test]
    async fn two_notes_on_a_github_pr_saved_at_once_both_land() {
        let server = wiremock::MockServer::start().await;
        let text = std::sync::Arc::new(std::sync::Mutex::new("Two nits.".to_owned()));
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/graphql"))
            .respond_with(PendingReview(text.clone()))
            .mount(&server)
            .await;
        let creds = crate::auth::Credentials { host: "github.com".into(), token: "ghp_xxxx".into() };
        let forge = Forge::GitHub(crate::forge::github::Client::with_base(&creds, &server.uri()).unwrap());
        let backend = Backend { forge, ..backend_on_nothing() };
        let pr = MrKey::new("acme/widgets", 42);
        let (one, two) = tokio::join!(
            backend.save_draft(pr.clone(), Box::new(Draft::new(None, "Also: docs"))),
            backend.save_draft(pr.clone(), Box::new(Draft::new(None, "And tests")))
        );
        assert!(one.is_ok() && two.is_ok(), "{one:?} {two:?}");
        let text = text.lock().unwrap().clone();
        assert!(text.contains("Also: docs") && text.contains("And tests"), "one note overwrote the other: {text:?}");
    }

    #[tokio::test]
    async fn a_publish_whose_approval_is_refused_still_forgets_the_published_drafts() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let base = "/api/v4/projects/acme%2Fwidgets/merge_requests/42";
        Mock::given(method("POST"))
            .and(path(format!("{base}/draft_notes/bulk_publish")))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        Mock::given(method("POST")).and(path(format!("{base}/approve"))).respond_with(ResponseTemplate::new(401)).mount(&server).await;
        let backend = backend_on(&server);
        let held = vec![HeldDraft { id: 5, body: "nit".into(), position: None, reply_to: None, resolve: false }];
        backend.cache.write(&keys::drafts(&key()), &held).unwrap();
        let Incoming::Published { approval, count: 1, .. } = backend.publish(key(), true, 1).await.unwrap() else {
            panic!("not published")
        };
        assert!(matches!(approval, crate::forge::Approval::Refused(_)), "{approval:?}");
        assert_eq!(backend.cache.read::<Vec<HeldDraft>>(&keys::drafts(&key())), Some(vec![]));
    }

    #[tokio::test]
    async fn a_draft_the_forge_already_holds_is_not_posted_twice() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/acme%2Fwidgets/merge_requests/42/draft_notes"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([held_note(5, "nit", 12)])))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v4/projects/acme%2Fwidgets/merge_requests/42/draft_notes"))
            .respond_with(ResponseTemplate::new(201).set_body_json(held_note(6, "other", 13)))
            .expect(1)
            .mount(&server)
            .await;
        let backend = backend_on(&server);
        let same = Box::new(draft_at(12, "nit"));
        assert_eq!(
            backend.save_draft(key(), same.clone()).await.unwrap(),
            Incoming::DraftSaved { key: key(), draft: same, id: 5, body: "nit".into() }
        );
        let fresh = Box::new(draft_at(13, "other"));
        assert_eq!(
            backend.save_draft(key(), fresh.clone()).await.unwrap(),
            Incoming::DraftSaved { key: key(), draft: fresh, id: 6, body: "other".into() }
        );
    }
}
