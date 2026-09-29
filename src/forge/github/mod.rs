//! The GitHub backend: GraphQL for the queue, one PR, its threads and the pending review; REST for
//! the files, public comments and lookups. Wire shapes stay here and turn into the neutral model at
//! this edge.
mod attachment;
mod graphql;
mod rest;
mod wire;

use super::http::{Flavor, Transport, scrub};
use super::{LineRef, Side};
use crate::auth::Credentials;
use anyhow::{Context, Result};
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

/// What the contents API is asked for to answer with the file itself rather than base64 JSON.
const RAW: &str = "application/vnd.github.raw+json";
const API_VERSION: &str = "2022-11-28";

static FLAVOR: Flavor =
    Flavor { remaining: "x-ratelimit-remaining", reset: "x-ratelimit-reset", waits_on_spent_403: true, error_message: wire::error_message };

/// One GitHub host. The token goes to its API host only: `api.github.com` for github.com, the
/// host itself for GitHub Enterprise; redirects are refused and every URL is checked first.
#[derive(Clone, Debug)]
pub struct Client {
    http: Transport,
    host: String,
    /// The GraphQL endpoint, relative to the REST base.
    graphql: &'static str,
    /// Where comment pictures come from; shared, so the client stays small to clone.
    downloads: std::sync::Arc<attachment::Downloads>,
}

impl Client {
    pub fn new(credentials: &Credentials) -> Result<Self> {
        let web = format!("https://{}/", credentials.host);
        if credentials.host == "github.com" {
            Self::build(credentials, "https://api.github.com/", "graphql", &web)
        } else {
            Self::build(credentials, &format!("https://{}/api/v3/", credentials.host), "../graphql", &web)
        }
    }

    /// For tests: REST at `base`, GraphQL at `base/graphql`. The origin guard applies to that server.
    #[cfg(test)]
    pub fn with_base(credentials: &Credentials, base: &str) -> Result<Self> {
        let base = format!("{}/", base.trim_end_matches('/'));
        let client = Self::build(credentials, &base, "graphql", &base)?;
        Ok(Self { downloads: std::sync::Arc::new(client.downloads.trusting()), ..client })
    }

    fn build(credentials: &Credentials, rest: &str, graphql: &'static str, web: &str) -> Result<Self> {
        let mut headers = HeaderMap::new();
        let mut token =
            HeaderValue::from_str(&format!("Bearer {}", credentials.token)).context("token has characters a header cannot carry")?;
        token.set_sensitive(true);
        headers.insert(AUTHORIZATION, token);
        headers.insert(ACCEPT, HeaderValue::from_static("application/vnd.github+json"));
        headers.insert("X-GitHub-Api-Version", HeaderValue::from_static(API_VERSION));
        let http = Transport::new(rest, headers, &FLAVOR)?;
        let downloads = std::sync::Arc::new(attachment::Downloads::new(web)?);
        Ok(Self { http, host: credentials.host.clone(), graphql, downloads })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn rate(&self) -> super::budget::RateLimit {
        self.http.rate()
    }

    /// A file's bytes as text: the contents API answers raw when asked for `vnd.github.raw`.
    async fn get_raw(&self, path: &str) -> Result<String> {
        let response = self.http.fetch(path, RAW).await?;
        response.text().await.map_err(scrub).with_context(|| format!("GET {path}: unreadable answer"))
    }

    /// One GraphQL call; an answer carrying `errors` is an error, whatever its status.
    async fn graphql<T: DeserializeOwned>(&self, query: &str, variables: serde_json::Value) -> Result<T> {
        let body = serde_json::json!({"query": query, "variables": variables});
        let answer: wire::Answer<T> = self.http.post_json(self.graphql, &body).await?;
        answer.into_data()
    }
}

/// GitHub's page for one diff line: the PR's files, scrolled to `diff-sha256(path)` and the side.
pub fn line_url(web_url: &str, path: &str, line: LineRef) -> String {
    let anchor = format!("{:x}", Sha256::digest(path.as_bytes()));
    match (line.side(), line.number()) {
        (Side::New, Some(n)) => format!("{web_url}/files#diff-{anchor}R{n}"),
        (Side::Old, Some(n)) => format!("{web_url}/files#diff-{anchor}L{n}"),
        (_, None) => format!("{web_url}/files#diff-{anchor}"),
    }
}

/// `owner/repo` split for GraphQL, which names a repository by its two halves.
fn owner_and_name(project: &str) -> Result<(&str, &str)> {
    project
        .split_once('/')
        .filter(|(o, n)| !o.is_empty() && !n.is_empty())
        .with_context(|| format!("not a repository: {project} (owner/repo)"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn creds() -> Credentials {
        Credentials { host: "github.com".into(), token: "ghp_xxxx".into() }
    }

    #[test]
    fn github_com_talks_to_its_api_host_and_enterprise_to_its_own() {
        let dotcom = Client::new(&creds()).unwrap();
        assert_eq!(dotcom.http.url("user").unwrap().as_str(), "https://api.github.com/user");
        assert_eq!(dotcom.http.url(dotcom.graphql).unwrap().as_str(), "https://api.github.com/graphql");
        assert!(dotcom.http.url("https://github.com/user").is_err(), "the web host is not the API host");
        let enterprise = Client::new(&Credentials { host: "git.acme.dev".into(), token: "t".into() }).unwrap();
        assert_eq!(enterprise.http.url("user").unwrap().as_str(), "https://git.acme.dev/api/v3/user");
        assert_eq!(enterprise.http.url(enterprise.graphql).unwrap().as_str(), "https://git.acme.dev/api/graphql");
    }

    #[tokio::test]
    async fn requests_carry_the_bearer_token_and_the_api_version() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .and(header("authorization", "Bearer ghp_xxxx"))
            .and(header("x-github-api-version", API_VERSION))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": 7, "login": "nina", "name": "Nina"})))
            .mount(&server)
            .await;
        let me = Client::with_base(&creds(), &server.uri()).unwrap().me().await.unwrap();
        assert_eq!((me.id, me.username.as_str(), me.name.as_str()), (7, "nina", "Nina"));
    }

    #[tokio::test]
    async fn a_spent_quota_is_waited_out_once() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(ResponseTemplate::new(403).insert_header("x-ratelimit-remaining", "0").insert_header("retry-after", "0"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": 7, "login": "nina", "name": null})))
            .mount(&server)
            .await;
        let me = Client::with_base(&creds(), &server.uri()).unwrap().me().await.unwrap();
        assert_eq!(me.name, "nina", "no name falls back to the login");
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_plain_403_is_an_error_with_githubs_message() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(
                ResponseTemplate::new(403).set_body_json(serde_json::json!({"message": "Resource not accessible by integration"})),
            )
            .mount(&server)
            .await;
        let err = Client::with_base(&creds(), &server.uri()).unwrap().me().await.unwrap_err();
        assert_eq!(crate::forge::http::status(&err), Some(reqwest::StatusCode::FORBIDDEN));
        assert!(err.to_string().ends_with("HTTP 403 Resource not accessible by integration"), "{err}");
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[test]
    fn line_urls_hash_the_path_and_name_the_side() {
        let url = "https://github.com/acme/widgets/pull/42";
        let real = "bdfc5619650b795d1ffec5e8af3154a9a4b1833e5ed458694907d33709d12825";
        assert_eq!(real, format!("{:x}", Sha256::digest(b"src/a.rs")));
        assert_eq!(line_url(url, "src/a.rs", LineRef { old: None, new: Some(14) }), format!("{url}/files#diff-{real}R14"));
        assert_eq!(line_url(url, "src/a.rs", LineRef { old: Some(13), new: None }), format!("{url}/files#diff-{real}L13"));
        assert_eq!(line_url(url, "src/a.rs", LineRef { old: Some(12), new: Some(12) }), format!("{url}/files#diff-{real}R12"));
    }

    #[test]
    fn repositories_split_into_owner_and_name() {
        assert_eq!(owner_and_name("acme/widgets").unwrap(), ("acme", "widgets"));
        assert!(owner_and_name("widgets").is_err());
        assert!(owner_and_name("/widgets").is_err());
    }
}
