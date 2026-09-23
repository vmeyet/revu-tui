//! The review TUI: an event loop over a pure `App`, with the network at its edge.
mod answer_view;
mod app;
mod brief_view;
mod complete;
mod compose;
mod diff_view;
mod field;
mod ground;
mod jump;
mod palette;
mod pipeline_view;
mod publish_view;
mod theme;
mod thread_view;
mod tree_view;
mod ui;

use crate::ai;
use crate::ai::anthropic::{self, Ask, Claude, Outcome, Stop, Usage};
use crate::ai::triage::{self, Verdict};
use crate::ai::typesafe::{TypeSafe, Unavailable};
use crate::cache::{Cache, Entry, keys};
use crate::ctx::Ctx;
use crate::diff::fold::FoldState;
use crate::diff::words::InlineRule;
use crate::forge::{DiffFile, Discussion, Draft as HeldDraft, Forge, Mr, MrKey, Queue, Sections};
use crate::review::{Draft, Review};
use anyhow::{Context as _, Result};
use app::{Action, App, Failure, Incoming, Input, Part, Settings};
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
    /// The other hosts I am logged in to: every project's queue asks them too.
    others: Vec<crate::ctx::Home>,
    fold_globs: Vec<String>,
    watch_labels: Vec<String>,
    inline: InlineRule,
    open: crate::config::Open,
    /// The checkout `revu` runs in, when its origin is on this forge: `v` opens its real files.
    checkout: Option<crate::open::Checkout>,
    /// Jev, when `[ai.typesafe]` is on and a key was found.
    jev: Option<TypeSafe>,
    /// Claude, when `[ai.anthropic]` is on and a key was found.
    claude: Option<Claude>,
}

/// An answer kept in the cache: asking the same thing about the same diff paints it at once.
#[derive(Serialize, Deserialize)]
struct SavedAnswer {
    text: String,
    model: String,
    usage: Usage,
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
    let others = ctx.others();
    let hosts = ctx.hosts(&others);
    let backend = Backend {
        forge: ctx.forge.clone(),
        cache: ctx.cache.clone(),
        others,
        fold_globs: ctx.config.review.fold.clone(),
        watch_labels: ctx.config.queue.watch_labels.clone(),
        inline: ctx.config.review.inline(),
        open: ctx.config.open.clone(),
        checkout: std::env::current_dir().ok().and_then(|dir| crate::open::Checkout::find(&dir, ctx.forge.host())),
        jev: jev(&ctx.config.ai),
        claude: claude(&ctx.config.ai),
    };
    let settings = Settings {
        theme,
        host: ctx.forge.host().to_owned(),
        me: ctx.config.username_for(&ctx.credentials.host).unwrap_or_default(),
        project: ctx.project.clone(),
        ground,
        triage: backend.jev.is_some(),
        ask: backend.claude.as_ref().map(|_| ctx.config.ai.anthropic.model().to_owned()),
        notify: ctx.config.notify.enabled,
        hosts,
    };
    let mut app = App::new(settings);
    let mut terminal = ratatui::init();
    let outcome = event_loop(&mut terminal, &mut app, &backend).await;
    ratatui::restore();
    outcome
}

async fn event_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut App, backend: &Backend) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Incoming>();
    let mut keys = Keys::new();
    let mut ticks = tokio::time::interval(TICK);
    for action in app.start() {
        spawn(action, backend, tx.clone());
    }
    while !app.should_quit {
        let frame = terminal.draw(|f| ui::draw(f, app))?;
        let links = hyperlinks(frame.buffer, &app.links);
        print_links(&links);
        let actions = tokio::select! {
            Some(event) = keys.next() => match event? {
                Event::Key(key) if key.kind != KeyEventKind::Release => app.handle_key(key),
                _ => vec![],
            },
            Some(incoming) = rx.recv() => { app.apply(incoming); app.take_actions() }
            _ = ticks.tick() => {
                wake(app);
                app.rate = backend.forge.rate();
                app.tick()
            }
        };
        for action in actions {
            match action {
                Action::Compose { input, draft } => {
                    for follow_up in compose_inline(terminal, &mut keys, app, input, &draft) {
                        spawn(follow_up, backend, tx.clone());
                    }
                }
                other => spawn(other, backend, tx.clone()),
            }
        }
        if let Some(view) = app.take_view() {
            view_inline(terminal, &mut keys, app, view);
        }
    }
    Ok(())
}

/// The reader's program owns the terminal until it exits; the app, untouched, draws again after.
fn view_inline(terminal: &mut ratatui::DefaultTerminal, keys: &mut Keys, app: &mut App, view: crate::open::View) {
    keys.pause();
    ratatui::restore();
    let outcome = crate::open::run(&view.argv);
    *terminal = ratatui::init();
    keys.resume();
    let _ = terminal.clear();
    wake(app);
    app.apply(Incoming::Viewed { view, outcome });
}

/// The clock stood still while a program had the terminal: what comes next is timed from now.
fn wake(app: &mut App) {
    app.now = Instant::now();
    app.today = Utc::now();
}

/// The editor owns the terminal for a while; the answer goes through `apply` like any other.
fn compose_inline(terminal: &mut ratatui::DefaultTerminal, keys: &mut Keys, app: &mut App, input: Input, draft: &str) -> Vec<Action> {
    keys.pause();
    ratatui::restore();
    let edited = compose::edit(draft);
    *terminal = ratatui::init();
    keys.resume();
    let _ = terminal.clear();
    wake(app);
    match edited {
        Ok(text) => app.apply(Incoming::Composed { input, text }),
        Err(e) => app.apply(failed(Failure::Local, &e)),
    }
    app.take_actions()
}

/// The terminal's keys. The event stream keeps a thread reading the terminal, which would steal
/// the keys typed into a program that takes it over: `pause` drops the stream, which stops that
/// thread, and `resume` starts a new one once the program is gone. A new stream cannot be made
/// before the old one is dropped: both need the same reader lock.
struct Keys(Option<EventStream>);

impl Keys {
    fn new() -> Self {
        Self(Some(EventStream::new()))
    }

    async fn next(&mut self) -> Option<std::io::Result<Event>> {
        match &mut self.0 {
            Some(stream) => stream.next().await,
            None => None,
        }
    }

    fn pause(&mut self) {
        self.0 = None;
    }

    fn resume(&mut self) {
        self.0 = Some(EventStream::new());
    }
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
                let wanted = scope.clone();
                if let Some(cached) = backend.off(move |b| b.cached_queue(wanted)).await.ok().flatten().filter(|_| from_cache) {
                    send(cached);
                }
                send(backend.load_queue(scope).await.unwrap_or_else(|e| failed(Failure::Queue, &e)));
            }
            Action::Open(key) => {
                let wanted = key.clone();
                if let Some(cached) = backend.off(move |b| b.open_cached(&wanted)).await.ok().flatten() {
                    send(cached);
                }
                send(backend.fetch_review(key).await.unwrap_or_else(|e| failed(Failure::Open, &e)));
            }
            Action::RefreshMr(key) => send(backend.fetch_review(key).await.unwrap_or_else(|e| failed(Failure::Poll, &e))),
            Action::RefreshDiscussions(key) => send(backend.fetch_discussions(key).await.unwrap_or_else(|e| failed(Failure::Poll, &e))),
            Action::SaveState { key, fold, viewed, split } => {
                if let Err(e) = backend.off(move |b| b.save_state(&key, fold, viewed, split)).await.and_then(|saved| saved) {
                    send(failed(Failure::Local, &e));
                }
            }
            Action::Notify { title, body } => {
                if let Err(e) = notify(&title, &body).await {
                    send(failed(Failure::Local, &e));
                }
            }
            Action::OpenUrl(url) => {
                send(open_url(&url).await.map_or_else(|e| failed(Failure::Local, &e), |()| Incoming::Done("opened in the browser".into())));
            }
            Action::SaveTheme(name) => {
                if let Err(e) = blocking(move || save_theme(&name)).await {
                    send(failed(Failure::Local, &e));
                }
            }
            Action::Yank(url) => send(copy(&url).await.map_or_else(|e| failed(Failure::Local, &e), |()| Incoming::Done("copied".into()))),
            Action::SaveDraft { key, index, draft } => {
                send(backend.save_draft(key, index, &draft).await.unwrap_or_else(|e| failed(Failure::Draft { index }, &e)));
            }
            Action::UpdateDraft { key, id, draft } => {
                if let Err(e) = backend.forge_of(&key).update_draft(&key, id, &draft.payload()).await {
                    send(failed(Failure::Local, &e));
                }
            }
            Action::DeleteDraft { key, id } => {
                if let Err(e) = backend.forge_of(&key).delete_draft(&key, id).await {
                    send(failed(Failure::Local, &e));
                }
            }
            Action::Publish { key, approve, count } => {
                send(backend.publish(key, approve, count).await.unwrap_or_else(|e| failed(Failure::Publish, &e)));
            }
            Action::Resolve { key, thread, resolved } => send(backend.forge_of(&key).resolve(&key, &thread, resolved).await.map_or_else(
                |e| failed(Failure::Resolve { thread: thread.clone(), resolved }, &e),
                |()| Incoming::Resolved { key, thread: thread.clone(), resolved },
            )),
            Action::LoadFile { key, path, sha } => {
                let outcome = backend.forge_of(&key).file(&key, &path, &sha).await;
                send(outcome.map_or_else(|e| failed(Failure::Local, &e), |text| Incoming::File { key, path, text }));
            }
            Action::Apply { key, branch, suggestion } => {
                let outcome = backend.forge_of(&key).apply(&key, &branch, &suggestion).await;
                send(outcome.map_or_else(|e| failed(Failure::Apply, &e), |()| Incoming::Applied { key, branch }));
            }
            Action::LoadChecks { key, head } => {
                let outcome = backend.forge_of(&key).checks(&key, &head).await;
                send(outcome.map_or_else(|e| failed(Failure::Checks, &e), |checks| Incoming::Checks { key, checks }));
            }
            Action::Approve { key, approve } => {
                let outcome = backend.forge_of(&key).approve(&key, approve).await;
                send(outcome.map_or_else(|e| failed(Failure::Approve, &e), |()| Incoming::Approved { key, approve }));
            }
            Action::View { key, path, sha, line, note } => {
                let outcome = backend.view(key, &path, &sha, line, note).await;
                send(outcome.unwrap_or_else(|e| Incoming::Failed { what: Failure::Local, message: format!("{e:#}") }));
            }
            Action::Ask { key, id, request, fresh } => backend.ask(&key, id, &request, fresh, &send).await,
            Action::Triage(mr) => {
                send(backend.triage(&mr).await.unwrap_or_else(|e| Incoming::Failed { what: Failure::Triage, message: e.notice() }));
            }
            Action::Read { key, head, waits, files } => {
                send(
                    backend
                        .read(key, head, waits, files)
                        .await
                        .unwrap_or_else(|e| Incoming::Failed { what: Failure::Triage, message: e.notice() }),
                );
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

/// Jev, only when the config switches it on and a key is found; the environment can still switch it off.
fn jev(ai: &crate::config::Ai) -> Option<TypeSafe> {
    if !ai.typesafe.enabled {
        return None;
    }
    let keychain = crate::auth::SecurityCli::new(crate::auth::SERVICE);
    let (key, _) = ai::key(ai::Provider::Typesafe, &ai::KeyEnv::from_process(), &keychain).ok().flatten()?;
    TypeSafe::connect(&key).ok()
}

/// Claude, only when the config switches it on and a key is found.
fn claude(ai: &crate::config::Ai) -> Option<Claude> {
    if !ai.anthropic.enabled {
        return None;
    }
    let keychain = crate::auth::SecurityCli::new(crate::auth::SERVICE);
    let (key, _) = ai::key(ai::Provider::Anthropic, &ai::KeyEnv::from_process(), &keychain).ok().flatten()?;
    Some(Claude::new(key, ai.anthropic.model()))
}

/// `:set theme=`: the one setting the TUI writes, read back on the next start.
fn save_theme(name: &str) -> Result<()> {
    let mut config = crate::config::Config::load()?;
    config.tui.theme = Some(name.to_owned());
    config.save()
}

/// Runs `work` on tokio's blocking pool: file I/O and CPU-heavy work stay off the threads that drive the network.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(work).await.context("a background task stopped")?
}

fn failed(what: Failure, err: &anyhow::Error) -> Incoming {
    Incoming::Failed { what, message: err.to_string() }
}

impl Backend {
    /// `work` with this backend on tokio's blocking pool: the cache is plain file I/O, and
    /// building a review highlights every hunk.
    async fn off<T: Send + 'static>(&self, work: impl FnOnce(&Backend) -> T + Send + 'static) -> Result<T> {
        let me = self.clone();
        tokio::task::spawn_blocking(move || work(&me)).await.context("a background task stopped")
    }

    /// Claude's answer, piece by piece as it streams, or at once from the cache unless `fresh`.
    /// Only a finished answer is kept, so a cut or refused one is asked again next time.
    async fn ask(&self, key: &MrKey, id: u64, request: &Ask, fresh: bool, send: &impl Fn(Incoming)) {
        let part = |part: Part| Incoming::Answer { key: key.clone(), id, part };
        let Some(claude) = &self.claude else {
            send(part(Part::Failed("Claude is off".into())));
            return;
        };
        let cache_key = keys::answer(key, &anthropic::body(claude.model(), request).to_string());
        let (wanted, reading) = (key.clone(), cache_key.clone());
        let saved = self.off(move |b| b.cache_of(&wanted).read::<SavedAnswer>(&reading)).await.ok().flatten();
        if let Some(saved) = saved.filter(|_| !fresh) {
            let outcome = Outcome { stop: Stop::Done, usage: saved.usage, model: saved.model };
            send(part(Part::Done { outcome, cached_text: Some(saved.text) }));
            return;
        }
        let mut text = String::new();
        let streamed = claude
            .stream(request, |event| match event {
                anthropic::Event::Text(more) => {
                    text.push_str(&more);
                    send(part(Part::Text(more)));
                }
                anthropic::Event::Restart => {
                    text.clear();
                    send(part(Part::Restart));
                }
            })
            .await;
        match streamed {
            Ok(outcome) => {
                if outcome.stop == Stop::Done {
                    let (wanted, saved) = (key.clone(), SavedAnswer { text, model: outcome.model.clone(), usage: outcome.usage });
                    let _ = self.off(move |b| b.cache_of(&wanted).write(&cache_key, &saved)).await;
                }
                send(part(Part::Done { outcome, cached_text: None }));
            }
            Err(failure) => send(part(Part::Failed(failure.0))),
        }
    }

    /// Jev's verdict on a queue MR, from the cache while the MR has not moved.
    async fn triage(&self, mr: &crate::forge::QueueMr) -> Result<Incoming, Unavailable> {
        let key = mr.key();
        let wanted = key.clone();
        let cached: Option<Verdict> = self.off(move |b| b.cache_of(&wanted).read(&keys::verdict(&wanted))).await.ok().flatten();
        let verdict = if let Some(verdict) = cached.filter(|v| v.fresh_for(mr)) {
            verdict
        } else {
            let jev = self.jev.as_ref().ok_or_else(|| Unavailable("switched off".into()))?;
            let verdict = triage::judge_mr(jev, mr, Utc::now()).await?;
            let (wanted, saved) = (key.clone(), verdict.clone());
            let _ = self.off(move |b| b.cache_of(&wanted).write(&keys::verdict(&wanted), &saved)).await;
            verdict
        };
        Ok(Incoming::Triaged { key, verdict })
    }

    /// Jev's reading of an open MR at `head`, from the cache once read.
    async fn read(
        &self,
        key: MrKey,
        head: String,
        waits: Option<serde_json::Value>,
        files: Vec<(String, serde_json::Value)>,
    ) -> Result<Incoming, Unavailable> {
        let (wanted, at) = (key.clone(), head.clone());
        let cached = self.off(move |b| b.cache_of(&wanted).read(&keys::reading(&wanted, &at))).await.ok().flatten();
        let reading = if let Some(reading) = cached {
            reading
        } else {
            let jev = self.jev.as_ref().ok_or_else(|| Unavailable("switched off".into()))?;
            let reading = triage::judge_open(jev, waits, files).await?;
            let (wanted, at, saved) = (key.clone(), head.clone(), reading.clone());
            let _ = self.off(move |b| b.cache_of(&wanted).write(&keys::reading(&wanted, &at), &saved)).await;
            reading
        };
        Ok(Incoming::Read { key, head, reading })
    }

    /// Every project's queue also asks the other hosts I am logged in to; one that fails to
    /// answer is left out rather than failing the whole queue.
    async fn load_queue(&self, scope: Option<String>) -> Result<Incoming> {
        let queue = self.forge.queue(scope.as_deref()).await?;
        let (wanted, saved) = (scope.clone(), queue.clone());
        let _ = self.off(move |b| b.cache.write_entry(&keys::queue(wanted.as_deref()), &saved)).await;
        let others = if scope.is_none() {
            futures_util::future::join_all(self.others.iter().map(crate::ctx::Home::queue))
                .await
                .into_iter()
                .filter_map(Result::ok)
                .collect()
        } else {
            vec![]
        };
        Ok(self.queue_answer(scope, &queue, &others, false))
    }

    fn cached_queue(&self, scope: Option<String>) -> Option<Incoming> {
        let queue: Queue = self.cache.read_entry(&keys::queue(scope.as_deref()))?.value;
        let others: Vec<Queue> = if scope.is_none() { self.others.iter().filter_map(crate::ctx::Home::cached).collect() } else { vec![] };
        Some(self.queue_answer(scope, &queue, &others, true))
    }

    fn queue_answer(&self, scope: Option<String>, queue: &Queue, others: &[Queue], cached: bool) -> Incoming {
        let parts = std::iter::once(queue).chain(others).map(|q| q.sections(&self.watch_labels)).collect();
        let sections = Sections::merge(parts);
        let opened = self.opened_at(&sections);
        Incoming::Queue { scope, me: queue.me.clone(), sections, opened, cached }
    }

    /// The forge an MR lives on: its own host's when the queue merged several.
    fn forge_of(&self, key: &MrKey) -> &Forge {
        self.home_of(key).map_or(&self.forge, |home| &home.forge)
    }

    fn cache_of(&self, key: &MrKey) -> &Cache {
        self.home_of(key).map_or(&self.cache, |home| &home.cache)
    }

    fn home_of(&self, key: &MrKey) -> Option<&crate::ctx::Home> {
        let host = key.host.as_deref()?;
        self.others.iter().find(|home| home.host == host)
    }

    fn opened_at(&self, sections: &Sections) -> HashMap<MrKey, DateTime<Utc>> {
        [&sections.to_review, &sections.mine, &sections.watching, &sections.open, &sections.done]
            .into_iter()
            .flatten()
            .filter_map(|mr| {
                let key = mr.key();
                let state: MrState = self.cache_of(&key).read(&keys::state(&key))?;
                Some((key, state.opened_at?))
            })
            .collect()
    }

    fn state(&self, key: &MrKey) -> MrState {
        self.cache_of(key).read(&keys::state(key)).unwrap_or_default()
    }

    fn open_cached(&self, key: &MrKey) -> Option<Incoming> {
        let mr: Entry<Mr> = self.cache_of(key).read_entry(&keys::mr(key))?;
        let diffs: Vec<DiffFile> = self.cache_of(key).read(&keys::diffs(key, &mr.value.refs.head))?;
        let discussions: Vec<Discussion> = self.cache_of(key).read(&keys::discussions(key)).unwrap_or_default();
        let drafts: Vec<HeldDraft> = self.cache_of(key).read(&keys::drafts(key)).unwrap_or_default();
        let age = mr.age(Utc::now());
        let review = self.build(key, mr.value, &diffs, discussions, &drafts);
        Some(Incoming::Review { key: key.clone(), review: Box::new(review), cached: Some(age) })
    }

    async fn fetch_review(&self, key: MrKey) -> Result<Incoming> {
        let forge = self.forge_of(&key);
        let (mr, diffs, discussions, drafts) =
            tokio::try_join!(forge.mr(&key), forge.diffs(&key), forge.discussions(&key), forge.drafts(&key))?;
        self.off(move |b| {
            let cache = b.cache_of(&key);
            let _ = cache.write_entry(&keys::mr(&key), &mr);
            let _ = cache.write(&keys::diffs(&key, &mr.refs.head), &diffs);
            let _ = cache.write(&keys::discussions(&key), &discussions);
            let _ = cache.write(&keys::drafts(&key), &drafts);
            let state = MrState { opened_at: Some(Utc::now()), ..b.state(&key) };
            let _ = cache.write(&keys::state(&key), &state);
            let review = b.build(&key, mr, &diffs, discussions, &drafts);
            Incoming::Review { key, review: Box::new(review), cached: None }
        })
        .await
    }

    /// Posts the draft unless the forge already lists it: a retry after a lost answer never doubles a note.
    async fn save_draft(&self, key: MrKey, index: usize, draft: &Draft) -> Result<Incoming> {
        let held = self.forge_of(&key).drafts(&key).await?;
        let id = match held.iter().find(|note| draft.same_as(&Draft::held(note))) {
            Some(note) => note.id,
            None => self.forge_of(&key).create_draft(&key, &draft.payload()).await?.id,
        };
        Ok(Incoming::DraftSaved { key, index, id })
    }

    async fn publish(&self, key: MrKey, approve: bool, count: usize) -> Result<Incoming> {
        self.forge_of(&key).publish(&key, approve).await?;
        let wanted = key.clone();
        let _ = self.off(move |b| b.cache_of(&wanted).write(&keys::drafts(&wanted), &Vec::<HeldDraft>::new())).await;
        Ok(Incoming::Published { key, approved: approve, count })
    }

    async fn fetch_discussions(&self, key: MrKey) -> Result<Incoming> {
        let discussions = self.forge_of(&key).discussions(&key).await?;
        let (wanted, saved) = (key.clone(), discussions.clone());
        let _ = self.off(move |b| b.cache_of(&wanted).write(&keys::discussions(&wanted), &saved)).await;
        Ok(Incoming::Discussions { key, discussions })
    }

    /// The file ready for the reader's program: the checkout's own when it sits on `sha`,
    /// else a private read-only copy of what the forge serves.
    async fn view(&self, key: MrKey, path: &str, sha: &str, line: u32, note: Option<String>) -> Result<Incoming> {
        crate::open::safe_path(path)?;
        let checkout = self.checkout.as_ref().filter(|_| key.host.is_none()).and_then(|c| c.head().map(|head| (c, head)));
        let source = crate::open::Source::pick(&key.project, sha, checkout.as_ref().map(|(c, head)| (c.project.as_str(), head.as_str())));
        let (file, dir, note) = if let (crate::open::Source::Checkout, Some((checkout, _))) = (source, checkout) {
            (checkout.root.join(path), None, Some("your checkout · edits are real".to_owned()))
        } else {
            let text = self.file_text(&key, path, sha).await?;
            let name = path.to_owned();
            let (dir, file) = blocking(move || crate::open::write_private(&name, text.as_bytes())).await?;
            (file, Some(std::sync::Arc::new(dir)), note)
        };
        let command = crate::open::command_for(path, &self.open, env("VISUAL").as_deref(), env("EDITOR").as_deref());
        let argv = crate::open::argv(&command, &file.display().to_string(), line, dir.is_some())?;
        let shown = format!("{}:{line}", path.rsplit('/').next().unwrap_or(path));
        Ok(Incoming::ViewReady { key, view: crate::open::View { argv, shown, note, _copy: dir } })
    }

    /// A file at a commit never changes, so the cache serves it forever.
    async fn file_text(&self, key: &MrKey, path: &str, sha: &str) -> Result<String> {
        let cache_key = keys::file(key, sha, path);
        let (wanted, reading) = (key.clone(), cache_key.clone());
        if let Some(text) = self.off(move |b| b.cache_of(&wanted).read::<String>(&reading)).await.ok().flatten() {
            return Ok(text);
        }
        let short = sha.get(..8).unwrap_or(sha);
        let text = self.forge_of(key).file(key, path, sha).await.with_context(|| format!("{path} at {short} · v tries again"))?;
        anyhow::ensure!(
            text.len() <= crate::open::MAX_BYTES,
            "too large to open here ({} MB) · o opens it in the browser",
            text.len() / (1024 * 1024)
        );
        let (wanted, saved) = (key.clone(), text.clone());
        let _ = self.off(move |b| b.cache_of(&wanted).write(&cache_key, &saved)).await;
        Ok(text)
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
        self.cache_of(key).write(&keys::state(key), &state)
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

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// The text goes in as arguments, never into the script, so an MR title cannot run AppleScript.
async fn notify(title: &str, body: &str) -> Result<()> {
    let status = tokio::process::Command::new("osascript")
        .args(["-e", "on run argv", "-e", "display notification (item 2 of argv) with title (item 1 of argv)", "-e", "end run"])
        .args([title, body])
        .stdout(std::process::Stdio::null())
        .status()
        .await
        .context("running osascript")?;
    anyhow::ensure!(status.success(), "osascript failed");
    Ok(())
}

async fn open_url(url: &str) -> Result<()> {
    let status = tokio::process::Command::new("open").arg(url).status().await.context("running open")?;
    anyhow::ensure!(status.success(), "open failed");
    Ok(())
}

async fn copy(text: &str) -> Result<()> {
    use tokio::io::AsyncWriteExt;
    let mut child = tokio::process::Command::new("pbcopy").stdin(std::process::Stdio::piped()).spawn().context("running pbcopy")?;
    let mut stdin = child.stdin.take().context("pbcopy stdin")?;
    stdin.write_all(text.as_bytes()).await?;
    drop(stdin);
    anyhow::ensure!(child.wait().await?.success(), "pbcopy failed");
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::forge::gitlab::Client;
    use crate::forge::gitlab::fixture::key;
    use crate::forge::{LineRef, Position, Refs};

    /// Never polled, so nothing runs: the types alone prove the helpers wait without blocking a runtime thread.
    #[test]
    fn the_macos_helpers_are_futures_so_they_never_block_the_runtime() {
        fn future<F: std::future::Future<Output = Result<()>>>(_: F) {}
        future(notify("title", "body"));
        future(open_url("https://gitlab.com"));
        future(copy("text"));
    }

    #[tokio::test]
    async fn off_runs_the_work_with_the_backend_and_hands_back_its_result() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend { cache: Cache::in_dir(dir.path()), ..backend_on_nothing() };
        backend.cache.write(&keys::queue(None), &7_u32).unwrap();
        assert_eq!(backend.off(|b| b.cache.read::<u32>(&keys::queue(None))).await.unwrap(), Some(7));
    }

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
        let backend = Backend {
            others: vec![],
            forge: test_forge(),
            cache,
            fold_globs: vec![],
            watch_labels: vec![],
            inline: InlineRule::default(),
            open: crate::config::Open::default(),
            checkout: None,
            jev: None,
            claude: None,
        };
        backend.save_state(&key(), FoldState::default(), BTreeMap::from([("a.rs".to_owned(), "f1".to_owned())]), true).unwrap();
        let state = backend.state(&key());
        assert_eq!(
            (state.viewed_files, state.split),
            (BTreeMap::from([("a.rs".to_owned(), "f1".to_owned())]), true),
            "the split choice is remembered per MR"
        );
    }

    #[tokio::test]
    async fn jev_is_asked_once_per_mr_state_and_answers_from_the_cache_after() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend {
            others: vec![],
            forge: test_forge(),
            cache: Cache::in_dir(dir.path()),
            fold_globs: vec![],
            watch_labels: vec![],
            inline: InlineRule::default(),
            open: crate::config::Open::default(),
            checkout: None,
            jev: None,
            claude: None,
        };
        let mr = crate::forge::gitlab::fixture::queue(include_str!("../forge/gitlab/fixtures/queue.json")).review_requested[0].clone();
        let verdict = Verdict { urgency: 2.8, size: triage::Size::Large, seen: mr.updated_at };
        backend.cache.write(&keys::verdict(&mr.key()), &verdict).unwrap();
        let Ok(Incoming::Triaged { verdict: cached, .. }) = backend.triage(&mr).await else { panic!("the cache answers") };
        assert_eq!(cached, verdict);
        let moved = crate::forge::QueueMr { updated_at: mr.updated_at + chrono::TimeDelta::minutes(5), ..mr };
        assert!(backend.triage(&moved).await.is_err(), "a moved MR needs Jev, which is off here");
        let reading = triage::Reading { waits_on_me: true, risks: BTreeMap::new() };
        backend.cache.write(&keys::reading(&key(), "abc"), &reading).unwrap();
        let Ok(Incoming::Read { reading: read, .. }) = backend.read(key(), "abc".into(), None, vec![]).await else { panic!("cached") };
        assert!(read.waits_on_me);
    }

    #[tokio::test]
    async fn an_answer_streams_once_then_comes_from_the_cache_until_asked_fresh() {
        use wiremock::matchers::method;
        let server = wiremock::MockServer::start().await;
        let stream = [
            r#"{"type":"message_start","message":{"model":"claude-opus-5","usage":{"input_tokens":10,"cache_read_input_tokens":900}}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Looks fine."}}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3}}"#,
        ]
        .iter()
        .map(|data| format!("data: {data}\n\n"))
        .collect::<String>();
        wiremock::Mock::given(method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(stream))
            .expect(2)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend {
            others: vec![],
            forge: test_forge(),
            cache: Cache::in_dir(dir.path()),
            fold_globs: vec![],
            watch_labels: vec![],
            inline: InlineRule::default(),
            open: crate::config::Open::default(),
            checkout: None,
            jev: None,
            claude: Some(Claude::with_base(&server.uri(), ai::Secret::new("sk-ant-test"), "claude-opus-5")),
        };
        let request = Ask { system: vec![], turns: vec![anthropic::Turn { role: anthropic::Role::User, text: "ok?".into() }] };
        let heard = std::sync::Mutex::new(vec![]);
        let listen = |incoming: Incoming| heard.lock().unwrap().push(incoming);
        backend.ask(&key(), 1, &request, false, &listen).await;
        backend.ask(&key(), 2, &request, false, &listen).await;
        backend.ask(&key(), 3, &request, true, &listen).await;
        let heard = heard.into_inner().unwrap();
        let cached: Vec<_> = heard
            .iter()
            .filter_map(|i| match i {
                Incoming::Answer { id, part: Part::Done { cached_text, .. }, .. } => Some((*id, cached_text.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(cached, vec![(1, None), (2, Some("Looks fine.".into())), (3, None)], "the second answer never reaches the API");
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
        Backend {
            others: vec![],
            forge,
            cache: Cache::in_dir(dir.path()),
            fold_globs: vec![],
            watch_labels: vec![],
            inline: InlineRule::default(),
            open: crate::config::Open::default(),
            checkout: None,
            jev: None,
            claude: None,
        }
    }

    #[test]
    fn an_mr_from_another_host_reaches_that_host_and_its_cache() {
        let dir = tempfile::tempdir().unwrap();
        let github =
            crate::forge::github::Client::new(&crate::auth::Credentials { host: "github.com".into(), token: "ghp_xxxx".into() }).unwrap();
        let other =
            crate::ctx::Home { host: "github.com".into(), forge: Forge::GitHub(github), cache: Cache::in_dir(dir.path().join("gh")) };
        let queue = crate::forge::gitlab::fixture::queue(include_str!("../forge/gitlab/fixtures/queue.json"));
        other.cache.write_entry(&keys::queue(None), &queue.clone().on_host("github.com")).unwrap();
        let backend = Backend { others: vec![other], cache: Cache::in_dir(dir.path().join("gl")), ..backend_on_nothing() };
        backend.cache.write_entry(&keys::queue(None), &queue).unwrap();
        let there = MrKey { host: Some("github.com".into()), ..key() };
        assert_eq!(
            (backend.forge_of(&there).kind(), backend.forge_of(&key()).kind()),
            (crate::forge::Kind::GitHub, crate::forge::Kind::GitLab)
        );
        let Some(Incoming::Queue { sections, .. }) = backend.cached_queue(None) else { panic!("both caches answer") };
        assert_eq!(
            sections.to_review.iter().filter(|mr| mr.host.as_deref() == Some("github.com")).count(),
            queue.sections(&[]).to_review.len()
        );
        assert!(backend.cached_queue(Some("acme/widgets".into())).is_none(), "a scoped queue keeps to its own host");
    }

    fn backend_on_nothing() -> Backend {
        Backend {
            others: vec![],
            forge: test_forge(),
            cache: Cache::in_dir(std::path::Path::new("/nonexistent")),
            fold_globs: vec![],
            watch_labels: vec![],
            inline: InlineRule::default(),
            open: crate::config::Open::default(),
            checkout: None,
            jev: None,
            claude: None,
        }
    }

    fn draft_at(new_line: u32, body: &str) -> Draft {
        let refs = Refs { base: "a".into(), start: "a".into(), head: "b".into() };
        let line = LineRef { old: None, new: Some(new_line) };
        Draft::on(Position { refs, old_path: "src/a.rs".into(), new_path: "src/a.rs".into(), line, start: None }, body)
    }

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
        let backend = Backend { open: crate::config::Open { default: Some("nvim".into()), files: BTreeMap::new() }, ..backend_on(&server) };
        for _ in 0..2 {
            let Incoming::ViewReady { view, .. } = backend.view(key(), "src/pay/charge.rs", "bbbb", 12, None).await.unwrap() else {
                panic!("not ready")
            };
            let file = std::path::Path::new(&view.argv[3]);
            assert_eq!(view.argv[..3], ["nvim", "-R", "+12"], "a copy opens read-only");
            assert_eq!(std::fs::read_to_string(file).unwrap(), "fn charge() {}\n");
            assert_eq!(std::fs::metadata(file).unwrap().permissions().mode() & 0o777, 0o400);
            assert_eq!(view.shown, "charge.rs:12");
        }
        let refused = backend.view(key(), "../../etc/passwd", "bbbb", 1, None).await.unwrap_err();
        assert!(refused.to_string().contains("does not trust"), "{refused}");
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
