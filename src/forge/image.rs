//! Fetching a picture a note points at: pictures only, capped in size and time, and the token
//! never leaves its own host. Each backend decides which links it may fetch; this module holds
//! the rules they share.
use anyhow::{Context, Result, bail};
use reqwest::Response;
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE};
use std::time::Duration;
use url::Url;

/// A bigger file is not a comment's picture worth a thumbnail.
pub const MAX_BYTES: usize = 5 * 1024 * 1024;
/// A picture that takes longer shows its `[image: …]` line instead.
pub const TIMEOUT: Duration = Duration::from_secs(15);

/// A client without the token and without redirects, for the signed links a forge hands out:
/// such a link carries its own right to read, so nothing else needs to travel with it.
pub fn bare_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(TIMEOUT)
        .user_agent(concat!("revu/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("building the download client")
}

/// The body of a successful answer when it is a picture no larger than [`MAX_BYTES`].
/// GitLab uploads and GitHub's raw files come untyped, so such a body counts when its bytes are an image.
pub async fn read(mut response: Response) -> Result<Vec<u8>> {
    let status = response.status();
    if !status.is_success() {
        bail!("HTTP {}", status.as_u16());
    }
    let kind = header(&response, CONTENT_TYPE).unwrap_or_default().to_ascii_lowercase();
    let typed = kind.starts_with("image/");
    let untyped = ["application/octet-stream", "binary/octet-stream", "application/vnd.github.raw"];
    if !typed && !kind.is_empty() && !untyped.iter().any(|u| kind.starts_with(u)) {
        bail!("not a picture ({kind})");
    }
    if header(&response, CONTENT_LENGTH).and_then(|n| n.parse::<usize>().ok()).is_some_and(|n| n > MAX_BYTES) {
        bail!("larger than {} MB", MAX_BYTES / 1024 / 1024);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| anyhow::anyhow!("{}", e.without_url()))? {
        if bytes.len() + chunk.len() > MAX_BYTES {
            bail!("larger than {} MB", MAX_BYTES / 1024 / 1024);
        }
        bytes.extend_from_slice(&chunk);
    }
    if !typed && image::guess_format(&bytes).is_err() {
        bail!("not a picture");
    }
    Ok(bytes)
}

/// Where GitHub sends an attachment link: its user-content hosts and the S3 bucket behind them.
pub fn signed_host(url: &Url) -> bool {
    let Some(host) = url.host_str() else { return false };
    url.scheme() == "https"
        && (host.ends_with(".githubusercontent.com")
            || (host.starts_with("github-production-user-asset-") && host.ends_with(".s3.amazonaws.com")))
}

/// The link a reader's browser opens for `url` as written in a note of `project` on `host`:
/// GitLab writes uploads relative to the project, other relative links to the host.
pub fn web_url(host: &str, project: &str, url: &str) -> String {
    if url.starts_with("http://") || url.starts_with("https://") {
        return url.to_owned();
    }
    if url.starts_with("/uploads/") {
        return format!("https://{host}/{project}{url}");
    }
    format!("https://{host}/{}", url.trim_start_matches('/'))
}

fn header(response: &Response, name: reqwest::header::HeaderName) -> Option<String> {
    response.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn png() -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(4, 2).write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    async fn served(template: ResponseTemplate) -> Result<Vec<u8>> {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/p")).respond_with(template).mount(&server).await;
        let response = bare_client().unwrap().get(format!("{}/p", server.uri())).send().await.unwrap();
        read(response).await
    }

    #[tokio::test]
    async fn pictures_are_read_and_everything_else_refused() {
        let ok = ResponseTemplate::new(200).insert_header("content-type", "image/png").set_body_bytes(png());
        assert_eq!(served(ok).await.unwrap(), png());
        let untyped = ResponseTemplate::new(200).insert_header("content-type", "application/octet-stream").set_body_bytes(png());
        assert_eq!(served(untyped).await.unwrap(), png(), "GitLab uploads come untyped");
        let html = ResponseTemplate::new(200).insert_header("content-type", "text/html").set_body_string("<html>");
        assert!(served(html).await.unwrap_err().to_string().contains("not a picture"));
        let fake = ResponseTemplate::new(200).insert_header("content-type", "application/octet-stream").set_body_bytes(b"MZ\x90".to_vec());
        assert!(served(fake).await.unwrap_err().to_string().contains("not a picture"));
        assert!(served(ResponseTemplate::new(404)).await.unwrap_err().to_string().contains("404"));
    }

    #[tokio::test]
    async fn a_picture_past_the_cap_is_refused_even_without_a_length() {
        let big = vec![0_u8; MAX_BYTES + 1];
        let template = ResponseTemplate::new(200).insert_header("content-type", "image/png").set_body_bytes(big);
        assert!(served(template).await.unwrap_err().to_string().contains("larger than 5 MB"));
    }

    #[test]
    fn only_github_user_content_and_its_bucket_are_followed_without_the_token() {
        let ok = |u: &str| signed_host(&Url::parse(u).unwrap());
        assert!(ok("https://private-user-images.githubusercontent.com/1/2.png?jwt=x"));
        assert!(ok("https://github-production-user-asset-6210df.s3.amazonaws.com/1/2.png?X-Amz-Signature=x"));
        assert!(!ok("http://private-user-images.githubusercontent.com/1.png"), "https only");
        assert!(!ok("https://evil.example/githubusercontent.com"));
        assert!(!ok("https://attacker.s3.amazonaws.com/x.png"));
    }

    #[test]
    fn relative_links_open_on_the_forge() {
        assert_eq!(
            web_url("gitlab.com", "acme/widgets", "/uploads/ab12/chart.png"),
            "https://gitlab.com/acme/widgets/uploads/ab12/chart.png"
        );
        assert_eq!(
            web_url("gitlab.com", "acme/widgets", "/-/project/7/uploads/ab12/c.png"),
            "https://gitlab.com/-/project/7/uploads/ab12/c.png"
        );
        assert_eq!(
            web_url("github.com", "acme/widgets", "https://github.com/user-attachments/assets/1f"),
            "https://github.com/user-attachments/assets/1f"
        );
    }
}
