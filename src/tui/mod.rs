pub mod app;
pub mod diff_view;
pub mod theme;
pub mod thread_view;
pub mod ui;

use crate::api::Client;
use crate::cache::{Cache, Entry, keys};
use crate::ctx::Ctx;
use crate::diff::fold::FoldState;
use crate::review::Review;
use anyhow::{Context as _, Result};
use app::{Action, App, Failure, Incoming, MrKey, Settings};
use chrono::{DateTime, Utc};
use crossterm::event::{Event, EventStream, KeyEventKind};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

const TICK: Duration = Duration::from_millis(100);

/// What survives between two openings of one MR, in the cache.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct MrState {
    #[serde(default)]
    fold: FoldState,
    #[serde(default)]
    viewed: BTreeSet<String>,
    #[serde(default)]
    opened_at: Option<DateTime<Utc>>,
}

#[derive(Clone)]
struct Backend {
    gitlab: Client,
    cache: Cache,
    fold_globs: Vec<String>,
    watch_labels: Vec<String>,
}

pub async fn run(ctx: Ctx) -> Result<()> {
    let theme = match ctx.config.tui.theme.as_deref() {
        Some(name) => theme::Theme::named(name)
            .ok_or_else(|| anyhow::anyhow!("config `tui.theme = \"{name}\"` is not a theme (try {})", theme::Theme::NAMES.join(", ")))?,
        None => theme::Theme::default(),
    };
    let backend =
        Backend { gitlab: ctx.gitlab.clone(), cache: Cache::for_host(ctx.gitlab.host()), fold_globs: vec![], watch_labels: vec![] };
    let settings = Settings {
        theme,
        host: ctx.gitlab.host().to_owned(),
        me: ctx.config.username.clone().unwrap_or_default(),
        fold_globs: backend.fold_globs.clone(),
        watch_labels: backend.watch_labels.clone(),
    };
    let mut app = App::new(settings);
    let mut terminal = ratatui::init();
    let outcome = event_loop(&mut terminal, &mut app, &backend).await;
    ratatui::restore();
    outcome
}

async fn event_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut App, backend: &Backend) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Incoming>();
    let mut events = EventStream::new();
    let mut ticks = tokio::time::interval(TICK);
    for action in app.start() {
        spawn(action, backend, tx.clone());
    }
    while !app.should_quit {
        terminal.draw(|f| ui::draw(f, app))?;
        let actions = tokio::select! {
            Some(event) = events.next() => match event? {
                Event::Key(key) if key.kind != KeyEventKind::Release => app.handle_key(key),
                _ => vec![],
            },
            Some(incoming) = rx.recv() => { app.apply(incoming); vec![] }
            _ = ticks.tick() => {
                app.now = Instant::now();
                app.today = Utc::now();
                app.tick()
            }
        };
        for action in actions {
            spawn(action, backend, tx.clone());
        }
    }
    Ok(())
}

/// Every action runs in its own task and answers through `Incoming`; the loop never awaits the network.
fn spawn(action: Action, backend: &Backend, tx: mpsc::UnboundedSender<Incoming>) {
    let backend = backend.clone();
    tokio::spawn(async move {
        let send = |incoming: Incoming| {
            let _ = tx.send(incoming);
        };
        match action {
            Action::LoadQueue => send(backend.load_queue().await.unwrap_or_else(|e| failed(Failure::Queue, e))),
            Action::Open(key) => {
                if let Some(cached) = backend.open_cached(key) {
                    send(cached);
                }
                send(backend.fetch_review(key).await.unwrap_or_else(|e| failed(Failure::Open, e)));
            }
            Action::RefreshMr(key) => send(backend.fetch_review(key).await.unwrap_or_else(|e| failed(Failure::Poll, e))),
            Action::RefreshDiscussions(key) => send(backend.fetch_discussions(key).await.unwrap_or_else(|e| failed(Failure::Poll, e))),
            Action::SaveState { key, fold, viewed } => {
                if let Err(e) = backend.save_state(key, fold, viewed) {
                    send(failed(Failure::Local, e));
                }
            }
            Action::OpenUrl(url) => {
                send(open_url(&url).map(|()| Incoming::Done("opened in the browser".into())).unwrap_or_else(|e| failed(Failure::Local, e)))
            }
            Action::Yank(url) => send(copy(&url).map(|()| Incoming::Done("copied".into())).unwrap_or_else(|e| failed(Failure::Local, e))),
        }
    });
}

fn failed(what: Failure, err: anyhow::Error) -> Incoming {
    Incoming::Failed { what, message: err.to_string() }
}

impl Backend {
    async fn load_queue(&self) -> Result<Incoming> {
        let queue = self.gitlab.queue().await?;
        let _ = self.cache.write_entry(&keys::queue(), &queue);
        let sections = queue.sections(&self.watch_labels);
        let opened = self.opened_at(&sections);
        Ok(Incoming::Queue { sections, opened })
    }

    fn opened_at(&self, sections: &crate::api::Sections) -> HashMap<MrKey, DateTime<Utc>> {
        [&sections.to_review, &sections.mine, &sections.watching, &sections.done]
            .into_iter()
            .flatten()
            .filter_map(|mr| {
                let key = (mr.project_id, mr.iid);
                let state: MrState = self.cache.read(&keys::state(key.0, key.1))?;
                Some((key, state.opened_at?))
            })
            .collect()
    }

    fn state(&self, key: MrKey) -> MrState {
        self.cache.read(&keys::state(key.0, key.1)).unwrap_or_default()
    }

    fn open_cached(&self, key: MrKey) -> Option<Incoming> {
        let mr: Entry<crate::api::Mr> = self.cache.read_entry(&keys::mr(key.0, key.1))?;
        let diffs: Vec<crate::api::DiffFile> = self.cache.read(&keys::diffs(key.0, key.1, &mr.value.diff_refs.head_sha))?;
        let discussions: Vec<crate::api::Discussion> = self.cache.read(&keys::discussions(key.0, key.1)).unwrap_or_default();
        let age = mr.age(Utc::now());
        let review = self.build(key, mr.value, diffs, discussions);
        Some(Incoming::Review { key, review: Box::new(review), cached: Some(age) })
    }

    async fn fetch_review(&self, key: MrKey) -> Result<Incoming> {
        let (project_id, iid) = key;
        let (mr, diffs, discussions) = tokio::try_join!(
            self.gitlab.mr(project_id, iid),
            self.gitlab.diffs(project_id, iid),
            self.gitlab.discussions(project_id, iid)
        )?;
        let _ = self.cache.write_entry(&keys::mr(project_id, iid), &mr);
        let _ = self.cache.write(&keys::diffs(project_id, iid, &mr.diff_refs.head_sha), &diffs);
        let _ = self.cache.write(&keys::discussions(project_id, iid), &discussions);
        let state = MrState { opened_at: Some(Utc::now()), ..self.state(key) };
        let _ = self.cache.write(&keys::state(project_id, iid), &state);
        let review = self.build(key, mr, diffs, discussions);
        Ok(Incoming::Review { key, review: Box::new(review), cached: None })
    }

    async fn fetch_discussions(&self, key: MrKey) -> Result<Incoming> {
        let discussions = self.gitlab.discussions(key.0, key.1).await?;
        let _ = self.cache.write(&keys::discussions(key.0, key.1), &discussions);
        Ok(Incoming::Discussions { key, discussions })
    }

    fn build(&self, key: MrKey, mr: crate::api::Mr, diffs: Vec<crate::api::DiffFile>, discussions: Vec<crate::api::Discussion>) -> Review {
        let state = self.state(key);
        let review = Review::new(mr, diffs, discussions, &self.fold_globs);
        let fold = merged_fold(review.fold.clone(), state.fold);
        review.with_fold(fold).with_viewed(state.viewed)
    }

    fn save_state(&self, key: MrKey, fold: FoldState, viewed: BTreeSet<String>) -> Result<()> {
        let state = MrState { fold, viewed, ..self.state(key) };
        self.cache.write(&keys::state(key.0, key.1), &state)
    }
}

/// What the reader folded wins over the defaults, file by file.
fn merged_fold(initial: FoldState, saved: FoldState) -> FoldState {
    let mut files = initial.files;
    files.extend(saved.files);
    let mut hunks = initial.hunks;
    hunks.extend(saved.hunks);
    FoldState { files, hunks }
}

fn open_url(url: &str) -> Result<()> {
    let status = std::process::Command::new("open").arg(url).status().context("running open")?;
    anyhow::ensure!(status.success(), "open failed");
    Ok(())
}

fn copy(text: &str) -> Result<()> {
    use std::io::Write;
    let mut child = std::process::Command::new("pbcopy").stdin(std::process::Stdio::piped()).spawn().context("running pbcopy")?;
    child.stdin.take().context("pbcopy stdin")?.write_all(text.as_bytes())?;
    anyhow::ensure!(child.wait()?.success(), "pbcopy failed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::fold::Fold;

    #[test]
    fn saved_folds_win_over_the_defaults() {
        let mut initial = FoldState::default();
        initial.files.insert("Cargo.lock".into(), Fold::Closed);
        let mut saved = FoldState::default();
        saved.files.insert("src/a.rs".into(), Fold::Closed);
        let merged = merged_fold(initial, saved);
        assert!(!merged.file_is_open("Cargo.lock"));
        assert!(!merged.file_is_open("src/a.rs"));
    }

    #[test]
    fn state_round_trips_and_tolerates_an_empty_file() {
        let state: MrState = serde_json::from_str("{}").unwrap();
        assert_eq!(state, MrState::default());
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::in_dir(dir.path());
        let backend = Backend { gitlab: test_client(), cache, fold_globs: vec![], watch_labels: vec![] };
        backend.save_state((7, 42), FoldState::default(), BTreeSet::from(["a.rs".to_owned()])).unwrap();
        assert_eq!(backend.state((7, 42)).viewed, BTreeSet::from(["a.rs".to_owned()]));
    }

    fn test_client() -> Client {
        Client::new(&crate::auth::Credentials { host: "gitlab.com".into(), token: "glpat-xxxx".into() }).unwrap()
    }
}
