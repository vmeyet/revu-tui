//! The HTTP transport both forges share: the token rides on a client that follows no redirect and
//! reaches the origin of its base URL only; lists are read page by page and a rate limit is waited
//! out once. What differs between forges is a `Flavor`.
use super::budget::{self, Budget, RateLimit};
use anyhow::{Context, Result, bail};
use reqwest::header::{ACCEPT, HeaderMap};
use reqwest::{Method, Response, StatusCode};
use serde::de::DeserializeOwned;
use std::fmt;
use std::time::Duration;
use url::Url;

const PAGE_SIZE: &str = "100";
const MAX_WAIT: Duration = Duration::from_secs(60);

/// How one forge speaks HTTP where the two differ.
#[derive(Debug)]
pub struct Flavor {
    /// The header counting the requests left in the window.
    pub remaining: &'static str,
    /// The header giving the unix time the window starts again.
    pub reset: &'static str,
    /// Whether a 403 with no request left is a rate limit to wait out rather than a refusal.
    pub waits_on_spent_403: bool,
    /// The forge's error body, in one line.
    pub error_message: fn(&str) -> String,
}

/// An answer outside 2xx, kept typed so callers sort it by status.
#[derive(Debug)]
pub struct HttpError {
    /// `GET /api/v4/user`: the method and path, never the query or host.
    pub request: String,
    pub status: StatusCode,
    pub message: String,
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: HTTP {} {}", self.request, self.status.as_u16(), self.message)
    }
}

impl std::error::Error for HttpError {}

/// The status of an `HttpError` anywhere in `err`'s chain.
pub fn status(err: &anyhow::Error) -> Option<StatusCode> {
    err.downcast_ref::<HttpError>().map(|e| e.status)
}

#[derive(Clone, Debug)]
pub struct Transport {
    client: reqwest::Client,
    base: Url,
    budget: Budget,
    flavor: &'static Flavor,
}

impl Transport {
    /// `headers` carry the token: every request goes to `base`'s scheme, host and port or not at all.
    pub fn new(base: &str, headers: HeaderMap, flavor: &'static Flavor) -> Result<Self> {
        let client = crate::http::client()
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("revu/", env!("CARGO_PKG_VERSION")))
            .build()?;
        let base = Url::parse(base).context("host is not a hostname")?;
        Ok(Self { client, base, budget: Budget::default(), flavor })
    }

    /// The token-carrying client, for a caller that checked its own URL.
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    pub fn rate(&self) -> RateLimit {
        self.budget.now()
    }

    /// `path` joined to the base, refused when it leaves the base's origin.
    pub fn url(&self, path: &str) -> Result<Url> {
        self.guard(self.base.join(path.trim_start_matches('/'))?)
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let response = self.send(Method::GET, path, None).await?;
        response.json().await.map_err(scrub).with_context(|| format!("GET {path}: unreadable answer"))
    }

    pub async fn get_text(&self, path: &str) -> Result<String> {
        let response = self.send(Method::GET, path, None).await?;
        response.text().await.map_err(scrub).with_context(|| format!("GET {path}: unreadable answer"))
    }

    /// A GET asking for `accept` instead of the client's default type.
    pub async fn fetch(&self, path: &str, accept: &'static str) -> Result<Response> {
        self.send_to(Method::GET, self.url(path)?, None, Some(accept)).await
    }

    pub async fn post_json<T: DeserializeOwned>(&self, path: &str, body: &serde_json::Value) -> Result<T> {
        let response = self.send(Method::POST, path, Some(body)).await?;
        response.json().await.map_err(scrub).with_context(|| format!("POST {path}: unreadable answer"))
    }

    pub async fn put_json<T: DeserializeOwned>(&self, path: &str, body: &serde_json::Value) -> Result<T> {
        let response = self.send(Method::PUT, path, Some(body)).await?;
        response.json().await.map_err(scrub).with_context(|| format!("PUT {path}: unreadable answer"))
    }

    /// A request whose answer body does not matter: `204`s, publishes, approvals.
    pub async fn send_empty(&self, method: Method, path: &str, body: Option<&serde_json::Value>) -> Result<()> {
        self.send(method, path, body).await.map(|_| ())
    }

    pub async fn delete(&self, path: &str) -> Result<()> {
        self.send_empty(Method::DELETE, path, None).await
    }

    /// Every page of a list, following `Link: rel="next"` until it stops.
    pub async fn get_all<T: DeserializeOwned>(&self, path: &str) -> Result<Vec<T>> {
        let mut url = self.url(path)?;
        url.query_pairs_mut().append_pair("per_page", PAGE_SIZE);
        let mut items = Vec::new();
        loop {
            let response = self.send_to(Method::GET, url, None, None).await?;
            let next = response.headers().get("link").and_then(|h| h.to_str().ok()).and_then(next_link).map(str::to_owned);
            let page: Vec<T> = response.json().await.map_err(scrub).with_context(|| format!("GET {path}: unreadable page"))?;
            items.extend(page);
            match next {
                Some(link) => url = self.guard(Url::parse(&link).with_context(|| format!("bad link {link:?}"))?)?,
                None => return Ok(items),
            }
        }
    }

    /// One request with the token; a non-2xx answer is an `HttpError`.
    pub async fn send(&self, method: Method, path: &str, body: Option<&serde_json::Value>) -> Result<Response> {
        self.send_to(method, self.url(path)?, body, None).await
    }

    /// Retried once after the wait the forge asks for when it rate-limits.
    async fn send_to(&self, method: Method, url: Url, body: Option<&serde_json::Value>, accept: Option<&str>) -> Result<Response> {
        let first = self.send_once(method.clone(), url.clone(), body, accept).await?;
        if !self.rate_limited(&first) {
            return self.checked(first, &method, &url).await;
        }
        let wait = self.wait_from(first.headers()).min(MAX_WAIT);
        self.budget.waiting_for(wait);
        tokio::time::sleep(wait).await;
        self.budget.done_waiting();
        let second = self.send_once(method.clone(), url.clone(), body, accept).await?;
        self.checked(second, &method, &url).await
    }

    async fn send_once(&self, method: Method, url: Url, body: Option<&serde_json::Value>, accept: Option<&str>) -> Result<Response> {
        let request = self.client.request(method, url);
        let request = match body {
            Some(json) => request.json(json),
            None => request,
        };
        let request = match accept {
            Some(kind) => request.header(ACCEPT, kind),
            None => request,
        };
        let response = request.send().await.map_err(scrub)?;
        self.budget.note(budget::header_number(response.headers(), self.flavor.remaining));
        Ok(response)
    }

    fn rate_limited(&self, response: &Response) -> bool {
        let spent = budget::header_number(response.headers(), self.flavor.remaining) == Some(0);
        match response.status() {
            StatusCode::TOO_MANY_REQUESTS => true,
            StatusCode::FORBIDDEN => self.flavor.waits_on_spent_403 && spent,
            _ => false,
        }
    }

    async fn checked(&self, response: Response, method: &Method, url: &Url) -> Result<Response> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let body = response.text().await.unwrap_or_default();
        let request = format!("{method} {}", url.path());
        Err(HttpError { request, status, message: (self.flavor.error_message)(&body) }.into())
    }

    /// `Retry-After` in seconds, else the reset header as a unix time, else one second.
    fn wait_from(&self, headers: &HeaderMap) -> Duration {
        if let Some(seconds) = budget::header_number(headers, "retry-after") {
            return Duration::from_secs(seconds);
        }
        budget::header_number(headers, self.flavor.reset)
            .map_or(Duration::from_secs(1), |reset| Duration::from_secs(reset.saturating_sub(budget::unix_now())))
    }

    /// Same origin as the base: a host alone would let an `http://` link or another port take the token.
    fn guard(&self, url: Url) -> Result<Url> {
        if url.origin() != self.base.origin() {
            bail!("refusing to send the token to {}", url.origin().ascii_serialization());
        }
        Ok(url)
    }
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
/// above the transport can leak a request.
pub fn scrub(err: reqwest::Error) -> anyhow::Error {
    anyhow::anyhow!("{}", err.without_url())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use reqwest::header::HeaderValue;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    static PLAIN: Flavor = Flavor {
        remaining: "ratelimit-remaining",
        reset: "ratelimit-reset",
        waits_on_spent_403: false,
        error_message: |b| b.trim().to_owned(),
    };

    fn transport(base: &str) -> Transport {
        Transport::new(base, HeaderMap::new(), &PLAIN).unwrap()
    }

    #[tokio::test]
    async fn a_429_is_retried_once_after_the_asked_wait() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/user"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/user"))
            .respond_with(ResponseTemplate::new(200).set_body_string("nina"))
            .mount(&server)
            .await;
        assert_eq!(transport(&format!("{}/api/", server.uri())).get_text("user").await.unwrap(), "nina");
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_second_429_is_a_typed_error_with_the_forges_message() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/user"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0").set_body_string(" Retry later "))
            .mount(&server)
            .await;
        let err = transport(&format!("{}/api/", server.uri())).get_text("user").await.unwrap_err();
        assert_eq!(status(&err), Some(StatusCode::TOO_MANY_REQUESTS));
        assert_eq!(err.to_string(), "GET /api/user: HTTP 429 Retry later");
    }

    #[tokio::test]
    async fn a_403_with_no_request_left_is_a_refusal_unless_the_flavor_waits() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(403).insert_header("ratelimit-remaining", "0").insert_header("retry-after", "0"))
            .mount(&server)
            .await;
        let err = transport(&server.uri()).get_text("user").await.unwrap_err();
        assert_eq!(status(&err), Some(StatusCode::FORBIDDEN));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[test]
    fn the_token_stays_on_the_base_scheme_host_and_port() {
        let transport = transport("https://gitlab.com/api/v4/");
        assert_eq!(transport.url("/user").unwrap().as_str(), "https://gitlab.com/api/v4/user");
        for elsewhere in ["https://evil.example/api/v4/user", "http://gitlab.com/api/v4/user", "https://gitlab.com:8443/api/v4/user"] {
            let err = transport.guard(Url::parse(elsewhere).unwrap()).unwrap_err().to_string();
            assert!(err.starts_with("refusing to send the token"), "{elsewhere}: {err}");
        }
        assert!(transport.url("https://evil.example/x").is_err());
        assert!(transport.guard(Url::parse("https://gitlab.com:443/api/v4/user").unwrap()).is_ok());
    }

    #[tokio::test]
    async fn a_next_link_on_another_scheme_is_refused() {
        let server = MockServer::start().await;
        let next = server.uri().replacen("http://", "https://", 1) + "/api/items?page=2";
        Mock::given(method("GET"))
            .and(path("/api/items"))
            .respond_with(ResponseTemplate::new(200).insert_header("link", format!("<{next}>; rel=\"next\"")).set_body_string("[1]"))
            .mount(&server)
            .await;
        let err = transport(&format!("{}/api/", server.uri())).get_all::<u8>("items").await.unwrap_err().to_string();
        assert!(err.starts_with("refusing to send the token to https://"), "{err}");
    }

    #[test]
    fn next_link_is_found_among_the_relations() {
        let header = r#"<https://gitlab.com/api/v4/x?page=2>; rel="next", <https://gitlab.com/api/v4/x?page=1>; rel="first""#;
        assert_eq!(next_link(header), Some("https://gitlab.com/api/v4/x?page=2"));
        assert_eq!(next_link(r#"<https://gitlab.com/api/v4/x?page=1>; rel="first""#), None);
    }

    #[test]
    fn wait_prefers_retry_after_then_reset_then_a_second() {
        let transport = transport("https://gitlab.com/api/v4/");
        let mut headers = HeaderMap::new();
        assert_eq!(transport.wait_from(&headers), Duration::from_secs(1));
        let soon = budget::unix_now() + 30;
        headers.insert("ratelimit-reset", HeaderValue::from_str(&soon.to_string()).unwrap());
        assert!((28..=30).contains(&transport.wait_from(&headers).as_secs()));
        headers.insert("retry-after", HeaderValue::from_static("5"));
        assert_eq!(transport.wait_from(&headers), Duration::from_secs(5));
    }
}
