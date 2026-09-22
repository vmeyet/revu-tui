pub mod graphql;
pub mod rest;
pub mod types;

pub use graphql::{Queue, QueueMr, ReviewState, Sections};
pub use types::{Approvals, DiffFile, DiffRefs, Discussion, Mr, Note, Pipeline, Position, User};

use crate::auth::Credentials;
use anyhow::{Context, Result, bail};
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{Method, Response, StatusCode};
use serde::de::DeserializeOwned;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::Url;

const TOKEN_HEADER: &str = "PRIVATE-TOKEN";
const PAGE_SIZE: &str = "100";
const MAX_WAIT: Duration = Duration::from_secs(60);

/// One GitLab host. The token is sent to that host and nowhere else: redirects are refused
/// and every URL is checked against `host` before the header goes on.
#[derive(Clone, Debug)]
pub struct Client {
    http: reqwest::Client,
    host: String,
    base: Url,
    remaining: Arc<AtomicU64>,
}

impl Client {
    pub fn new(credentials: &Credentials) -> Result<Self> {
        let mut headers = HeaderMap::new();
        let mut token = HeaderValue::from_str(&credentials.token).context("token has characters a header cannot carry")?;
        token.set_sensitive(true);
        headers.insert(TOKEN_HEADER, token);
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("gitlabmr/", env!("CARGO_PKG_VERSION")))
            .build()?;
        let base = Url::parse(&format!("https://{}/api/v4/", credentials.host)).context("host is not a hostname")?;
        Ok(Self { http, host: credentials.host.clone(), base, remaining: Arc::new(AtomicU64::new(u64::MAX)) })
    }

    /// For tests: point at a mock server. The host guard still applies to that server's host.
    pub fn with_base(credentials: &Credentials, base: &str) -> Result<Self> {
        let base = Url::parse(base)?;
        let host = base.host_str().context("base url without a host")?.to_owned();
        let client = Self::new(&Credentials { host: host.clone(), token: credentials.token.clone() })?;
        Ok(Self { base, host, ..client })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    /// Requests left in the current rate-limit window, from the last answer; `None` before the first one.
    pub fn remaining(&self) -> Option<u64> {
        Some(self.remaining.load(Ordering::Relaxed)).filter(|n| *n != u64::MAX)
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let response = self.send(Method::GET, self.url(path)?, None).await?;
        response.json().await.map_err(scrub).with_context(|| format!("GET {path}: unreadable answer"))
    }

    /// `path` is relative to `/api/v4/` except `graphql`, which lives beside it.
    pub async fn post_json<T: DeserializeOwned>(&self, path: &str, body: &serde_json::Value) -> Result<T> {
        let response = self.send(Method::POST, self.url(path)?, Some(body)).await?;
        response.json().await.map_err(scrub).with_context(|| format!("POST {path}: unreadable answer"))
    }

    /// Every page of a list, following `Link: rel="next"` until it stops.
    pub async fn get_all<T: DeserializeOwned>(&self, path: &str) -> Result<Vec<T>> {
        let mut url = self.url(path)?;
        url.query_pairs_mut().append_pair("per_page", PAGE_SIZE);
        let mut items = Vec::new();
        loop {
            let response = self.send(Method::GET, url, None).await?;
            let next = response.headers().get("link").and_then(|h| h.to_str().ok()).and_then(next_link).map(str::to_owned);
            let page: Vec<T> = response.json().await.map_err(scrub).with_context(|| format!("GET {path}: unreadable page"))?;
            items.extend(page);
            match next {
                Some(link) => url = self.url_from(&link)?,
                None => return Ok(items),
            }
        }
    }

    pub async fn me(&self) -> Result<User> {
        self.get("user").await
    }

    /// One request with the token, retried once after the wait GitLab asks for on 429.
    async fn send(&self, method: Method, url: Url, body: Option<&serde_json::Value>) -> Result<Response> {
        let first = self.send_once(method.clone(), url.clone(), body).await?;
        if first.status() != StatusCode::TOO_MANY_REQUESTS {
            return checked(first, &method, &url).await;
        }
        tokio::time::sleep(wait_from(first.headers()).min(MAX_WAIT)).await;
        let second = self.send_once(method.clone(), url.clone(), body).await?;
        checked(second, &method, &url).await
    }

    async fn send_once(&self, method: Method, url: Url, body: Option<&serde_json::Value>) -> Result<Response> {
        let request = self.http.request(method, url);
        let request = match body {
            Some(json) => request.json(json),
            None => request,
        };
        let response = request.send().await.map_err(scrub)?;
        if let Some(left) = response.headers().get("ratelimit-remaining").and_then(|h| h.to_str().ok()?.parse().ok()) {
            self.remaining.store(left, Ordering::Relaxed);
        }
        Ok(response)
    }

    fn url(&self, path: &str) -> Result<Url> {
        let joined = match path {
            "graphql" => self.base.join("../graphql")?,
            _ => self.base.join(path.trim_start_matches('/'))?,
        };
        self.guard(joined)
    }

    fn url_from(&self, absolute: &str) -> Result<Url> {
        self.guard(Url::parse(absolute).with_context(|| format!("bad link {absolute:?}"))?)
    }

    fn guard(&self, url: Url) -> Result<Url> {
        if url.host_str() != Some(self.host.as_str()) {
            bail!("refusing to send the token to {}", url.host_str().unwrap_or("?"));
        }
        Ok(url)
    }
}

async fn checked(response: Response, method: &Method, url: &Url) -> Result<Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let message = response.text().await.unwrap_or_default();
    bail!("{method} {}: HTTP {} {}", url.path(), status.as_u16(), types::error_message(&message))
}

/// `Retry-After` in seconds, else `Ratelimit-Reset` as a unix time, else one second.
fn wait_from(headers: &HeaderMap) -> Duration {
    let number = |name: &str| headers.get(name).and_then(|h| h.to_str().ok()?.trim().parse::<u64>().ok());
    if let Some(seconds) = number("retry-after") {
        return Duration::from_secs(seconds);
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    number("ratelimit-reset").map(|reset| Duration::from_secs(reset.saturating_sub(now))).unwrap_or(Duration::from_secs(1))
}

/// The `<url>` whose `rel="next"` in a `Link` header.
fn next_link(header: &str) -> Option<&str> {
    header.split(',').map(str::trim).find(|part| part.contains("rel=\"next\"")).and_then(|part| {
        let start = part.find('<')? + 1;
        let end = part.find('>')?;
        part.get(start..end)
    })
}

/// reqwest errors carry the URL, never the header, but the chain is cut here anyway so nothing
/// below this module can leak a request.
fn scrub(err: reqwest::Error) -> anyhow::Error {
    anyhow::anyhow!("{}", err.without_url())
}

#[cfg(test)]
mod tests {
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
    async fn me_sends_the_token_header_and_reads_the_rate_limit() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/user"))
            .and(header(TOKEN_HEADER, "glpat-xxxx"))
            .respond_with(ResponseTemplate::new(200).set_body_json(me_json()).insert_header("Ratelimit-Remaining", "1992"))
            .mount(&server)
            .await;
        let client = Client::with_base(&creds(), &format!("{}/api/v4/", server.uri())).unwrap();
        assert_eq!(client.remaining(), None);
        let me = client.me().await.unwrap();
        assert_eq!((me.id, me.username.as_str()), (7, "nina"));
        assert_eq!(client.remaining(), Some(1992));
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
        let err = client.me().await.unwrap_err().to_string();
        assert!(err.contains("HTTP 401") && err.contains("Unauthorized"), "{err}");
    }

    #[tokio::test]
    async fn a_429_is_retried_once_after_the_asked_wait() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/user"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v4/user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(me_json()))
            .mount(&server)
            .await;
        let client = Client::with_base(&creds(), &format!("{}/api/v4/", server.uri())).unwrap();
        assert_eq!(client.me().await.unwrap().username, "nina");
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_second_429_is_an_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/user"))
            .respond_with(
                ResponseTemplate::new(429).insert_header("Retry-After", "0").set_body_json(serde_json::json!({"message": "Retry later"})),
            )
            .mount(&server)
            .await;
        let client = Client::with_base(&creds(), &format!("{}/api/v4/", server.uri())).unwrap();
        let err = client.me().await.unwrap_err().to_string();
        assert!(err.contains("429") && err.contains("Retry later"), "{err}");
    }

    #[test]
    fn token_never_leaves_the_host() {
        let client = Client::new(&creds()).unwrap();
        let err = client.url("https://evil.example/api/v4/user").unwrap_err().to_string();
        assert!(err.contains("evil.example"), "{err}");
        let err = client.url_from("https://evil.example/api/v4/user?page=2").unwrap_err().to_string();
        assert!(err.contains("evil.example"), "{err}");
    }

    #[test]
    fn graphql_lives_beside_v4() {
        let client = Client::new(&creds()).unwrap();
        assert_eq!(client.url("graphql").unwrap().as_str(), "https://gitlab.com/api/graphql");
        assert_eq!(client.url("/user").unwrap().as_str(), "https://gitlab.com/api/v4/user");
    }

    #[test]
    fn next_link_is_found_among_the_relations() {
        let header = r#"<https://gitlab.com/api/v4/x?page=2>; rel="next", <https://gitlab.com/api/v4/x?page=1>; rel="first""#;
        assert_eq!(next_link(header), Some("https://gitlab.com/api/v4/x?page=2"));
        assert_eq!(next_link(r#"<https://gitlab.com/api/v4/x?page=1>; rel="first""#), None);
    }

    #[test]
    fn wait_prefers_retry_after_then_reset_then_a_second() {
        let mut headers = HeaderMap::new();
        assert_eq!(wait_from(&headers), Duration::from_secs(1));
        let soon = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + 30;
        headers.insert("ratelimit-reset", HeaderValue::from_str(&soon.to_string()).unwrap());
        assert!((28..=30).contains(&wait_from(&headers).as_secs()));
        headers.insert("retry-after", HeaderValue::from_static("5"));
        assert_eq!(wait_from(&headers), Duration::from_secs(5));
    }
}
