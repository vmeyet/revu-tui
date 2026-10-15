//! The HTTP transport both forges share: the token rides on a client that follows no redirect and
//! reaches the origin of its base URL, and of its web URL when a forge sets one; lists are read page by page and a rate limit is waited
//! out once. What differs between forges is a `Flavor`.
use super::budget::{self, Budget, RateLimit};
use anyhow::{Context, Result, bail, ensure};
use reqwest::header::{ACCEPT, HeaderMap, RETRY_AFTER};
use reqwest::{Method, Response, StatusCode};
use serde::de::DeserializeOwned;
use std::fmt;
use std::time::Duration;
use url::Url;

const PAGE_SIZE: &str = "100";
/// The longest a request sleeps on a rate limit; past it, the request fails saying until when.
const MAX_WAIT: Duration = Duration::from_secs(60);
/// What GitHub asks after a secondary limit that names no wait: a minute at least.
const SECONDARY_WAIT: Duration = Duration::from_secs(60);
/// The most one request may take, its answer read whole: a slow page of diffs still fits.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// How one forge speaks HTTP where the two differ.
#[derive(Debug)]
pub struct Flavor {
    /// The header counting the requests left in the window.
    pub remaining: &'static str,
    /// The header giving the unix time the window starts again.
    pub reset: &'static str,
    /// Whether a 403 can be a rate limit rather than a refusal: with no request left, a
    /// `Retry-After`, or a message naming a secondary rate limit.
    pub limits_with_403: bool,
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
    /// The forge's web origin, which `fetch_web` alone may reach.
    web: Option<Url>,
    budget: Budget,
    flavor: &'static Flavor,
}

impl Transport {
    /// `headers` carry the token: every request goes to `base`'s scheme, host and port or not at all.
    pub fn new(base: &str, headers: HeaderMap, flavor: &'static Flavor) -> Result<Self> {
        Self::from_builder(crate::http::client().timeout(REQUEST_TIMEOUT), base, headers, flavor)
    }

    fn from_builder(client: reqwest::ClientBuilder, base: &str, headers: HeaderMap, flavor: &'static Flavor) -> Result<Self> {
        let client = client
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("revu/", env!("CARGO_PKG_VERSION")))
            .build()?;
        let base = Url::parse(base).context("host is not a hostname")?;
        Ok(Self { client, base, web: None, budget: Budget::default(), flavor })
    }

    /// Lets `fetch_web` take the token to `web` too; it keeps the base's scheme, so an `https` base
    /// never sends the token in clear.
    pub fn with_web(self, web: &str) -> Result<Self> {
        let web = Url::parse(web).context("host is not a hostname")?;
        ensure!(web.scheme() == self.base.scheme(), "refusing to send the token over {}", web.scheme());
        Ok(Self { web: Some(web), ..self })
    }

    pub fn rate(&self) -> RateLimit {
        self.budget.now()
    }

    /// `path` joined to the base, refused when it leaves the base's origin.
    pub fn url(&self, path: &str) -> Result<Url> {
        join_within(&self.base, path)
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

    /// One GET of `path` on the web origin, answered as is: a redirect comes back, never followed.
    pub async fn fetch_web(&self, path: &str, accept: &'static str) -> Result<Response> {
        let web = self.web.as_ref().context("no web host to send the token to")?;
        self.send_once(Method::GET, join_within(web, path)?, None, Some(accept)).await
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
                Some(link) => url = same_origin(&self.base, Url::parse(&link).with_context(|| format!("bad link {link:?}"))?)?,
                None => return Ok(items),
            }
        }
    }

    /// One request with the token; a non-2xx answer is an `HttpError`.
    pub async fn send(&self, method: Method, path: &str, body: Option<&serde_json::Value>) -> Result<Response> {
        self.send_to(method, self.url(path)?, body, None).await
    }

    /// Retried once after the wait the forge asks for when it rate-limits; a wait past `MAX_WAIT`
    /// fails at once, saying until when, and the budget holds that time.
    async fn send_to(&self, method: Method, url: Url, body: Option<&serde_json::Value>, accept: Option<&str>) -> Result<Response> {
        let first = self.send_once(method.clone(), url.clone(), body, accept).await?;
        if first.status().is_success() {
            return Ok(first);
        }
        let wait = Box::pin(self.limit_wait(first, &method, &url)).await?;
        self.budget.waiting_for(wait);
        if wait > MAX_WAIT {
            return Err(limited_until(wait));
        }
        tokio::time::sleep(wait).await;
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
        let response = crate::log::send(request).await.map_err(scrub)?;
        self.budget.note(budget::header_number(response.headers(), self.flavor.remaining));
        Ok(response)
    }

    /// How long a refused answer asks to wait; an answer that is no rate limit is its `HttpError`.
    async fn limit_wait(&self, refused: Response, method: &Method, url: &Url) -> Result<Duration> {
        let headers = refused.headers().clone();
        let refusal = self.refusal(refused, method, url).await;
        self.wait_asked(refusal.status, &headers, &refusal.message).ok_or_else(|| refusal.into())
    }

    /// How long a refused answer asks to wait, when it is a rate limit.
    fn wait_asked(&self, status: StatusCode, headers: &HeaderMap, message: &str) -> Option<Duration> {
        let is_secondary = names_a_secondary_limit(message);
        let asks_a_wait = headers.contains_key(RETRY_AFTER) || self.is_spent(headers) || is_secondary;
        let is_limit = match status {
            StatusCode::TOO_MANY_REQUESTS => true,
            StatusCode::FORBIDDEN => self.flavor.limits_with_403 && asks_a_wait,
            _ => false,
        };
        is_limit.then(|| self.wait_from(headers, is_secondary))
    }

    fn is_spent(&self, headers: &HeaderMap) -> bool {
        budget::header_number(headers, self.flavor.remaining) == Some(0)
    }

    async fn checked(&self, response: Response, method: &Method, url: &Url) -> Result<Response> {
        if response.status().is_success() {
            return Ok(response);
        }
        Err(self.refusal(response, method, url).await.into())
    }

    async fn refusal(&self, response: Response, method: &Method, url: &Url) -> HttpError {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        HttpError { request: format!("{method} {}", url.path()), status, message: (self.flavor.error_message)(&body) }
    }

    /// `Retry-After` in seconds; else a minute for a secondary limit with requests left, whose reset
    /// header times the other window; else the reset header as a unix time; else one second.
    fn wait_from(&self, headers: &HeaderMap, is_secondary: bool) -> Duration {
        if let Some(seconds) = budget::header_number(headers, "retry-after") {
            return Duration::from_secs(seconds);
        }
        if is_secondary && !self.is_spent(headers) {
            return SECONDARY_WAIT;
        }
        budget::header_number(headers, self.flavor.reset)
            .map_or(Duration::from_secs(1), |reset| Duration::from_secs(reset.saturating_sub(budget::unix_now())))
    }
}

fn join_within(origin: &Url, path: &str) -> Result<Url> {
    same_origin(origin, origin.join(path.trim_start_matches('/'))?)
}

/// The origin, not the host alone: a host would let an `http://` link or another port take the token.
fn same_origin(allowed: &Url, url: Url) -> Result<Url> {
    if url.origin() != allowed.origin() {
        bail!("refusing to send the token to {}", url.origin().ascii_serialization());
    }
    Ok(url)
}

fn names_a_secondary_limit(message: &str) -> bool {
    message.to_lowercase().contains("secondary rate limit")
}

/// Said in local time, to the minute, as the reader reads a clock.
fn limited_until(wait: Duration) -> anyhow::Error {
    let until = chrono::Local::now() + wait;
    anyhow::anyhow!("rate limited until {}", until.format("%H:%M"))
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
/// above the transport can leak a request. A timeout says so, which the cut chain would hide.
pub fn scrub(err: reqwest::Error) -> anyhow::Error {
    let timed_out = if err.is_timeout() { ": timed out" } else { "" };
    anyhow::anyhow!("{}{timed_out}", err.without_url())
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
        limits_with_403: false,
        error_message: |b| b.trim().to_owned(),
    };

    static LIMITS_WITH_403: Flavor = Flavor { limits_with_403: true, ..PLAIN };

    fn transport(base: &str) -> Transport {
        Transport::new(base, HeaderMap::new(), &PLAIN).unwrap()
    }

    fn limiting(base: &str) -> Transport {
        Transport::new(base, HeaderMap::new(), &LIMITS_WITH_403).unwrap()
    }

    #[tokio::test]
    async fn a_secondary_limit_403_with_requests_left_is_waited_out_and_retried() {
        let server = MockServer::start().await;
        let limited = ResponseTemplate::new(403).insert_header("ratelimit-remaining", "4999").insert_header("retry-after", "0");
        Mock::given(method("GET")).and(path("/api/user")).respond_with(limited).up_to_n_times(1).mount(&server).await;
        Mock::given(method("GET"))
            .and(path("/api/user"))
            .respond_with(ResponseTemplate::new(200).set_body_string("nina"))
            .mount(&server)
            .await;
        assert_eq!(limiting(&format!("{}/api/", server.uri())).get_text("user").await.unwrap(), "nina");
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_wait_past_a_minute_fails_at_once_saying_until_when_and_the_budget_holds_it() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(403).insert_header("ratelimit-remaining", "12").insert_header("retry-after", "3600"))
            .mount(&server)
            .await;
        let transport = limiting(&server.uri());
        let clock =
            |at: chrono::DateTime<chrono::Local>| format!("rate limited until {}", (at + Duration::from_secs(3600)).format("%H:%M"));
        let before = chrono::Local::now();
        let err = transport.get_text("user").await.unwrap_err().to_string();
        assert!([clock(before), clock(chrono::Local::now())].contains(&err), "{err}");
        assert_eq!(server.received_requests().await.unwrap().len(), 1, "no sleep, no retry");
        assert!(transport.rate().wait.is_some_and(|wait| wait > MAX_WAIT));
    }

    #[test]
    fn a_403_is_a_rate_limit_when_it_asks_a_wait_or_names_a_secondary_limit() {
        let headers = |pairs: &[(&'static str, &'static str)]| {
            pairs.iter().map(|(name, value)| (reqwest::header::HeaderName::from_static(name), HeaderValue::from_static(value))).collect()
        };
        let limiting = limiting("https://github.com/");
        let wait = |headers: &HeaderMap, message: &str| limiting.wait_asked(StatusCode::FORBIDDEN, headers, message);
        assert_eq!(wait(&headers(&[("retry-after", "30"), ("ratelimit-remaining", "4000")]), ""), Some(Duration::from_secs(30)));
        assert_eq!(wait(&HeaderMap::new(), "You have exceeded a secondary rate limit."), Some(SECONDARY_WAIT));
        assert_eq!(wait(&HeaderMap::new(), "Resource not accessible by integration"), None);
        let soon = budget::unix_now() + 3000;
        let reset = HeaderValue::from_str(&soon.to_string()).unwrap();
        let other_window = [(reqwest::header::HeaderName::from_static("ratelimit-reset"), reset)].into_iter().collect();
        let secondary_429 = limiting.wait_asked(StatusCode::TOO_MANY_REQUESTS, &other_window, "secondary rate limit");
        assert_eq!(secondary_429, Some(SECONDARY_WAIT), "the reset of the hourly window is not the secondary limit's");
        let plain = transport("https://gitlab.com/");
        assert_eq!(plain.wait_asked(StatusCode::FORBIDDEN, &headers(&[("retry-after", "30")]), ""), None, "a GitLab 403 is a refusal");
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
            let err = same_origin(&transport.base, Url::parse(elsewhere).unwrap()).unwrap_err().to_string();
            assert!(err.starts_with("refusing to send the token"), "{elsewhere}: {err}");
        }
        assert!(transport.url("https://evil.example/x").is_err());
        assert!(same_origin(&transport.base, Url::parse("https://gitlab.com:443/api/v4/user").unwrap()).is_ok());
    }

    #[tokio::test]
    async fn the_web_host_gets_the_token_through_fetch_web_only_and_its_redirect_is_not_followed() {
        let (api, web, elsewhere) = (MockServer::start().await, MockServer::start().await, MockServer::start().await);
        let away = format!("{}/stolen", elsewhere.uri());
        Mock::given(method("GET"))
            .and(path("/user-attachments/assets/1f2e"))
            .and(wiremock::matchers::header("authorization", "Bearer ghp_xxxx"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", away.as_str()))
            .mount(&web)
            .await;
        let token = [(reqwest::header::AUTHORIZATION, HeaderValue::from_static("Bearer ghp_xxxx"))].into_iter().collect();
        let transport = Transport::new(&api.uri(), token, &PLAIN).unwrap().with_web(&web.uri()).unwrap();
        let answer = transport.fetch_web("/user-attachments/assets/1f2e", "image/*").await.unwrap();
        assert_eq!(answer.status(), StatusCode::FOUND);
        assert!(transport.get_text(&format!("{}/user", web.uri())).await.is_err(), "the API calls stay on the API host");
        assert!(transport.fetch_web(&away, "image/*").await.is_err(), "fetch_web stays on the web host");
        assert_eq!(web.received_requests().await.unwrap().len(), 1);
        assert!(elsewhere.received_requests().await.unwrap().is_empty());
    }

    #[test]
    fn a_web_host_on_another_scheme_than_the_base_is_refused() {
        assert!(transport("https://api.github.com/").with_web("http://github.com/").is_err());
        assert!(transport("https://api.github.com/").with_web("https://github.com/").is_ok());
    }

    #[tokio::test]
    async fn without_a_web_host_fetch_web_sends_nothing() {
        let err = transport("https://gitlab.com/api/v4/").fetch_web("x.png", "image/*").await.unwrap_err().to_string();
        assert_eq!(err, "no web host to send the token to");
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

    #[tokio::test]
    async fn a_forge_that_never_answers_fails_the_request_instead_of_holding_it() {
        let base = crate::http::tests::silent_server().await;
        let quick = crate::http::client().read_timeout(Duration::from_millis(200));
        let transport = Transport::from_builder(quick, &base, HeaderMap::new(), &PLAIN).unwrap();
        let answer = tokio::time::timeout(Duration::from_secs(5), transport.get_text("user")).await.expect("the request ended");
        let err = answer.unwrap_err().to_string();
        assert!(err.contains("timed out"), "{err}");
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
        assert_eq!(transport.wait_from(&headers, false), Duration::from_secs(1));
        let soon = budget::unix_now() + 30;
        headers.insert("ratelimit-reset", HeaderValue::from_str(&soon.to_string()).unwrap());
        assert!((28..=30).contains(&transport.wait_from(&headers, false).as_secs()));
        headers.insert("retry-after", HeaderValue::from_static("5"));
        assert_eq!(transport.wait_from(&headers, false), Duration::from_secs(5));
    }
}
