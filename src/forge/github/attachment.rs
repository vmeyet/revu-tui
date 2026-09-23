//! Pictures in GitHub comments: uploaded attachments on the web host, user-content links, and
//! files of the repo. Each goes where its token may go, or goes without one.
use super::Client;
use crate::forge::image;
use anyhow::{Context, Result, bail};
use reqwest::header::{ACCEPT, LOCATION};
use url::Url;

/// What fetching a comment picture needs besides the API client.
#[derive(Clone, Debug)]
pub struct Downloads {
    /// The web host, where attachment links live: the token goes there for those links only.
    web: Url,
    /// Signed links an attachment redirects to are fetched with this client, which has no token.
    client: reqwest::Client,
    /// Which redirect targets `client` may follow: GitHub's user content in real use.
    pub(super) follow: fn(&Url) -> bool,
}

impl Downloads {
    /// For tests: any redirect is followed, since the mock server stands in for GitHub's user content.
    #[cfg(test)]
    pub(super) fn trusting(&self) -> Self {
        Self { follow: |_| true, ..self.clone() }
    }

    pub(super) fn new(web: &str) -> Result<Self> {
        Ok(Self { web: Url::parse(web).context("host is not a hostname")?, client: image::bare_client()?, follow: image::signed_host })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Link {
    /// `https://<host>/user-attachments/assets/<id>` or the older `/<owner>/<repo>/assets/<n>/<id>`:
    /// the web host answers the token with a redirect to a signed link.
    Attachment(String),
    /// A `*.githubusercontent.com` link that needs no token: public, or signed in its query.
    Open(Url),
    /// A file of a repo on this host (`blob/<ref>/…?raw=true`, `raw/<ref>/…`, raw.githubusercontent):
    /// read through the contents API, where the token belongs.
    File { repo: String, git_ref: String, path: String },
}

impl Client {
    /// The bytes of a picture a comment points at; links elsewhere are not fetched.
    pub async fn image(&self, url: &str) -> Result<Vec<u8>> {
        let Some(link) = Link::parse(url, &self.host) else { bail!("not a picture on {}", self.host) };
        match link {
            Link::Attachment(path) => self.attachment(&path).await,
            Link::Open(url) => image::read(self.downloads.client.get(url).send().await.map_err(super::scrub)?).await,
            Link::File { repo, git_ref, path } => {
                let url = self.url(&format!("repos/{repo}/contents/{path}?ref={git_ref}"))?;
                let response = self.http.get(url).header(ACCEPT, super::RAW).send().await.map_err(super::scrub)?;
                image::read(response).await
            }
        }
    }

    /// The token goes to the web host only; the signed link it redirects to is fetched without it,
    /// since the main client refuses redirects so the token can never follow one.
    async fn attachment(&self, path: &str) -> Result<Vec<u8>> {
        let url = self.downloads.web.join(path.trim_start_matches('/'))?;
        let response = self.http.get(url).header(ACCEPT, "image/*").send().await.map_err(super::scrub)?;
        if !response.status().is_redirection() {
            return image::read(response).await;
        }
        let location = response.headers().get(LOCATION).and_then(|v| v.to_str().ok()).context("a redirect without a location")?;
        let signed = Url::parse(location).context("a redirect to a bad link")?;
        if !(self.downloads.follow)(&signed) {
            bail!("refusing to follow a redirect to {}", signed.host_str().unwrap_or("?"));
        }
        image::read(self.downloads.client.get(signed).send().await.map_err(super::scrub)?).await
    }
}

impl Link {
    fn parse(url: &str, host: &str) -> Option<Self> {
        let parsed = Url::parse(url).ok()?;
        let url_host = parsed.host_str()?;
        let segments: Vec<&str> = parsed.path_segments()?.filter(|s| !s.is_empty()).collect();
        if url_host == host || url_host == format!("www.{host}") {
            return Self::on_web(&parsed, &segments);
        }
        if host == "github.com" && url_host == "raw.githubusercontent.com" {
            let [owner, repo, git_ref, path @ ..] = segments.as_slice() else { return None };
            return file(owner, repo, git_ref, path);
        }
        if parsed.scheme() == "https" && url_host.ends_with(".githubusercontent.com") {
            return Some(Link::Open(parsed));
        }
        None
    }

    fn on_web(parsed: &Url, segments: &[&str]) -> Option<Self> {
        match segments {
            ["user-attachments", "assets", id] => Some(Link::Attachment(format!("user-attachments/assets/{id}"))),
            [owner, repo, "assets", n, id] => Some(Link::Attachment(format!("{owner}/{repo}/assets/{n}/{id}"))),
            [owner, repo, "raw", git_ref, path @ ..] => file(owner, repo, git_ref, path),
            [owner, repo, "blob", git_ref, path @ ..] if parsed.query_pairs().any(|(k, v)| k == "raw" && v == "true") => {
                file(owner, repo, git_ref, path)
            }
            _ => None,
        }
    }
}

fn file(owner: &str, repo: &str, git_ref: &str, path: &[&str]) -> Option<Link> {
    let safe = |s: &str| !s.is_empty() && s != ".." && s != ".";
    if path.is_empty() || ![owner, repo, git_ref].into_iter().chain(path.iter().copied()).all(safe) {
        return None;
    }
    Some(Link::File { repo: format!("{owner}/{repo}"), git_ref: git_ref.to_owned(), path: path.join("/") })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::auth::Credentials;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    fn png() -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        ::image::DynamicImage::new_rgb8(2, 2).write_to(&mut out, ::image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn file_link(repo: &str, git_ref: &str, path: &str) -> Link {
        Link::File { repo: repo.into(), git_ref: git_ref.into(), path: path.into() }
    }

    #[test]
    fn links_sort_into_attachments_open_links_and_repo_files() {
        let parse = |url: &str| Link::parse(url, "github.com");
        assert_eq!(
            parse("https://github.com/user-attachments/assets/1f2e-3d"),
            Some(Link::Attachment("user-attachments/assets/1f2e-3d".into()))
        );
        assert_eq!(parse("https://github.com/acme/widgets/assets/7/1f2e"), Some(Link::Attachment("acme/widgets/assets/7/1f2e".into())));
        let signed = "https://private-user-images.githubusercontent.com/1/2.png?jwt=abc";
        assert_eq!(parse(signed), Some(Link::Open(Url::parse(signed).unwrap())));
        assert_eq!(
            parse("https://github.com/acme/widgets/blob/main/docs/a.png?raw=true"),
            Some(file_link("acme/widgets", "main", "docs/a.png"))
        );
        assert_eq!(parse("https://github.com/acme/widgets/raw/feat/a.png"), Some(file_link("acme/widgets", "feat", "a.png")));
        assert_eq!(parse("https://raw.githubusercontent.com/acme/widgets/main/a.png"), Some(file_link("acme/widgets", "main", "a.png")));
    }

    #[test]
    fn other_links_are_not_fetched() {
        let parse = |url: &str| Link::parse(url, "github.com");
        assert_eq!(parse("https://img.shields.io/badge.svg"), None);
        assert_eq!(parse("https://github.com/acme/widgets/blob/main/a.png"), None, "a blob page is html, not the file");
        assert_eq!(parse("http://user-images.githubusercontent.com/1.png"), None);
        assert_eq!(parse("https://github.com.evil.example/user-attachments/assets/1"), None);
        assert_eq!(parse("https://raw.githubusercontent.com/acme/widgets/main/../x.png"), None);
    }

    #[tokio::test]
    async fn an_attachment_takes_the_token_to_the_web_host_and_follows_the_redirect_without_it() {
        let server = MockServer::start().await;
        let signed = format!("{}/signed/chart.png?X-Amz-Signature=abc", server.uri());
        Mock::given(method("GET"))
            .and(path("/user-attachments/assets/1f2e"))
            .and(header("authorization", "Bearer ghp_xxxx"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", signed.as_str()))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/signed/chart.png"))
            .respond_with(ResponseTemplate::new(200).insert_header("content-type", "image/png").set_body_bytes(png()))
            .mount(&server)
            .await;
        let client = Client::with_base(&Credentials { host: "github.com".into(), token: "ghp_xxxx".into() }, &server.uri()).unwrap();
        assert_eq!(client.image("https://github.com/user-attachments/assets/1f2e").await.unwrap(), png());
        let requests: Vec<Request> = server.received_requests().await.unwrap();
        let second = requests.iter().find(|r| r.url.path() == "/signed/chart.png").unwrap();
        assert!(second.headers.get("authorization").is_none(), "the signed link never sees the token");
    }

    #[tokio::test]
    async fn a_redirect_off_github_is_refused() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user-attachments/assets/1f2e"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", "https://evil.example/steal.png"))
            .mount(&server)
            .await;
        let client = Client::with_base(&Credentials { host: "github.com".into(), token: "ghp_xxxx".into() }, &server.uri()).unwrap();
        let downloads = Downloads { follow: crate::forge::image::signed_host, ..(*client.downloads).clone() };
        let client = Client { downloads: std::sync::Arc::new(downloads), ..client };
        let err = client.image("https://github.com/user-attachments/assets/1f2e").await.unwrap_err().to_string();
        assert!(err.contains("evil.example"), "{err}");
    }

    #[tokio::test]
    async fn a_repo_file_is_read_through_the_contents_api() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/widgets/contents/docs/a.png"))
            .and(header("accept", super::super::RAW))
            .respond_with(ResponseTemplate::new(200).insert_header("content-type", "application/vnd.github.raw").set_body_bytes(png()))
            .mount(&server)
            .await;
        let client = Client::with_base(&Credentials { host: "github.com".into(), token: "ghp_xxxx".into() }, &server.uri()).unwrap();
        assert_eq!(client.image("https://github.com/acme/widgets/blob/main/docs/a.png?raw=true").await.unwrap(), png());
    }
}
