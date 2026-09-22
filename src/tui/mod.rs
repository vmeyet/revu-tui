pub mod app;
pub mod compose;
pub mod diff_view;
pub mod field;
pub mod publish_view;
pub mod theme;
pub mod thread_view;
pub mod ui;

use crate::api::{Client, DraftNote, NewDraft};
use crate::cache::{Cache, Entry, keys};
use crate::ctx::Ctx;
use crate::diff::fold::FoldState;
use crate::review::{Draft, Review};
use anyhow::{Context as _, Result};
use app::{Action, App, Failure, Incoming, Input, MrKey, Settings};
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
    let backend = Backend {
        gitlab: ctx.gitlab.clone(),
        cache: ctx.cache.clone(),
        fold_globs: ctx.config.review.fold.clone(),
        watch_labels: ctx.config.queue.watch_labels.clone(),
    };
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
            Some(incoming) = rx.recv() => { app.apply(incoming); app.take_actions() }
            _ = ticks.tick() => {
                app.now = Instant::now();
                app.today = Utc::now();
                app.tick()
            }
        };
        for action in actions {
            match action {
                Action::Compose { input, draft } => {
                    for follow_up in compose_inline(terminal, app, input, &draft) {
                        spawn(follow_up, backend, tx.clone());
                    }
                }
                other => spawn(other, backend, tx.clone()),
            }
        }
    }
    Ok(())
}

/// The editor owns the terminal for a while; the answer goes through `apply` like any other.
fn compose_inline(terminal: &mut ratatui::DefaultTerminal, app: &mut App, input: Input, draft: &str) -> Vec<Action> {
    ratatui::restore();
    let edited = compose::edit(draft);
    *terminal = ratatui::init();
    let _ = terminal.clear();
    match edited {
        Ok(text) => app.apply(Incoming::Composed { input, text }),
        Err(e) => app.apply(failed(Failure::Local, e)),
    }
    app.take_actions()
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
            Action::SaveDraft { key, index, draft } => {
                send(backend.save_draft(key, index, &draft).await.unwrap_or_else(|e| failed(Failure::Draft { index }, e)))
            }
            Action::UpdateDraft { key, id, draft } => {
                if let Err(e) = backend.gitlab.update_draft(key.0, key.1, id, &new_draft(&draft)).await {
                    send(failed(Failure::Local, e));
                }
            }
            Action::DeleteDraft { key, id } => {
                if let Err(e) = backend.gitlab.delete_draft(key.0, key.1, id).await {
                    send(failed(Failure::Local, e));
                }
            }
            Action::Publish { key, approve, count } => {
                send(backend.publish(key, approve, count).await.unwrap_or_else(|e| failed(Failure::Publish, e)))
            }
            Action::Resolve { key, thread, resolved } => send(
                backend
                    .gitlab
                    .resolve(key.0, key.1, &thread, resolved)
                    .await
                    .map(|_| Incoming::Resolved { key, thread: thread.clone(), resolved })
                    .unwrap_or_else(|e| failed(Failure::Resolve { thread, resolved }, e)),
            ),
            Action::Approve { key, approve } => {
                let outcome =
                    if approve { backend.gitlab.approve(key.0, key.1).await } else { backend.gitlab.unapprove(key.0, key.1).await };
                send(outcome.map(|()| Incoming::Approved { key, approve }).unwrap_or_else(|e| failed(Failure::Approve, e)));
            }
            Action::Compose { .. } => unreachable!("the loop runs the editor itself"),
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
        let drafts: Vec<DraftNote> = self.cache.read(&keys::drafts(key.0, key.1)).unwrap_or_default();
        let age = mr.age(Utc::now());
        let review = self.build(key, mr.value, diffs, discussions, &drafts);
        Some(Incoming::Review { key, review: Box::new(review), cached: Some(age) })
    }

    async fn fetch_review(&self, key: MrKey) -> Result<Incoming> {
        let (project_id, iid) = key;
        let (mr, diffs, discussions, drafts) = tokio::try_join!(
            self.gitlab.mr(project_id, iid),
            self.gitlab.diffs(project_id, iid),
            self.gitlab.discussions(project_id, iid),
            self.gitlab.draft_notes(project_id, iid)
        )?;
        let _ = self.cache.write_entry(&keys::mr(project_id, iid), &mr);
        let _ = self.cache.write(&keys::diffs(project_id, iid, &mr.diff_refs.head_sha), &diffs);
        let _ = self.cache.write(&keys::discussions(project_id, iid), &discussions);
        let _ = self.cache.write(&keys::drafts(project_id, iid), &drafts);
        let state = MrState { opened_at: Some(Utc::now()), ..self.state(key) };
        let _ = self.cache.write(&keys::state(project_id, iid), &state);
        let review = self.build(key, mr, diffs, discussions, &drafts);
        Ok(Incoming::Review { key, review: Box::new(review), cached: None })
    }

    /// Posts the draft unless GitLab already lists it: a retry after a lost answer never doubles a note.
    async fn save_draft(&self, key: MrKey, index: usize, draft: &Draft) -> Result<Incoming> {
        let (project_id, iid) = key;
        let held = self.gitlab.draft_notes(project_id, iid).await?;
        let id = match held.iter().find(|note| draft.same_as(&to_draft(note))) {
            Some(note) => note.id,
            None => self.gitlab.create_draft(project_id, iid, &new_draft(draft)).await?.id,
        };
        Ok(Incoming::DraftSaved { key, index, id })
    }

    async fn publish(&self, key: MrKey, approve: bool, count: usize) -> Result<Incoming> {
        self.gitlab.publish_drafts(key.0, key.1).await?;
        let _ = self.cache.write(&keys::drafts(key.0, key.1), &Vec::<DraftNote>::new());
        if approve {
            self.gitlab.approve(key.0, key.1).await?;
        }
        Ok(Incoming::Published { key, approved: approve, count })
    }

    async fn fetch_discussions(&self, key: MrKey) -> Result<Incoming> {
        let discussions = self.gitlab.discussions(key.0, key.1).await?;
        let _ = self.cache.write(&keys::discussions(key.0, key.1), &discussions);
        Ok(Incoming::Discussions { key, discussions })
    }

    fn build(
        &self,
        key: MrKey,
        mr: crate::api::Mr,
        diffs: Vec<crate::api::DiffFile>,
        discussions: Vec<crate::api::Discussion>,
        drafts: &[DraftNote],
    ) -> Review {
        let state = self.state(key);
        let review = Review::new(mr, diffs, discussions, &self.fold_globs);
        let fold = merged_fold(review.fold.clone(), state.fold);
        review.with_fold(fold).with_viewed(state.viewed).with_drafts(drafts.iter().map(to_draft).collect())
    }

    fn save_state(&self, key: MrKey, fold: FoldState, viewed: BTreeSet<String>) -> Result<()> {
        let state = MrState { fold, viewed, ..self.state(key) };
        self.cache.write(&keys::state(key.0, key.1), &state)
    }
}

fn to_draft(note: &DraftNote) -> Draft {
    Draft::from_note(note.id, note.note.clone(), note.position.as_ref(), note.discussion_id.clone(), note.resolve_discussion)
}

fn new_draft(draft: &Draft) -> NewDraft {
    NewDraft {
        note: draft.body.clone(),
        position: draft.position.clone(),
        in_reply_to_discussion_id: draft.reply_to.clone(),
        resolve_discussion: draft.resolve,
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

    fn held_note(id: u64, note: &str, new_line: u32) -> serde_json::Value {
        serde_json::json!({
            "id": id, "author_id": 1, "merge_request_id": 42, "note": note,
            "position": {"base_sha": "a", "head_sha": "b", "start_sha": "a", "position_type": "text",
                         "old_path": "src/a.rs", "new_path": "src/a.rs", "old_line": null, "new_line": new_line}
        })
    }

    async fn backend_on(server: &wiremock::MockServer) -> Backend {
        let creds = crate::auth::Credentials { host: "gitlab.com".into(), token: "glpat-xxxx".into() };
        let gitlab = Client::with_base(&creds, &format!("{}/api/v4/", server.uri())).unwrap();
        let dir = tempfile::tempdir().unwrap();
        Backend { gitlab, cache: Cache::in_dir(dir.path()), fold_globs: vec![], watch_labels: vec![] }
    }

    fn draft_at(new_line: u32, body: &str) -> Draft {
        let refs = crate::api::DiffRefs { base_sha: "a".into(), head_sha: "b".into(), start_sha: "a".into() };
        Draft::on(crate::api::Position::line(&refs, "src/a.rs", "src/a.rs", None, Some(new_line)), body)
    }

    #[tokio::test]
    async fn a_draft_gitlab_already_holds_is_not_posted_twice() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/7/merge_requests/42/draft_notes"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([held_note(5, "nit", 12)])))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v4/projects/7/merge_requests/42/draft_notes"))
            .respond_with(ResponseTemplate::new(201).set_body_json(held_note(6, "other", 13)))
            .expect(1)
            .mount(&server)
            .await;
        let backend = backend_on(&server).await;
        let same = backend.save_draft((7, 42), 0, &draft_at(12, "nit")).await.unwrap();
        assert_eq!(same, Incoming::DraftSaved { key: (7, 42), index: 0, id: 5 });
        let fresh = backend.save_draft((7, 42), 1, &draft_at(13, "other")).await.unwrap();
        assert_eq!(fresh, Incoming::DraftSaved { key: (7, 42), index: 1, id: 6 });
    }
}
