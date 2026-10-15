//! The action runner: each action the app asks for runs in its own task and answers through `Incoming`.
use super::app::{self, Action, Failure, Incoming, Part, Post};
use super::backend::{Backend, MrState, read_cache, run_blocking};
use crate::blocking;
use crate::cache::Cache;
use crate::forge::{Forge, MrKey};
use crate::log::OrWarn as _;
use anyhow::{Context as _, Result};
use futures_util::StreamExt;
use tokio::sync::mpsc;

/// How many MRs load ahead at once: enough to fill the cache soon, few enough to leave the network to the reader.
const AHEAD: usize = 2;

/// Every action runs in its own task and answers through `Incoming`; the loop never awaits the network.
pub(super) fn spawn(action: Action, backend: &Backend, tx: mpsc::UnboundedSender<Incoming>) {
    let backend = backend.clone();
    tokio::spawn(async move {
        let send = |incoming: Incoming| {
            log_failure(&incoming);
            #[expect(clippy::let_underscore_must_use, reason = "the loop drops its receiver only when revu quits")]
            let _ = tx.send(incoming);
        };
        match action {
            Action::LoadQueue { scope, from_cache } => {
                if from_cache {
                    let wanted = scope.clone();
                    if let Some(view) = read_cache(&backend.saves, move |saves| saves.queue_view(wanted.as_ref())).await {
                        send(Incoming::QueueView { scope: scope.clone(), view });
                    }
                    let wanted = scope.clone();
                    let pins = read_cache(&backend.saves, move |saves| saves.pins(wanted.as_ref())).await;
                    send(Incoming::Pins { scope: scope.clone(), pins });
                }
                let wanted = scope.clone();
                if let Some(cached) = read_cache(&backend.queues, move |queues| queues.cached(wanted)).await.filter(|_| from_cache) {
                    send(cached);
                }
                match backend.queues.load(scope).await {
                    Ok((answer, follow)) => {
                        let counted = match &answer {
                            Incoming::Queue { sections, .. } => Some(sections.clone()),
                            _ => None,
                        };
                        send(answer);
                        if let Some(sections) = counted
                            && let Ok(progress) = run_blocking(&backend.saves, move |saves| saves.progress(&sections)).await
                        {
                            send(Incoming::Progress(progress));
                        }
                        follow.into_iter().for_each(send);
                    }
                    Err(e) => send(failed(Failure::Queue, &e)),
                }
            }
            Action::ChangeQueueView { scope, change, order } => {
                let saved = run_blocking(&backend.saves, move |saves| saves.change_queue_view(scope.as_ref(), change, order)).await;
                if let Err(e) = saved.and_then(|written| written) {
                    send(failed(Failure::Local, &e));
                }
            }
            Action::ChangePin { scope, key, on, order } => {
                let saved = run_blocking(&backend.saves, move |saves| saves.change_pin(scope.as_ref(), &key, on, order)).await;
                if let Err(e) = saved.and_then(|written| written) {
                    send(failed(Failure::Local, &e));
                }
            }
            Action::Open(key) => {
                let _opening = backend.reviews.start_opening();
                let wanted = key.clone();
                let cached = read_cache(&backend.reviews, move |reviews| reviews.open_cached(&wanted)).await;
                let painted = cached.as_ref().and_then(painted_head);
                if let Some(cached) = cached {
                    send(cached);
                    backend.reviews.resume(&key, &send).await;
                }
                let fresh = match &painted {
                    Some(head) => backend.reviews.reopen(key.clone(), head).await,
                    None => backend.reviews.fetch_review(key.clone()).await,
                };
                let arrived = fresh.is_ok();
                send(fresh.unwrap_or_else(|e| failed(Failure::Open { key: key.clone() }, &e)));
                if arrived && painted.is_none() {
                    backend.reviews.resume(&key, &send).await;
                }
            }
            Action::Prefetch(plan) => in_turn(plan, AHEAD, |ahead| async { backend.reviews.load_ahead(ahead).await }).await,
            Action::RefreshMr(key) => {
                send(backend.reviews.fetch_review(key.clone()).await.unwrap_or_else(|e| failed(Failure::Poll { key }, &e)));
            }
            Action::PollMr { key, head } => {
                send(backend.reviews.poll_review(key.clone(), &head).await.unwrap_or_else(|e| failed(Failure::Poll { key }, &e)));
            }
            Action::RefreshDiscussions(key) => {
                send(backend.reviews.fetch_discussions(key.clone()).await.unwrap_or_else(|e| failed(Failure::Poll { key }, &e)));
            }
            Action::SaveState { key, order, fold, viewed, auto_folded, side_by_side, spot } => {
                let chosen =
                    MrState { fold, viewed_files: viewed, auto_folded: (*auto_folded).clone(), side_by_side, spot, opened_at: None };
                let save = run_blocking(&backend.saves, move |saves| saves.save_state(&key, order, chosen));
                if let Err(e) = save.await.and_then(|saved| saved) {
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
                if let Err(e) = blocking::run(move || save_theme(&name)).await {
                    send(failed(Failure::Local, &e));
                }
            }
            Action::Share { target, message, done } => {
                send(crate::share::send(&target, &message).await.map_or_else(|e| failed(Failure::Local, &e), |()| Incoming::Done(done)));
            }
            Action::RememberShare { host, target } => {
                blocking::run(move || crate::share::remember(&Cache::shared(), &host, target.as_deref()))
                    .await
                    .or_warn("remembering the share target");
            }
            Action::Copy { text, done } => send(copy(&text).await.map_or_else(|e| failed(Failure::Local, &e), |()| Incoming::Done(done))),
            Action::SaveDraft { key, draft } => send(match backend.drafts.save(key.clone(), draft).await {
                Ok(saved) => saved,
                Err(e) => failed(Failure::Draft { key }, &e),
            }),
            Action::UpdateDraft { key, id, draft, held } => send(match backend.drafts.update(&key, id, &draft).await {
                Ok(()) => Incoming::DraftChanged { key },
                Err(e) => failed(Failure::DraftUpdate { key, held, wanted: draft.body }, &e),
            }),
            Action::DeleteDraft { key, id, held } => send(match backend.drafts.delete(&key, id).await {
                Ok(()) => Incoming::DraftChanged { key },
                Err(e) => failed(Failure::DraftDelete { key, held }, &e),
            }),
            Action::Publish { key, approve, count } => {
                send(backend.drafts.publish(key, approve, count).await.unwrap_or_else(|e| failed(Failure::Publish, &e)));
            }
            Action::Post { key, to, body } => {
                send(match post(backend.homes.forge_of(&key), &key, &to, &body).await {
                    Ok(()) => Incoming::Posted { key, to },
                    Err(e) => failed(Failure::Post { key, to }, &e),
                });
            }
            Action::Resolve { key, thread, resolved } => {
                send(backend.homes.forge_of(&key).resolve(&key, &thread, resolved).await.map_or_else(
                    |e| failed(Failure::Resolve { thread: thread.clone(), resolved }, &e),
                    |()| Incoming::Resolved { key, thread: thread.clone(), resolved },
                ));
            }
            Action::LoadFile { key, path, sha } => {
                let outcome = backend.homes.forge_of(&key).file(&key, &path, &sha).await;
                send(outcome.map_or_else(|e| failed(Failure::Local, &e), |text| Incoming::File { key, path, sha, text }));
            }
            Action::React { key, thread, index, note, emoji, on } => {
                if let Err(e) = backend.homes.forge_of(&key).react(&note, emoji, on).await {
                    send(failed(app::react_failure(thread, index, emoji, on), &e));
                }
            }
            Action::CountReaction { host, emoji } => {
                blocking::run(move || crate::reactions::record(&Cache::shared(), &host, emoji)).await.or_warn("counting the reaction");
            }
            Action::Merge { key, head, plan } => {
                let outcome = backend.homes.forge_of(&key).merge(&key, &head, plan).await;
                send(outcome.map_or_else(|e| failed(Failure::Merge, &e), |()| Incoming::Merged { key }));
            }
            Action::SetDraft { key, draft } => {
                let outcome = backend.homes.forge_of(&key).set_draft(&key, draft).await;
                send(outcome.map_or_else(|e| failed(Failure::SetDraft, &e), |()| Incoming::DraftSet { key, draft }));
            }
            Action::Apply { key, branch, suggestion } => {
                let outcome = backend.homes.forge_of(&key).apply(&key, &branch, &suggestion).await;
                send(outcome.map_or_else(|e| failed(Failure::Apply, &e), |()| Incoming::Applied { key, branch }));
            }
            Action::LoadImage { key, url } => {
                let image = backend.files.image(&key, &url).await;
                send(Incoming::Image { url, image });
            }
            Action::LoadChecks { key, head } => {
                let outcome = backend.homes.forge_of(&key).checks(&key, &head).await;
                send(outcome.map_or_else(|e| failed(Failure::Checks, &e), |checks| Incoming::Checks { key, checks }));
            }
            Action::LoadOutline { key, base, head, files } => {
                let outcome = backend.files.outline(&key, &base, &head, files).await;
                send(outcome.map_or_else(|e| failed(Failure::Outline, &e), |reading| Incoming::Outline { key, reading }));
            }
            Action::LoadProse { key, base, head, sides } => {
                let outcome = backend.files.prose(&key, &base, &head, sides.clone()).await;
                send(outcome.map_or_else(|e| failed(Failure::Prose, &e), |versions| Incoming::Prose { key, sides, versions }));
            }
            Action::LoadDeployments { key, branch, head } => {
                if let Ok(deployments) = backend.homes.forge_of(&key).deployments(&key, &branch, &head).await {
                    send(Incoming::Deployments { key, deployments });
                }
            }
            Action::Approve { key, approve } => {
                let outcome = backend.homes.forge_of(&key).approve(&key, approve).await;
                send(outcome.map_or_else(|e| failed(Failure::Approve, &e), |()| Incoming::Approved { key, approve }));
            }
            Action::View { key, path, sha, line, note } => {
                let outcome = backend.files.view(key, &path, &sha, line, note).await;
                send(outcome.unwrap_or_else(|e| Incoming::Failed { what: Failure::Local, message: format!("{e:#}") }));
            }
            Action::Ask { key, id, request, fresh, label, head } => backend.ai.ask(&key, id, &request, fresh, &label, &head, &send).await,
            Action::LoadAnswers(key) => {
                let wanted = key.clone();
                let answers = read_cache(&backend.ai, move |ai| ai.past_answers(&wanted)).await;
                send(Incoming::PastAnswers { key, answers });
            }
            Action::Triage(mr) => {
                send(backend.ai.triage(&mr).await.unwrap_or_else(|e| Incoming::Failed { what: Failure::Triage, message: e.notice() }));
            }
            Action::Read { key, head, waits, files } => {
                send(
                    backend
                        .ai
                        .read(key, head, waits, files)
                        .await
                        .unwrap_or_else(|e| Incoming::Failed { what: Failure::Triage, message: e.notice() }),
                );
            }
            Action::Session { mr, choice } => {
                send(
                    backend
                        .session(*mr, choice)
                        .await
                        .unwrap_or_else(|e| Incoming::Failed { what: Failure::Local, message: format!("{e:#}") }),
                );
            }
            Action::Compose { .. } => unreachable!("the loop runs the editor itself"),
        }
    });
}

/// `:set theme=`: the one setting the TUI writes, read back on the next start.
fn save_theme(name: &str) -> Result<()> {
    let mut config = crate::config::Config::load()?;
    config.tui.theme = Some(name.to_owned());
    config.save()
}

/// The head commit of a review painted from the cache.
fn painted_head(cached: &Incoming) -> Option<crate::forge::Sha> {
    match cached {
        Incoming::Review { review, .. } => Some(review.mr.refs.head.clone()),
        _ => None,
    }
}

/// Runs `load` over `plan`, at most `cap` at a time.
async fn in_turn<T, F, Fut>(plan: Vec<T>, cap: usize, load: F)
where
    F: Fn(T) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    futures_util::stream::iter(plan).for_each_concurrent(cap, load).await;
}

async fn post(forge: &Forge, key: &MrKey, to: &Post, body: &str) -> Result<()> {
    match to {
        Post::Thread(position) => forge.comment(key, body, Some(position)).await.map(|_| ()),
        Post::Reply(thread) => forge.reply(key, thread, body).await,
    }
}

fn log_failure(incoming: &Incoming) {
    match incoming {
        Incoming::Failed { what, message } => tracing::warn!("{what:?} failed: {message}"),
        Incoming::Answer { part: Part::Failed(message), .. } => tracing::warn!("Claude failed: {message}"),
        _ => {}
    }
}

pub(super) fn failed(what: Failure, err: &anyhow::Error) -> Incoming {
    Incoming::Failed { what, message: err.to_string() }
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
    use super::*;
    use std::time::Duration;

    /// Never polled, so nothing runs: the types alone prove the helpers wait without blocking a runtime thread.
    #[test]
    fn the_macos_helpers_are_futures_so_they_never_block_the_runtime() {
        fn future<F: std::future::Future<Output = Result<()>>>(_: F) {}
        future(notify("title", "body"));
        future(open_url("https://gitlab.com"));
        future(copy("text"));
    }

    #[tokio::test(start_paused = true)]
    async fn in_turn_runs_at_most_the_cap_at_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let (running, most) = (AtomicUsize::new(0), AtomicUsize::new(0));
        in_turn((0..6).collect(), 2, |_: u32| async {
            let now = running.fetch_add(1, Ordering::SeqCst) + 1;
            most.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            running.fetch_sub(1, Ordering::SeqCst);
        })
        .await;
        assert_eq!(most.load(Ordering::SeqCst), 2);
    }
}
