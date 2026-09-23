//! Pictures in GitLab notes are uploads: `/uploads/<secret>/<file>` under a project. The web link
//! needs a browser session on a private project, so revu asks the uploads API with the token.
use super::Client;
use crate::forge::image;
use anyhow::{Result, bail};
use reqwest::Method;

/// Whose upload a link names: the MR's own project unless the link says otherwise.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Owner {
    Mr,
    Path(String),
    Id(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Upload {
    owner: Owner,
    secret: String,
    file: String,
}

impl Client {
    /// The bytes of a picture a note in `project` points at; only this host's uploads are fetched.
    pub async fn image(&self, project: &str, url: &str) -> Result<Vec<u8>> {
        let Some(upload) = Upload::parse(url, &self.host) else { bail!("not an upload on {}", self.host) };
        let owner = match &upload.owner {
            Owner::Mr => super::rest::project_path(project),
            Owner::Path(path) => super::rest::project_path(path),
            Owner::Id(id) => format!("projects/{id}"),
        };
        let path = format!("{owner}/uploads/{}/{}", upload.secret, upload.file);
        let response = self.send(Method::GET, self.url(&path)?, None).await?;
        image::read(response).await
    }
}

impl Upload {
    /// `/uploads/<secret>/<file>`, `/-/project/<id>/uploads/…`, or either written in full on `host`.
    fn parse(url: &str, host: &str) -> Option<Self> {
        let path = match url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")) {
            Some(rest) => {
                let (url_host, path) = rest.split_once('/')?;
                if url_host.split(':').next()? != host {
                    return None;
                }
                path
            }
            None => url.strip_prefix('/')?,
        };
        let path = path.split(['?', '#']).next()?;
        let parts: Vec<&str> = path.split('/').collect();
        let at = parts.iter().position(|p| *p == "uploads")?;
        let [secret, file] = parts.get(at + 1..)? else { return None };
        if secret.len() < 16 || !secret.bytes().all(|b| b.is_ascii_hexdigit()) || file.is_empty() || *file == ".." || *file == "." {
            return None;
        }
        let owner = match &parts[..at] {
            [] => Owner::Mr,
            ["-", "project", id] => Owner::Id(id.parse().ok()?),
            project if project.iter().all(|p| !p.is_empty() && *p != "-" && *p != "..") => Owner::Path(project.join("/")),
            _ => return None,
        };
        Some(Self { owner, secret: (*secret).to_owned(), file: (*file).to_owned() })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::auth::Credentials;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const SECRET: &str = "bfa2cc58a8b33cc5149cbc6144d7058e";

    fn upload(owner: Owner) -> Upload {
        Upload { owner, secret: SECRET.into(), file: "chart.png".into() }
    }

    #[test]
    fn upload_links_resolve_against_the_project_or_the_one_they_name() {
        let parse = |url: &str| Upload::parse(url, "gitlab.com");
        assert_eq!(parse(&format!("/uploads/{SECRET}/chart.png")), Some(upload(Owner::Mr)));
        assert_eq!(parse(&format!("/-/project/42/uploads/{SECRET}/chart.png")), Some(upload(Owner::Id(42))));
        assert_eq!(
            parse(&format!("https://gitlab.com/acme/tools/uploads/{SECRET}/chart.png")),
            Some(upload(Owner::Path("acme/tools".into())))
        );
        assert_eq!(parse(&format!("https://gitlab.com/-/project/42/uploads/{SECRET}/chart.png?x=1")), Some(upload(Owner::Id(42))));
    }

    #[test]
    fn anything_else_is_not_fetched() {
        let parse = |url: &str| Upload::parse(url, "gitlab.com");
        assert_eq!(parse(&format!("https://evil.example/acme/uploads/{SECRET}/chart.png")), None, "another host never gets the token");
        assert_eq!(parse("https://img.shields.io/badge.svg"), None);
        assert_eq!(parse(&format!("/uploads/{SECRET}/../../etc")), None);
        assert_eq!(parse(&format!("/uploads/{SECRET}/..")), None);
        assert_eq!(parse("/uploads/not-a-secret/chart.png"), None);
        assert_eq!(parse(&format!("/../uploads/{SECRET}/chart.png")), None);
    }

    #[tokio::test]
    async fn a_relative_upload_is_asked_through_the_api_with_the_token() {
        let server = MockServer::start().await;
        let mut png = std::io::Cursor::new(Vec::new());
        ::image::DynamicImage::new_rgb8(2, 2).write_to(&mut png, ::image::ImageFormat::Png).unwrap();
        Mock::given(method("GET"))
            .and(path(format!("/api/v4/projects/acme%2Fwidgets/uploads/{SECRET}/chart.png")))
            .and(header("PRIVATE-TOKEN", "glpat-xxxx"))
            .respond_with(
                ResponseTemplate::new(200).insert_header("content-type", "application/octet-stream").set_body_bytes(png.get_ref().clone()),
            )
            .mount(&server)
            .await;
        let client =
            Client::with_base(&Credentials { host: "x".into(), token: "glpat-xxxx".into() }, &format!("{}/api/v4/", server.uri())).unwrap();
        let bytes = client.image("acme/widgets", &format!("/uploads/{SECRET}/chart.png")).await.unwrap();
        assert_eq!(bytes, png.into_inner());
        assert!(client.image("acme/widgets", "https://img.shields.io/badge.svg").await.is_err());
    }
}
