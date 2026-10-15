//! What the forge serves beside the diff: files at a commit, for the viewer, the outline and prose, and pictures in notes.
use super::{Homes, read_cache, run_blocking, warn_stopped, write_cache};
use crate::blocking;
use crate::cache::keys;
use crate::forge::{MrKey, Sha};
use crate::log::OrWarn as _;
use crate::tui::app::{Incoming, Versions};
use crate::tui::images;
use anyhow::{Context as _, Result};
use futures_util::{StreamExt, TryStreamExt};
use std::sync::Arc;

/// Files the outline reads from the forge at once, so a large MR does not burst into its rate limit.
const OUTLINE_READS: usize = 8;

#[derive(Clone)]
pub(in crate::tui) struct Files {
    pub(super) homes: Arc<Homes>,
    pub(super) open: crate::config::Open,
    /// The checkout `revu` runs in, when its origin is on this forge: `v` opens its real files.
    pub(super) checkout: Option<crate::open::Checkout>,
}

impl Files {
    /// The file ready for the reader's program: the checkout's own when it sits on `sha`,
    /// else a private read-only copy of what the forge serves.
    pub(in crate::tui) async fn view(&self, key: MrKey, path: &str, sha: &Sha, line: u32, note: Option<String>) -> Result<Incoming> {
        crate::open::safe_path(path)?;
        let checkout = match self.checkout.as_ref().filter(|_| key.host.is_none()) {
            Some(checkout) => checkout.head().await.map(|head| (checkout, head)),
            None => None,
        };
        let source = crate::open::Source::pick(&key.project, sha.as_str(), checkout.as_ref().map(|(c, head)| (&c.project, head.as_str())));
        let (file, dir, note) = if let (crate::open::Source::Checkout, Some((checkout, _))) = (source, checkout) {
            (checkout.root.join(path), None, Some("your checkout · edits are real".to_owned()))
        } else {
            let text = self.file_text(&key, path, sha).await?;
            let name = path.to_owned();
            let (dir, file) = blocking::run(move || crate::open::write_private(&name, text.as_bytes())).await?;
            (file, Some(std::sync::Arc::new(dir)), note)
        };
        let command = crate::open::command_for(path, &self.open, env("VISUAL").as_deref(), env("EDITOR").as_deref());
        let argv = crate::open::argv(&command, &file.display().to_string(), line, dir.is_some())?;
        let shown = format!("{}:{line}", path.rsplit('/').next().unwrap_or(path));
        Ok(Incoming::ViewReady { key, view: crate::open::View { argv, shown, note, _copy: dir } })
    }

    /// The symbols the files change and the calls between them, a few files read at once.
    pub(in crate::tui) async fn outline(
        &self,
        key: &MrKey,
        base: &Sha,
        head: &Sha,
        files: Vec<crate::outline::Sides>,
    ) -> Result<crate::outline::Reading> {
        let files: Vec<(String, String, String)> = futures_util::stream::iter(files)
            .map(|sides| async move {
                let path = sides.head.clone().or_else(|| sides.base.clone()).unwrap_or_default();
                let (old, new) = futures_util::try_join!(self.side_text(key, sides.base, base), self.side_text(key, sides.head, head))?;
                Ok::<_, anyhow::Error>((path, old, new))
            })
            .buffered(OUTLINE_READS)
            .try_collect()
            .await?;
        blocking::run(move || Ok(crate::outline::read(&files))).await
    }

    /// The Markdown file the prose view shows, at base and at head.
    pub(in crate::tui) async fn prose(&self, key: &MrKey, base: &Sha, head: &Sha, sides: crate::outline::Sides) -> Result<Versions> {
        let (base, head) = futures_util::try_join!(self.side_text(key, sides.base, base), self.side_text(key, sides.head, head))?;
        Ok(Versions { base, head })
    }

    /// The file at `sha`, empty when it does not exist on that side.
    async fn side_text(&self, key: &MrKey, path: Option<String>, sha: &Sha) -> Result<String> {
        match path {
            Some(path) => self.file_text(key, &path, sha).await,
            None => Ok(String::new()),
        }
    }

    /// A file at a commit never changes, so the cache serves it forever.
    async fn file_text(&self, key: &MrKey, path: &str, sha: &Sha) -> Result<String> {
        let cache_key = keys::file(key, sha, path);
        let (wanted, reading) = (key.clone(), cache_key.clone());
        if let Some(text) = read_cache(&self.homes, move |homes| homes.cache_of(&wanted).read::<String>(&reading)).await {
            return Ok(text);
        }
        let short = sha.as_str().get(..8).unwrap_or(sha.as_str());
        let text = self.homes.forge_of(key).file(key, path, sha).await.with_context(|| format!("{path} at {short} · v tries again"))?;
        anyhow::ensure!(
            text.len() <= crate::open::MAX_BYTES,
            "too large to open here ({} MB) · o opens it in the browser",
            text.len() / (1024 * 1024)
        );
        let (wanted, saved) = (key.clone(), text.clone());
        write_cache("caching the file", &self.homes, move |homes| homes.cache_of(&wanted).write(&cache_key, &saved)).await;
        Ok(text)
    }

    /// A picture from the disk cache, else from the forge and then kept; decoded off the loop.
    /// Only a picture that decodes is kept, so a failed one is asked again next time.
    pub(in crate::tui) async fn image(&self, key: &MrKey, url: &str) -> Option<image::DynamicImage> {
        let (owner, name) = (key.clone(), keys::image(url));
        let cached = read_cache(&self.homes, move |homes| homes.cache_of(&owner).read_bytes(&name)).await;
        let fresh = cached.is_none();
        let bytes = match cached {
            Some(bytes) => bytes,
            // Not logged: the error carries the URL, whose query may hold a signed link's secret; the request line is logged already.
            None => self.homes.forge_of(key).image(key, url).await.ok()?,
        };
        let (owner, name) = (key.clone(), keys::image(url));
        run_blocking(&self.homes, move |homes| {
            let picture = images::decode(&bytes)?;
            if fresh {
                homes.cache_of(&owner).write_bytes(&name, &bytes).or_warn("caching a picture");
            }
            Some(picture)
        })
        .await
        .inspect_err(warn_stopped)
        .ok()
        .flatten()
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::forge::gitlab::fixture::key;
    use crate::tui::backend::test_support::homes_on;
    use std::collections::BTreeMap;

    #[tokio::test]
    async fn a_viewed_file_is_fetched_once_copied_read_only_and_handed_to_the_program() {
        use std::os::unix::fs::PermissionsExt;
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/acme%2Fwidgets/repository/files/src%2Fpay%2Fcharge.rs/raw"))
            .and(query_param("ref", "bbbb"))
            .respond_with(ResponseTemplate::new(200).set_body_string("fn charge() {}\n"))
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let open = crate::config::Open { default: Some("nvim".into()), files: BTreeMap::new() };
        let files = Files { homes: homes_on(&server, dir.path()).into(), open, checkout: None };
        for _ in 0..2 {
            let Incoming::ViewReady { view, .. } = files.view(key(), "src/pay/charge.rs", &"bbbb".into(), 12, None).await.unwrap() else {
                panic!("not ready")
            };
            let file = std::path::Path::new(&view.argv[3]);
            assert_eq!(view.argv[..3], ["nvim", "-R", "+12"], "a copy opens read-only");
            assert_eq!(std::fs::read_to_string(file).unwrap(), "fn charge() {}\n");
            assert_eq!(std::fs::metadata(file).unwrap().permissions().mode() & 0o777, 0o400);
            assert_eq!(view.shown, "charge.rs:12");
        }
        let refused = files.view(key(), "../../etc/passwd", &"bbbb".into(), 1, None).await.unwrap_err();
        assert!(refused.to_string().contains("does not trust"), "{refused}");
    }
}
