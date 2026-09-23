//! The review TUI: an event loop over a pure `App`, with the network at its edge.
mod app;
mod brief_view;
mod complete;
mod compose;
mod diff_view;
mod field;
mod ground;
mod jump;
mod palette;
mod publish_view;
mod theme;
mod thread_view;
mod tree_view;
mod ui;

use crate::cache::{Cache, Entry, keys};
use crate::ctx::Ctx;
use crate::diff::fold::FoldState;
use crate::diff::words::InlineRule;
use crate::forge::{DiffFile, Discussion, Draft as HeldDraft, Forge, Mr, MrKey, Queue, Sections};
use crate::review::{Draft, Review};
use anyhow::{Context as _, Result};
use app::{Action, App, Failure, Incoming, Input, Settings};
use chrono::{DateTime, Utc};
use crossterm::event::{Event, EventStream, KeyEventKind};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

const TICK: Duration = Duration::from_millis(100);

/// What survives between two openings of one MR, in the cache.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct MrState {
    #[serde(default)]
    fold: FoldState,
    /// Viewed files by path, with the fingerprint of the change seen.
    #[serde(default)]
    viewed_files: BTreeMap<String, String>,
    #[serde(default)]
    opened_at: Option<DateTime<Utc>>,
    #[serde(default)]
    split: bool,
}

#[derive(Clone)]
struct Backend {
    forge: Forge,
    cache: Cache,
    fold_globs: Vec<String>,
    watch_labels: Vec<String>,
    inline: InlineRule,
}

/// Runs the review TUI until the user quits, restoring the terminal on the way out.
pub async fn run(ctx: Ctx) -> Result<()> {
    let theme = match ctx.config.tui.theme.as_deref() {
        Some(name) => theme::Theme::named(name)
            .ok_or_else(|| anyhow::anyhow!("config `tui.theme = \"{name}\"` is not a theme (try {})", theme::Theme::NAMES.join(", ")))?,
        None => theme::Theme::default(),
    };
    let ground = ground::ask();
    let theme = ground.map_or(theme, |ground| theme.with_ground(ground));
    let backend = Backend {
        forge: ctx.forge.clone(),
        cache: ctx.cache.clone(),
        fold_globs: ctx.config.review.fold.clone(),
        watch_labels: ctx.config.queue.watch_labels.clone(),
        inline: ctx.config.review.inline(),
    };
    let settings = Settings {
        theme,
        host: ctx.forge.host().to_owned(),
        kind: ctx.forge.kind(),
        me: ctx.config.username_for(&ctx.credentials.host).unwrap_or_default(),
        project: ctx.project.clone(),
        ground,
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
        let frame = terminal.draw(|f| ui::draw(f, app))?;
        let links = hyperlinks(frame.buffer, &app.links);
        print_links(&links);
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
        Err(e) => app.apply(failed(Failure::Local, &e)),
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
            Action::LoadQueue { scope, from_cache } => {
                if let Some(cached) = backend.cached_queue(scope.clone()).filter(|_| from_cache) {
                    send(cached);
                }
                send(backend.load_queue(scope).await.unwrap_or_else(|e| failed(Failure::Queue, &e)));
            }
            Action::Open(key) => {
                if let Some(cached) = backend.open_cached(&key) {
                    send(cached);
                }
                send(backend.fetch_review(key).await.unwrap_or_else(|e| failed(Failure::Open, &e)));
            }
            Action::RefreshMr(key) => send(backend.fetch_review(key).await.unwrap_or_else(|e| failed(Failure::Poll, &e))),
            Action::RefreshDiscussions(key) => send(backend.fetch_discussions(key).await.unwrap_or_else(|e| failed(Failure::Poll, &e))),
            Action::SaveState { key, fold, viewed, split } => {
                if let Err(e) = backend.save_state(&key, fold, viewed, split) {
                    send(failed(Failure::Local, &e));
                }
            }
            Action::OpenUrl(url) => {
                send(open_url(&url).map_or_else(|e| failed(Failure::Local, &e), |()| Incoming::Done("opened in the browser".into())));
            }
            Action::SaveTheme(name) => {
                if let Err(e) = save_theme(&name) {
                    send(failed(Failure::Local, &e));
                }
            }
            Action::Yank(url) => send(copy(&url).map_or_else(|e| failed(Failure::Local, &e), |()| Incoming::Done("copied".into()))),
            Action::SaveDraft { key, index, draft } => {
                send(backend.save_draft(key, index, &draft).await.unwrap_or_else(|e| failed(Failure::Draft { index }, &e)));
            }
            Action::UpdateDraft { key, id, draft } => {
                if let Err(e) = backend.forge.update_draft(&key, id, &draft.payload()).await {
                    send(failed(Failure::Local, &e));
                }
            }
            Action::DeleteDraft { key, id } => {
                if let Err(e) = backend.forge.delete_draft(&key, id).await {
                    send(failed(Failure::Local, &e));
                }
            }
            Action::Publish { key, approve, count } => {
                send(backend.publish(key, approve, count).await.unwrap_or_else(|e| failed(Failure::Publish, &e)));
            }
            Action::Resolve { key, thread, resolved } => send(backend.forge.resolve(&key, &thread, resolved).await.map_or_else(
                |e| failed(Failure::Resolve { thread: thread.clone(), resolved }, &e),
                |()| Incoming::Resolved { key, thread: thread.clone(), resolved },
            )),
            Action::Approve { key, approve } => {
                let outcome = backend.forge.approve(&key, approve).await;
                send(outcome.map_or_else(|e| failed(Failure::Approve, &e), |()| Incoming::Approved { key, approve }));
            }
            Action::Compose { .. } => unreachable!("the loop runs the editor itself"),
        }
    });
}

/// A link as the terminal will print it: only where the drawn cells still spell its text,
/// in the colours they were drawn with.
struct Hyperlink {
    link: ui::Link,
    fg: ratatui::style::Color,
    bg: ratatui::style::Color,
}

fn hyperlinks(buffer: &ratatui::buffer::Buffer, links: &[ui::Link]) -> Vec<Hyperlink> {
    links
        .iter()
        .filter(|link| {
            link.text
                .chars()
                .enumerate()
                .all(|(i, c)| buffer.cell((link.x + i as u16, link.y)).is_some_and(|cell| cell.symbol().chars().eq(std::iter::once(c))))
        })
        .filter_map(|link| {
            let cell = buffer.cell((link.x, link.y))?;
            Some(Hyperlink { link: link.clone(), fg: cell.fg, bg: cell.bg })
        })
        .collect()
}

/// OSC 8 over the text ratatui drew; terminals without it show the same text, unchanged.
fn print_links(links: &[Hyperlink]) {
    use ratatui::backend::IntoCrossterm;
    use ratatui::crossterm::{cursor, queue, style};
    use std::io::Write;
    let mut out = std::io::stdout();
    for Hyperlink { link, fg, bg } in links {
        let _ = queue!(
            out,
            cursor::SavePosition,
            cursor::MoveTo(link.x, link.y),
            style::SetForegroundColor(fg.into_crossterm()),
            style::SetBackgroundColor(bg.into_crossterm()),
            style::Print(format!("\x1b]8;;{}\x1b\\{}\x1b]8;;\x1b\\", link.url, link.text)),
            style::ResetColor,
            cursor::RestorePosition,
        );
    }
    let _ = out.flush();
}

/// `:set theme=`: the one setting the TUI writes, read back on the next start.
fn save_theme(name: &str) -> Result<()> {
    let mut config = crate::config::Config::load()?;
    config.tui.theme = Some(name.to_owned());
    config.save()
}

fn failed(what: Failure, err: &anyhow::Error) -> Incoming {
    Incoming::Failed { what, message: err.to_string() }
}

impl Backend {
    async fn load_queue(&self, scope: Option<String>) -> Result<Incoming> {
        let queue = self.forge.queue(scope.as_deref()).await?;
        let _ = self.cache.write_entry(&keys::queue(scope.as_deref()), &queue);
        Ok(self.queue_answer(scope, &queue, false))
    }

    fn cached_queue(&self, scope: Option<String>) -> Option<Incoming> {
        let queue: Queue = self.cache.read_entry(&keys::queue(scope.as_deref()))?.value;
        Some(self.queue_answer(scope, &queue, true))
    }

    fn queue_answer(&self, scope: Option<String>, queue: &Queue, cached: bool) -> Incoming {
        let sections = queue.sections(&self.watch_labels);
        let opened = self.opened_at(&sections);
        Incoming::Queue { scope, me: queue.me.clone(), sections, opened, cached }
    }

    fn opened_at(&self, sections: &Sections) -> HashMap<MrKey, DateTime<Utc>> {
        [&sections.to_review, &sections.mine, &sections.watching, &sections.open, &sections.done]
            .into_iter()
            .flatten()
            .filter_map(|mr| {
                let key = mr.key();
                let state: MrState = self.cache.read(&keys::state(&key))?;
                Some((key, state.opened_at?))
            })
            .collect()
    }

    fn state(&self, key: &MrKey) -> MrState {
        self.cache.read(&keys::state(key)).unwrap_or_default()
    }

    fn open_cached(&self, key: &MrKey) -> Option<Incoming> {
        let mr: Entry<Mr> = self.cache.read_entry(&keys::mr(key))?;
        let diffs: Vec<DiffFile> = self.cache.read(&keys::diffs(key, &mr.value.refs.head))?;
        let discussions: Vec<Discussion> = self.cache.read(&keys::discussions(key)).unwrap_or_default();
        let drafts: Vec<HeldDraft> = self.cache.read(&keys::drafts(key)).unwrap_or_default();
        let age = mr.age(Utc::now());
        let review = self.build(key, mr.value, &diffs, discussions, &drafts);
        Some(Incoming::Review { key: key.clone(), review: Box::new(review), cached: Some(age) })
    }

    async fn fetch_review(&self, key: MrKey) -> Result<Incoming> {
        let forge = &self.forge;
        let (mr, diffs, discussions, drafts) =
            tokio::try_join!(forge.mr(&key), forge.diffs(&key), forge.discussions(&key), forge.drafts(&key))?;
        let _ = self.cache.write_entry(&keys::mr(&key), &mr);
        let _ = self.cache.write(&keys::diffs(&key, &mr.refs.head), &diffs);
        let _ = self.cache.write(&keys::discussions(&key), &discussions);
        let _ = self.cache.write(&keys::drafts(&key), &drafts);
        let state = MrState { opened_at: Some(Utc::now()), ..self.state(&key) };
        let _ = self.cache.write(&keys::state(&key), &state);
        let review = self.build(&key, mr, &diffs, discussions, &drafts);
        Ok(Incoming::Review { key, review: Box::new(review), cached: None })
    }

    /// Posts the draft unless the forge already lists it: a retry after a lost answer never doubles a note.
    async fn save_draft(&self, key: MrKey, index: usize, draft: &Draft) -> Result<Incoming> {
        let held = self.forge.drafts(&key).await?;
        let id = match held.iter().find(|note| draft.same_as(&Draft::held(note))) {
            Some(note) => note.id,
            None => self.forge.create_draft(&key, &draft.payload()).await?.id,
        };
        Ok(Incoming::DraftSaved { key, index, id })
    }

    async fn publish(&self, key: MrKey, approve: bool, count: usize) -> Result<Incoming> {
        self.forge.publish(&key, approve).await?;
        let _ = self.cache.write(&keys::drafts(&key), &Vec::<HeldDraft>::new());
        Ok(Incoming::Published { key, approved: approve, count })
    }

    async fn fetch_discussions(&self, key: MrKey) -> Result<Incoming> {
        let discussions = self.forge.discussions(&key).await?;
        let _ = self.cache.write(&keys::discussions(&key), &discussions);
        Ok(Incoming::Discussions { key, discussions })
    }

    fn build(&self, key: &MrKey, mr: Mr, diffs: &[DiffFile], discussions: Vec<Discussion>, drafts: &[HeldDraft]) -> Review {
        let state = self.state(key);
        let review = Review::new(mr, diffs, discussions, &self.fold_globs);
        let fold = merged_fold(review.fold.clone(), state.fold);
        review
            .with_fold(fold)
            .with_viewed(review.still_viewed(&state.viewed_files))
            .with_inline(self.inline)
            .with_split(state.split)
            .with_drafts(drafts.iter().map(Draft::held).collect())
    }

    fn save_state(&self, key: &MrKey, fold: FoldState, viewed_files: BTreeMap<String, String>, split: bool) -> Result<()> {
        let state = MrState { fold, viewed_files, split, ..self.state(key) };
        self.cache.write(&keys::state(key), &state)
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
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::forge::gitlab::Client;
    use crate::forge::gitlab::fixture::key;
    use crate::forge::{LineRef, Position, Refs};

    #[test]
    fn a_link_prints_only_where_its_text_is_still_on_screen() {
        let mut buffer = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 10, 2));
        buffer.set_string(1, 0, "!42 x", ratatui::style::Style::default());
        buffer.set_string(1, 1, "!4…", ratatui::style::Style::default());
        let link = |y| ui::Link { x: 1, y, text: "!42".into(), url: "https://gitlab.com/acme/widgets/-/merge_requests/42".into() };
        let printed = hyperlinks(&buffer, &[link(0), link(1)]);
        assert_eq!(printed.len(), 1, "the clipped `!4…` stays plain text");
        assert_eq!(printed[0].link.y, 0);
    }
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
        let backend = Backend { forge: test_forge(), cache, fold_globs: vec![], watch_labels: vec![], inline: InlineRule::default() };
        backend.save_state(&key(), FoldState::default(), BTreeMap::from([("a.rs".to_owned(), "f1".to_owned())]), true).unwrap();
        let state = backend.state(&key());
        assert_eq!((state.viewed_files, state.split), (BTreeMap::from([("a.rs".to_owned(), "f1".to_owned())]), true), "the split choice is remembered per MR");
    }

    fn test_forge() -> Forge {
        Forge::GitLab(Client::new(&crate::auth::Credentials { host: "gitlab.com".into(), token: "glpat-xxxx".into() }).unwrap())
    }

    fn held_note(id: u64, note: &str, new_line: u32) -> serde_json::Value {
        serde_json::json!({
            "id": id, "author_id": 1, "merge_request_id": 42, "note": note,
            "position": {"base_sha": "a", "head_sha": "b", "start_sha": "a", "position_type": "text",
                         "old_path": "src/a.rs", "new_path": "src/a.rs", "old_line": null, "new_line": new_line}
        })
    }

    fn backend_on(server: &wiremock::MockServer) -> Backend {
        let creds = crate::auth::Credentials { host: "gitlab.com".into(), token: "glpat-xxxx".into() };
        let forge = Forge::GitLab(Client::with_base(&creds, &format!("{}/api/v4/", server.uri())).unwrap());
        let dir = tempfile::tempdir().unwrap();
        Backend { forge, cache: Cache::in_dir(dir.path()), fold_globs: vec![], watch_labels: vec![], inline: InlineRule::default() }
    }

    fn draft_at(new_line: u32, body: &str) -> Draft {
        let refs = Refs { base: "a".into(), start: "a".into(), head: "b".into() };
        let line = LineRef { old: None, new: Some(new_line) };
        Draft::on(Position { refs, old_path: "src/a.rs".into(), new_path: "src/a.rs".into(), line, start: None }, body)
    }

    #[tokio::test]
    async fn a_draft_the_forge_already_holds_is_not_posted_twice() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/acme%2Fwidgets/merge_requests/42/draft_notes"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([held_note(5, "nit", 12)])))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v4/projects/acme%2Fwidgets/merge_requests/42/draft_notes"))
            .respond_with(ResponseTemplate::new(201).set_body_json(held_note(6, "other", 13)))
            .expect(1)
            .mount(&server)
            .await;
        let backend = backend_on(&server);
        let same = backend.save_draft(key(), 0, &draft_at(12, "nit")).await.unwrap();
        assert_eq!(same, Incoming::DraftSaved { key: key(), index: 0, id: 5 });
        let fresh = backend.save_draft(key(), 1, &draft_at(13, "other")).await.unwrap();
        assert_eq!(fresh, Incoming::DraftSaved { key: key(), index: 1, id: 6 });
    }
}
