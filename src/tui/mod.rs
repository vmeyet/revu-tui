pub mod app;
pub mod theme;
pub mod ui;

use crate::ctx::Ctx;
use anyhow::Result;
use app::{Action, App, Incoming};
use crossterm::event::{Event, EventStream, KeyEventKind};
use futures_util::StreamExt;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

const TICK: Duration = Duration::from_millis(100);

pub async fn run(ctx: Ctx) -> Result<()> {
    let theme = match ctx.config.tui.theme.as_deref() {
        Some(name) => theme::Theme::named(name)
            .ok_or_else(|| anyhow::anyhow!("config `tui.theme = \"{name}\"` is not a theme (try {})", theme::Theme::NAMES.join(", ")))?,
        None => theme::Theme::default(),
    };
    let mut app = App::new(theme, ctx.gitlab.host().to_owned(), ctx.config.username.clone().unwrap_or_default());
    let mut terminal = ratatui::init();
    let outcome = event_loop(&mut terminal, &mut app, &ctx).await;
    ratatui::restore();
    outcome
}

async fn event_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut App, ctx: &Ctx) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Incoming>();
    let mut events = EventStream::new();
    let mut ticks = tokio::time::interval(TICK);
    for action in app.start() {
        spawn(action, ctx, tx.clone());
    }
    while !app.should_quit {
        terminal.draw(|f| ui::draw(f, app))?;
        let actions = tokio::select! {
            Some(event) = events.next() => match event? {
                Event::Key(key) if key.kind != KeyEventKind::Release => app.handle_key(key),
                Event::Resize(..) => vec![],
                _ => vec![],
            },
            Some(incoming) = rx.recv() => { app.apply(incoming); vec![] }
            _ = ticks.tick() => { app.now = Instant::now(); vec![] }
        };
        for action in actions {
            spawn(action, ctx, tx.clone());
        }
    }
    Ok(())
}

/// Every action runs in its own task and answers through `Incoming`; the loop never awaits the network.
fn spawn(action: Action, ctx: &Ctx, tx: mpsc::UnboundedSender<Incoming>) {
    let gitlab = ctx.gitlab.clone();
    tokio::spawn(async move {
        let incoming = match action {
            Action::LoadMe => match gitlab.me().await {
                Ok(me) => Incoming::Me(me),
                Err(err) => Incoming::Failed(err.to_string()),
            },
        };
        let _ = tx.send(incoming);
    });
}
