//! The GitHub backend: GraphQL for the queue, one PR, its threads and the pending review; REST for
//! the files, public comments and lookups. Wire shapes stay here and turn into the neutral model at
//! this edge.
mod graphql;
mod rest;
mod wire;

use super::{LineRef, Side};
use crate::auth::Credentials;
use anyhow::{Context, Result, bail};
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue};
use reqwest::{Method, Response, StatusCode};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::Url;

const PAGE_SIZE: &str = "100";
const MAX_WAIT: Duration = Duration::from_secs(60);
const API_VERSION: &str = "2022-11-28";

/// One GitHub host. The token goes to its API host only: `api.github.com` for github.com, the
/// host itself for GitHub Enterprise; redirects are refused and every URL is checked first.
#[derive(Clone, Debug)]
pub struct Client {
    http: reqwest::Client,
    host: String,
    api_host: String,
    rest: Url,
    graphql: Url,
}

impl Client {
    pub fn new(credentials: &Credentials) -> Result<Self> {
        let (rest, graphql) = if credentials.host == "github.com" {
            ("https://api.github.com/".to_owned(), "https://api.github.com/graphql".to_owned())
        } else {
            (format!("https://{}/api/v3/", credentials.host), format!("https://{}/api/graphql", credentials.host))
        };
        Self::build(credentials, &rest, &graphql)
    }

    /// For tests: REST at `base`, GraphQL at `base/graphql`. The host guard applies to that server.
    #[cfg(test)]
    pub fn with_base(credentials: &Credentials, base: &str) -> Result<Self> {
        let base = base.trim_end_matches('/');
        Self::build(credentials, &format!("{base}/"), &format!("{base}/graphql"))
    }

    fn build(credentials: &Credentials, rest: &str, graphql: &str) -> Result<Self> {
        let mut headers = HeaderMap::new();
        let mut token =
            HeaderValue::from_str(&format!("Bearer {}", credentials.token)).context("token has characters a header cannot carry")?;
        token.set_sensitive(true);
        headers.insert(AUTHORIZATION, token);
        headers.insert(ACCEPT, HeaderValue::from_static("application/vnd.github+json"));
        headers.insert("X-GitHub-Api-Version", HeaderValue::from_static(API_VERSION));
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("revu/", env!("CARGO_PKG_VERSION")))
            .build()?;
        let rest = Url::parse(rest).context("host is not a hostname")?;
        let graphql = Url::parse(graphql).context("host is not a hostname")?;
        let api_host = rest.host_str().context("API url without a host")?.to_owned();
        Ok(Self { http, host: credentials.host.clone(), api_host, rest, graphql })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let response = self.send(Method::GET, self.url(path)?, None).await?;
        response.json().await.map_err(scrub).with_context(|| format!("GET {path}: unreadable answer"))
    }

    async fn post_json<T: DeserializeOwned>(&self, path: &str, body: &serde_json::Value) -> Result<T> {
        let response = self.send(Method::POST, self.url(path)?, Some(body)).await?;
        response.json().await.map_err(scrub).with_context(|| format!("POST {path}: unreadable answer"))
    }

    /// Every page of a list, following `Link: rel="next"` until it stops.
    async fn get_all<T: DeserializeOwned>(&self, path: &str) -> Result<Vec<T>> {
        let mut url = self.url(path)?;
        url.query_pairs_mut().append_pair("per_page", PAGE_SIZE);
        let mut items = Vec::new();
        loop {
            let response = self.send(Method::GET, url, None).await?;
            let next = response.headers().get("link").and_then(|h| h.to_str().ok()).and_then(next_link).map(str::to_owned);
            let page: Vec<T> = response.json().await.map_err(scrub).with_context(|| format!("GET {path}: unreadable page"))?;
            items.extend(page);
            match next {
                Some(link) => url = self.guard(Url::parse(&link).with_context(|| format!("bad link {link:?}"))?)?,
                None => return Ok(items),
            }
        }
    }

    /// One GraphQL call; an answer carrying `errors` is an error, whatever its status.
    async fn graphql<T: DeserializeOwned>(&self, query: &str, variables: serde_json::Value) -> Result<T> {
        let body = serde_json::json!({"query": query, "variables": variables});
        let response = self.send(Method::POST, self.guard(self.graphql.clone())?, Some(&body)).await?;
        let answer: wire::Answer<T> = response.json().await.map_err(scrub).context("GraphQL: unreadable answer")?;
        answer.into_data()
    }

    /// One request, retried once after the wait GitHub asks for when it rate-limits.
    async fn send(&self, method: Method, url: Url, body: Option<&serde_json::Value>) -> Result<Response> {
        let first = self.send_once(method.clone(), url.clone(), body).await?;
        if !rate_limited(&first) {
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
        request.send().await.map_err(scrub)
    }

    fn url(&self, path: &str) -> Result<Url> {
        self.guard(self.rest.join(path.trim_start_matches('/'))?)
    }

    fn guard(&self, url: Url) -> Result<Url> {
        if url.host_str() != Some(self.api_host.as_str()) {
            bail!("refusing to send the token to {}", url.host_str().unwrap_or("?"));
        }
        Ok(url)
    }
}

/// A 429, or a 403 that says the quota is spent: both come back after a wait.
fn rate_limited(response: &Response) -> bool {
    let spent = response.headers().get("x-ratelimit-remaining").and_then(|h| h.to_str().ok()) == Some("0");
    response.status() == StatusCode::TOO_MANY_REQUESTS || (response.status() == StatusCode::FORBIDDEN && spent)
}

async fn checked(response: Response, method: &Method, url: &Url) -> Result<Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let message = response.text().await.unwrap_or_default();
    bail!("{method} {}: HTTP {} {}", url.path(), status.as_u16(), wire::error_message(&message))
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

/// `Retry-After` in seconds, else `X-RateLimit-Reset` as a unix time, else one second.
fn wait_from(headers: &HeaderMap) -> Duration {
    let number = |name: &str| headers.get(name).and_then(|h| h.to_str().ok()?.trim().parse::<u64>().ok());
    if let Some(seconds) = number("retry-after") {
        return Duration::from_secs(seconds);
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    number("x-ratelimit-reset").map_or(Duration::from_secs(1), |reset| Duration::from_secs(reset.saturating_sub(now)))
}

/// The `<url>` whose `rel="next"` in a `Link` header.
fn next_link(header: &str) -> Option<&str> {
    header.split(',').map(str::trim).find(|part| part.contains("rel=\"next\"")).and_then(|part| {
        let start = part.find('<')? + 1;
        let end = part.find('>')?;
        part.get(start..end)
    })
}

/// reqwest errors carry the URL, never the header, but the chain is cut here anyway.
fn scrub(err: reqwest::Error) -> anyhow::Error {
    anyhow::anyhow!("{}", err.without_url())
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
        assert_eq!((dotcom.rest.as_str(), dotcom.graphql.as_str()), ("https://api.github.com/", "https://api.github.com/graphql"));
        let enterprise = Client::new(&Credentials { host: "git.acme.dev".into(), token: "t".into() }).unwrap();
        assert_eq!(enterprise.url("user").unwrap().as_str(), "https://git.acme.dev/api/v3/user");
        assert_eq!(enterprise.graphql.as_str(), "https://git.acme.dev/api/graphql");
    }

    #[test]
    fn token_never_leaves_the_api_host() {
        let client = Client::new(&creds()).unwrap();
        let err = client.guard(Url::parse("https://github.com/user").unwrap()).unwrap_err().to_string();
        assert!(err.contains("github.com"), "{err}");
        assert!(client.guard(Url::parse("https://evil.example/x").unwrap()).is_err());
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
        let err = Client::with_base(&creds(), &server.uri()).unwrap().me().await.unwrap_err().to_string();
        assert!(err.contains("HTTP 403") && err.contains("not accessible"), "{err}");
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

    #[test]
    fn next_link_is_found_among_the_relations() {
        let header = r#"<https://api.github.com/x?page=2>; rel="next", <https://api.github.com/x?page=5>; rel="last""#;
        assert_eq!(next_link(header), Some("https://api.github.com/x?page=2"));
    }
}
