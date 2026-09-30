//! The GitLab backend: REST for one MR, GraphQL for the queue. Wire shapes live in `wire` and
//! turn into the neutral model at this edge.
mod award;
mod graphql;
mod rest;
mod upload;
mod wire;

#[cfg(test)]
pub(crate) use wire::fixture;

use super::http::{Flavor, Transport};
use super::{LineRef, User};
use crate::auth::Credentials;
use anyhow::{Context, Result};
use reqwest::header::{HeaderMap, HeaderValue};

const TOKEN_HEADER: &str = "PRIVATE-TOKEN";
/// GitLab's GraphQL endpoint lives beside `/api/v4/`, not under it.
const GRAPHQL: &str = "../graphql";

static FLAVOR: Flavor =
    Flavor { remaining: "ratelimit-remaining", reset: "ratelimit-reset", waits_on_spent_403: false, error_message: wire::error_message };

/// One GitLab host. The token is sent to that host and nowhere else: redirects are refused
/// and every URL is checked against its origin before the header goes on.
#[derive(Clone, Debug)]
pub struct Client {
    http: Transport,
    host: String,
    /// My username here, asked once: an MR reads as mine by it.
    me: std::sync::Arc<tokio::sync::OnceCell<String>>,
}

impl Client {
    pub fn new(credentials: &Credentials) -> Result<Self> {
        Self::build(credentials, &format!("https://{}/api/v4/", credentials.host))
    }

    /// For tests: point at a mock server. The origin guard still applies to that server.
    #[cfg(test)]
    pub fn with_base(credentials: &Credentials, base: &str) -> Result<Self> {
        let host = url::Url::parse(base)?.host_str().context("base url without a host")?.to_owned();
        Self::build(&Credentials { host, token: credentials.token.clone() }, base)
    }

    fn build(credentials: &Credentials, base: &str) -> Result<Self> {
        let mut token = HeaderValue::from_str(&credentials.token).context("token has characters a header cannot carry")?;
        token.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(TOKEN_HEADER, token);
        Ok(Self { http: Transport::new(base, headers, &FLAVOR)?, host: credentials.host.clone(), me: std::sync::Arc::default() })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn rate(&self) -> super::budget::RateLimit {
        self.http.rate()
    }

    pub async fn me(&self) -> Result<User> {
        self.http.get::<wire::User>("user").await.map(User::from)
    }

    /// My username on this host, asked the first time and kept for the session.
    pub async fn my_name(&self) -> Result<String> {
        self.me.get_or_try_init(|| async { self.me().await.map(|u| u.username) }).await.cloned()
    }
}

/// GitLab's page for one diff line: the MR's diffs, scrolled to `sha1(path)_old_new`.
pub fn line_url(web_url: &str, path: &str, line: LineRef) -> String {
    format!("{web_url}/diffs#{}", wire::line_code(path, line.old, line.new))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn creds() -> Credentials {
        Credentials { host: "gitlab.com".into(), token: "glpat-xxxx".into() }
    }

    fn me_json() -> serde_json::Value {
        serde_json::json!({"id": 7, "username": "nina", "name": "Nina", "avatar_url": null})
    }

    #[tokio::test]
    async fn me_sends_the_token_header() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/user"))
            .and(header(TOKEN_HEADER, "glpat-xxxx"))
            .respond_with(ResponseTemplate::new(200).set_body_json(me_json()))
            .mount(&server)
            .await;
        let client = Client::with_base(&creds(), &format!("{}/api/v4/", server.uri())).unwrap();
        let me = client.me().await.unwrap();
        assert_eq!((me.id, me.username.as_str()), (7, "nina"));
    }

    #[tokio::test]
    async fn a_logged_request_never_holds_the_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(me_json()))
            .mount(&server)
            .await;
        crate::log::capture::start();
        let client = Client::with_base(&creds(), &format!("{}/api/v4/", server.uri())).unwrap();
        client.me().await.unwrap();
        let text = crate::log::capture::text();
        assert!(text.contains("GET 127.0.0.1/api/v4/user 200"), "{text}");
        assert!(!text.contains("glpat-xxxx") && !text.contains(TOKEN_HEADER), "{text}");
    }

    #[tokio::test]
    async fn http_errors_carry_gitlabs_message() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/user"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({"message": "401 Unauthorized"})))
            .mount(&server)
            .await;
        let client = Client::with_base(&creds(), &format!("{}/api/v4/", server.uri())).unwrap();
        let err = client.me().await.unwrap_err();
        assert_eq!(crate::forge::http::status(&err), Some(reqwest::StatusCode::UNAUTHORIZED));
        assert!(err.to_string().ends_with("HTTP 401 401 Unauthorized"), "{err}");
    }

    #[test]
    fn graphql_lives_beside_v4() {
        let client = Client::new(&creds()).unwrap();
        assert_eq!(client.http.url(GRAPHQL).unwrap().as_str(), "https://gitlab.com/api/graphql");
    }
}
