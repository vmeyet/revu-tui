pub mod types;

pub use types::User;

use crate::auth::Credentials;
use anyhow::{Context, Result, bail};
use reqwest::header::{HeaderMap, HeaderValue};
use serde::de::DeserializeOwned;
use url::Url;

const TOKEN_HEADER: &str = "PRIVATE-TOKEN";

/// One GitLab host. The token is sent to that host and nowhere else: redirects are refused
/// and every URL is checked against `host` before the header goes on.
#[derive(Clone, Debug)]
pub struct Client {
    http: reqwest::Client,
    host: String,
    base: Url,
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
        Ok(Self { http, host: credentials.host.clone(), base })
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

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = self.url(path)?;
        let response = self.http.get(url).send().await.map_err(scrub)?;
        let status = response.status();
        if !status.is_success() {
            let message = response.text().await.unwrap_or_default();
            bail!("GET {path}: HTTP {} {}", status.as_u16(), types::error_message(&message));
        }
        response.json().await.map_err(scrub).with_context(|| format!("GET {path}: unreadable answer"))
    }

    pub async fn me(&self) -> Result<User> {
        self.get("user").await
    }

    fn url(&self, path: &str) -> Result<Url> {
        let url = self.base.join(path.trim_start_matches('/'))?;
        if url.host_str() != Some(self.host.as_str()) {
            bail!("refusing to send the token to {}", url.host_str().unwrap_or("?"));
        }
        Ok(url)
    }
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

    #[tokio::test]
    async fn me_sends_the_token_header() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/user"))
            .and(header(TOKEN_HEADER, "glpat-xxxx"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": 7, "username": "nina", "name": "Nina", "avatar_url": null
            })))
            .mount(&server)
            .await;
        let client = Client::with_base(&creds(), &format!("{}/api/v4/", server.uri())).unwrap();
        let me = client.me().await.unwrap();
        assert_eq!(me.username, "nina");
        assert_eq!(me.id, 7);
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

    #[test]
    fn token_never_leaves_the_host() {
        let client = Client::new(&creds()).unwrap();
        let err = client.url("https://evil.example/api/v4/user").unwrap_err().to_string();
        assert!(err.contains("evil.example"), "{err}");
    }
}
