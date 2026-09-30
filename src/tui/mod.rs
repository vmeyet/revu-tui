//! The review TUI: an event loop over a pure `App`, with the network at its edge.
mod actions;
mod answer_view;
mod app;
mod backend;
mod brief_view;
mod complete;
mod compose;
mod diff_view;
mod drag;
mod field;
mod ground;
pub(crate) mod help;
mod images;
mod palette;
mod palette_view;
mod pipeline_view;
mod publish_view;
mod queue_view;
mod screen;
mod share_view;
mod table;
mod theme;
mod thread_view;
mod tree_view;
mod ui;

use crate::ai;
use crate::ai::anthropic::Claude;
use crate::ai::typesafe::TypeSafe;
use crate::cache::Cache;
use crate::ctx::Ctx;
use actions::{failed, spawn};
use anyhow::{Context as _, Result};
use app::{Action, App, Failure, Incoming, Input, Settings};
use backend::Backend;
use chrono::Utc;
use crossterm::event::{Event, EventStream, KeyEventKind};
use futures_util::StreamExt;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

const TICK: Duration = Duration::from_millis(100);
/// A screen that stood still for a tick is looked at once a second: ages, toasts, pulses and polls need no more.
const IDLE_TICK: Duration = Duration::from_secs(1);

/// Runs the review TUI until the user quits, restoring the terminal on the way out; on `start` in zen when given.
pub async fn run(ctx: Ctx, start: Option<crate::forge::MrKey>) -> Result<()> {
    let theme = match ctx.config.tui.theme.as_deref() {
        Some(name) => theme::Theme::named(name)
            .ok_or_else(|| anyhow::anyhow!("config `tui.theme = \"{name}\"` is not a theme (try {})", theme::Theme::NAMES.join(", ")))?,
        None => theme::Theme::default(),
    };
    let ground = ground::ask();
    let pictures = if ctx.config.tui.images.unwrap_or(true) { images::ask_terminal() } else { None };
    let theme = ground.map_or(theme, |ground| theme.with_ground(ground));
    let others = ctx.others();
    let hosts = ctx.hosts(&others);
    let backend = Backend {
        forge: ctx.forge.clone(),
        cache: ctx.cache.clone(),
        others,
        fold_globs: ctx.config.review.fold.clone(),
        watch_labels: ctx.config.queue.watch_labels.clone(),
        rules: ctx.config.queue.rules.clone(),
        ready_command: ctx.config.queue.ready.command.clone(),
        inline: ctx.config.review.inline(),
        open: ctx.config.open.clone(),
        checkout: std::env::current_dir().ok().and_then(|dir| crate::open::Checkout::find(&dir, ctx.forge.host())),
        jev: jev(&ctx.config.ai),
        claude: claude(&ctx.config.ai),
        opening: std::sync::Arc::default(),
        saved: std::sync::Arc::default(),
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
        keymap: crate::keymap::Keymap::new(&ctx.config.keys)?,
        pictures,
        queue_layout: ctx.config.tui.queue,
        zen_width: ctx.config.tui.zen_width,
        views: ctx.config.queue.views.clone().into_iter().collect(),
        share: crate::share::targets(&ctx.config.share),
        prefetch: ctx.config.queue.prefetch,
        ascii: ctx.config.tui.ascii,
        quit_confirm: ctx.config.keys.quit_confirm,
        usage: ctx.config.usage.enabled,
    };
    let mut app = App::new(settings);
    let first = match start {
        Some(key) => app.start_on(key),
        None => app.start(),
    };
    tokio::spawn({
        let backend = backend.clone();
        async move { backend.prune_daily().await }
    });
    let (mut terminal, screen) = screen::Screen::enter();
    let outcome = event_loop(&mut terminal, &screen, &mut app, &backend, first).await;
    screen.leave();
    outcome
}

async fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    screen: &screen::Screen,
    app: &mut App,
    backend: &Backend,
    first: Vec<Action>,
) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Incoming>();
    let mut keys = Keys::new();
    let mut shown = Shown::default();
    let mut ticked = Instant::now();
    let mut flushed = Instant::now();
    for action in first {
        spawn(action, backend, tx.clone());
    }
    while !app.should_quit {
        if flushed.elapsed() >= USAGE_FLUSH {
            flushed = Instant::now();
            if let Some(counts) = app.take_usage() {
                tokio::spawn(save_usage(counts));
            }
        }
        let frame = terminal.draw(|f| ui::draw(f, app))?;
        let changed = shown.changed(frame.buffer);
        if changed {
            print_links(&hyperlinks(frame.buffer, &app.links));
        }
        let next_tick = ticked + if changed { TICK } else { IDLE_TICK };
        let actions = tokio::select! {
            Some(event) = keys.next() => match event? {
                Event::Key(key) if key.kind != KeyEventKind::Release => { wake(app); app.handle_key(key) }
                Event::Mouse(mouse) => { wake(app); app.handle_mouse(mouse) }
                _ => vec![],
            },
            Some(incoming) = rx.recv() => { wake(app); app.apply_all(std::iter::once(incoming).chain(std::iter::from_fn(|| rx.try_recv().ok()))) }
            () = tokio::time::sleep_until(next_tick.into()) => {
                ticked = Instant::now();
                wake(app);
                app.rate = backend.forge.rate();
                app.tick()
            }
        };
        for action in actions {
            match action {
                Action::Compose { input, draft } => {
                    for follow_up in compose_inline(terminal, screen, &mut keys, app, input, &draft) {
                        spawn(follow_up, backend, tx.clone());
                    }
                    shown.forget();
                }
                other => spawn(other, backend, tx.clone()),
            }
        }
        if let Some(view) = app.take_view() {
            view_inline(terminal, screen, &mut keys, app, view);
            shown.forget();
        }
    }
    app.tick();
    if let Some(counts) = app.take_usage() {
        save_usage(counts).await;
    }
    Ok(())
}

/// The last frame on the terminal: an unchanged one needs no links printed and no quick tick after it.
#[derive(Default)]
struct Shown(Option<ratatui::buffer::Buffer>);

impl Shown {
    /// Keeps `frame` and says whether the terminal showed something else before it.
    fn changed(&mut self, frame: &ratatui::buffer::Buffer) -> bool {
        let changed = self.0.as_ref() != Some(frame);
        if changed {
            self.0 = Some(frame.clone());
        }
        changed
    }

    /// Another program drew over the terminal: whatever comes next is new to it.
    fn forget(&mut self) {
        self.0 = None;
    }
}

/// `[usage]` counts go to the cache now and then, not on every key.
const USAGE_FLUSH: Duration = Duration::from_secs(60);

/// Adds `counts` to today's line of the usage file; a failure costs a minute of counts, nothing else.
async fn save_usage(counts: crate::usage::Counts) {
    let today = chrono::Local::now().date_naive();
    let _ = blocking(move || crate::usage::record(&Cache::shared(), today, &counts)).await;
}

/// The reader's program owns the terminal until it exits; the app, untouched, draws again after.
fn view_inline(terminal: &mut ratatui::DefaultTerminal, screen: &screen::Screen, keys: &mut Keys, app: &mut App, view: crate::open::View) {
    let outcome = lend(terminal, screen, keys, || crate::open::run(&view.argv));
    wake(app);
    app.apply(Incoming::Viewed { view, outcome });
}

/// Another program owns the terminal and its keys while `run` lasts.
fn lend<T>(terminal: &mut ratatui::DefaultTerminal, screen: &screen::Screen, keys: &mut Keys, run: impl FnOnce() -> T) -> T {
    keys.pause();
    let outcome = screen.lend(terminal, run);
    keys.resume();
    outcome
}

/// The clock stood still while a program had the terminal: what comes next is timed from now.
fn wake(app: &mut App) {
    app.now = Instant::now();
    app.today = Utc::now();
}

/// The editor owns the terminal for a while; the answer goes through `apply` like any other.
fn compose_inline(
    terminal: &mut ratatui::DefaultTerminal,
    screen: &screen::Screen,
    keys: &mut Keys,
    app: &mut App,
    input: Input,
    draft: &str,
) -> Vec<Action> {
    let edited = lend(terminal, screen, keys, || compose::edit(draft));
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

/// Runs `work` on tokio's blocking pool: file I/O and CPU-heavy work stay off the threads that drive the network.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(work).await.context("a background task stopped")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_is_new_until_the_terminal_shows_it_and_again_once_another_program_drew() {
        let area = ratatui::layout::Rect::new(0, 0, 4, 1);
        let (idle, busy) = (ratatui::buffer::Buffer::with_lines(["⠋ ok"]), ratatui::buffer::Buffer::with_lines(["⠙ ok"]));
        let mut shown = Shown::default();
        assert!(shown.changed(&idle));
        assert!(!shown.changed(&idle));
        assert!(shown.changed(&busy));
        shown.forget();
        assert!(shown.changed(&busy));
        assert!(shown.changed(&ratatui::buffer::Buffer::empty(area)));
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
}
