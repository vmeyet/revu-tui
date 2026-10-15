//! The review TUI: an event loop over a pure `App`, with the network at its edge.
mod actions;
mod answer_view;
mod app;
mod backend;
mod brief_view;
mod clipboard;
mod complete;
mod compose;
mod diff_view;
mod drag;
mod field;
mod ground;
pub(crate) mod help;
mod images;
mod lanes;
mod motion;
mod outline_view;
mod palette;
mod palette_view;
mod pipeline_view;
#[cfg(feature = "prose")]
mod prose;
mod prose_view;
mod publish_view;
mod queue_view;
mod screen;
mod share_view;
mod table;
mod taps_view;
mod theme;
mod thread_view;
mod toward;
mod tree_view;
mod ui;
mod whats_new_view;

use crate::ai;
use crate::ai::anthropic::Claude;
use crate::ai::typesafe::TypeSafe;
use crate::blocking;
use crate::cache::Cache;
use crate::ctx::Ctx;
use crate::log::OrWarn as _;
use actions::{failed, spawn};
use anyhow::Result;
use app::{Action, App, Failure, Incoming, Input, Settings};
use backend::Backend;
use chrono::Utc;
use crossterm::event::{Event, EventStream, KeyEventKind};
use futures_util::StreamExt;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

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
    let checkout = std::env::current_dir().ok().and_then(|dir| crate::open::Checkout::find(&dir, ctx.forge.host()));
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
        agent: agent(&ctx.config.agent, ctx.config.tui.widths().compact, checkout.as_ref()),
        checkout,
        jev: jev(&ctx.config.ai),
        claude: claude(&ctx.config.ai),
        opening: std::sync::Arc::default(),
        saved: std::sync::Arc::default(),
        changed: std::sync::Arc::default(),
        draft_lanes: lanes::Lanes::default(),
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
        widths: ctx.config.tui.widths(),
        tap_bar: ctx.config.tui.tap_bar,
        views: ctx.config.queue.views.clone().into_iter().collect(),
        share: crate::share::targets(&ctx.config.share),
        last_share: crate::share::last_share(&Cache::shared()),
        prefetch: ctx.config.queue.prefetch,
        ascii: ctx.config.tui.ascii,
        reactions: crate::reactions::read(&Cache::shared()),
        quit_confirm: ctx.config.keys.quit_confirm,
        usage: ctx.config.usage.enabled.then(|| crate::usage::Tally::after(&crate::usage::read(&Cache::shared()))),
        whats_new: whats_new(&Cache::shared(), crate::changelog::Changelog::embedded()?),
        ignored_settings: crate::config::warnings(),
        agent: backend.agent.is_some(),
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
        let next_tick = ticked + if changed { app.motion().tick() } else { IDLE_TICK };
        let actions = tokio::select! {
            Some(event) = keys.next() => match event? {
                Event::Key(key) if key.kind != KeyEventKind::Release => { wake(app); app.handle_key(key) }
                Event::Mouse(mouse) => { wake(app); app.handle_mouse(mouse) }
                Event::FocusGained => { wake(app); app.terminal_focus(true); vec![] }
                Event::FocusLost => { wake(app); app.terminal_focus(false); vec![] }
                _ => vec![],
            },
            Some(incoming) = rx.recv() => {
                wake(app);
                // A failure backs off by the wait its request was just told.
                app.rate = backend.forge.rate();
                app.apply_all(std::iter::once(incoming).chain(std::iter::from_fn(|| rx.try_recv().ok())))
            }
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
                other => {
                    if let Action::Copy { text, .. } = &other {
                        clipboard::print(text);
                    }
                    spawn(other, backend, tx.clone());
                }
            }
        }
        if let Some(view) = app.take_view() {
            view_inline(terminal, screen, &mut keys, app, view);
            shown.forget();
        }
        if let (Some(handoff), Some(agent)) = (app.take_handoff(), &backend.agent) {
            let outcome = lend(terminal, screen, &mut keys, || agent.run.foreground(&handoff.call));
            wake(app);
            app.apply(Incoming::SessionLeft { said: handoff.said, outcome });
            shown.forget();
        }
    }
    app.tick();
    if let Some(counts) = app.take_usage() {
        save_usage(counts).await;
    }
    Ok(())
}

/// What is new since the last start. This start is remembered at once, so the next one has nothing to say again.
fn whats_new(cache: &Cache, changelog: crate::changelog::Changelog) -> app::WhatsNew {
    let key = crate::cache::keys::news();
    let seen = cache.read::<String>(&key).and_then(|position| position.parse().ok()).unwrap_or_else(|| changelog.previous());
    let here = changelog.position();
    if seen != here {
        cache.write(&key, &here.to_string()).or_warn("remembering the changelog position");
    }
    app::WhatsNew { changelog, seen }
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
    blocking::run(move || crate::usage::record(&Cache::shared(), today, &counts)).await.or_warn("saving the usage counts");
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
        Ok(edited) => app.apply(Incoming::Composed { input, edited }),
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
        queue!(
            out,
            cursor::SavePosition,
            cursor::MoveTo(link.x, link.y),
            style::SetForegroundColor(fg.into_crossterm()),
            style::SetBackgroundColor(bg.into_crossterm()),
            style::Print(format!("\x1b]8;;{}\x1b\\{}\x1b]8;;\x1b\\", link.url, link.text)),
            style::ResetColor,
            cursor::RestorePosition,
        )
        .or_warn("drawing a link");
    }
    out.flush().or_warn("drawing the links");
}

/// Jev, only when the config switches it on and a key is found; the environment can still switch it off.
fn jev(ai: &crate::config::Ai) -> Option<TypeSafe> {
    if !ai.typesafe.enabled {
        return None;
    }
    let keychain = crate::auth::SecurityCli::new(crate::auth::SERVICE);
    if let Some(reason) = crate::ai::typesafe::disabled_reason() {
        tracing::info!("Jev is off: {reason}");
        return None;
    }
    let (key, _) = ai_key(ai::Provider::Typesafe, &keychain)?;
    TypeSafe::connect(&key).inspect_err(|e| tracing::warn!("Jev is off: {e}")).ok()
}

/// Claude, only when the config switches it on and a key is found.
fn claude(ai: &crate::config::Ai) -> Option<Claude> {
    if !ai.anthropic.enabled {
        return None;
    }
    let keychain = crate::auth::SecurityCli::new(crate::auth::SERVICE);
    let (key, _) = ai_key(ai::Provider::Anthropic, &keychain)?;
    Claude::new(key, ai.anthropic.model()).inspect_err(|e| tracing::warn!("Claude is off: {e:#}")).ok()
}

/// A keychain that cannot be read leaves the provider off, with the reason in the log.
fn ai_key(provider: ai::Provider, keychain: &crate::auth::SecurityCli) -> Option<(ai::Secret, ai::KeySource)> {
    let found = ai::key(provider, &ai::KeyEnv::from_process(), keychain);
    found.inspect_err(|e| tracing::warn!("reading the {provider} key failed: {e:#}")).ok().flatten()
}

/// The agent of `[agent]`, when it is switched on, with what this machine says: the home folder, Claude
/// Code's own folder, whether revu runs in tmux, and the checkout it was started in.
fn agent(config: &crate::config::Agent, compact_width: u16, checkout: Option<&crate::open::Checkout>) -> Option<crate::agent::Agent> {
    if !config.enabled {
        return None;
    }
    let home = dirs::home_dir()?;
    let machine = crate::agent::Machine {
        claude_home: std::env::var_os("CLAUDE_CONFIG_DIR").map_or_else(|| home.join(".claude"), std::path::PathBuf::from),
        home,
        tmux: std::env::var_os("TMUX").is_some_and(|socket| !socket.is_empty()),
        terminal: crate::agent::Terminal::detect(|name| std::env::var(name).ok()),
        compact_width,
        started_in: checkout.map(|checkout| (checkout.project.to_string(), checkout.root.clone())),
    };
    Some(crate::agent::Agent::new(config, machine, std::sync::Arc::new(crate::agent::Programs)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn what_is_new_is_announced_on_the_first_start_after_an_update_only() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::in_dir(dir.path());
        let changelog = crate::changelog::tests::sample();
        let first = whats_new(&cache, changelog.clone());
        assert_eq!(
            first.announcement().as_deref(),
            Some("revu 0.6.0 · 4 new things · :news"),
            "nothing remembered: the last release is new"
        );
        assert_eq!(whats_new(&cache, changelog.clone()).announcement(), None, "never twice for the same version");
        let text = crate::changelog::tests::SAMPLE.replace("## Unreleased", "## 0.7.0 - 2026-10-20");
        let updated = crate::changelog::Changelog::parse(&text, "0.7.0".parse().unwrap()).unwrap();
        assert_eq!(whats_new(&cache, updated.clone()).announcement().as_deref(), Some("revu 0.7.0 · 1 new thing · :news"));
        assert_eq!(whats_new(&cache, updated).announcement(), None);
        assert_eq!(whats_new(&cache, changelog).announcement(), None, "an older binary has nothing new");
    }

    #[test]
    fn a_build_from_main_announces_only_the_entries_added_since_the_last_one() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::in_dir(dir.path());
        let changelog = crate::changelog::tests::sample();
        cache.write(&crate::cache::keys::news(), &"0.6.0").unwrap();
        assert_eq!(whats_new(&cache, changelog.clone()).announcement().as_deref(), Some("revu 0.6.0 · 1 new thing · :news"));
        assert_eq!(whats_new(&cache, changelog).announcement(), None, "the same build again");
    }

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
