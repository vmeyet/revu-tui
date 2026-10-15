use crate::auth::Credentials;
use crate::forge::MrKey;
use crate::forge::github::Client;
use serde_json::{Value, json};
use wiremock::MockServer;

pub(super) fn key() -> MrKey {
    MrKey::new("acme/widgets", 42)
}

pub(super) fn client(server: &MockServer) -> Client {
    Client::with_base(&Credentials { host: "github.com".into(), token: "ghp_xxxx".into() }, &server.uri()).unwrap()
}

pub(super) fn threads_json() -> Value {
    serde_json::from_str(include_str!("../fixtures/threads.json")).unwrap()
}

pub(super) fn comment_json(id: &str, db: u64, body: &str) -> Value {
    json!({"id": id, "fullDatabaseId": db.to_string(), "body": body, "createdAt": "2026-09-22T10:00:00Z", "updatedAt": "2026-09-22T10:00:00Z", "state": "PENDING", "author": {"login": "nina"}})
}
