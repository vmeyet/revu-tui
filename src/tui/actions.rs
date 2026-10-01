//! The action runner: each action the app asks for runs in its own task and answers through `Incoming`.
use super::app::{self, Action, Failure, Incoming, Part, Post, QueueView};
use super::backend::{Backend, MrState, Opening};
use super::blocking;
use crate::cache::keys;
use crate::forge::{Forge, MrKey};
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
            let _ = tx.send(incoming);
        };
        match action {
            Action::LoadQueue { scope, from_cache } => {
                if from_cache {
                    let wanted = scope.clone();
                    if let Some(view) =
                        backend.off(move |b| b.cache.read::<QueueView>(&keys::queue_view(wanted.as_deref()))).await.ok().flatten()
                    {
                        send(Incoming::QueueView { scope: scope.clone(), view });
                    }
                }
                let wanted = scope.clone();
                if let Some(cached) = backend.off(move |b| b.cached_queue(wanted)).await.ok().flatten().filter(|_| from_cache) {
                    send(cached);
                }
                match backend.load_queue(scope).await {
                    Ok((answer, ready_failure)) => {
                        let counted = match &answer {
                            Incoming::Queue { sections, .. } => Some(sections.clone()),
                            _ => None,
                        };
                        send(answer);
                        if let Some(sections) = counted
                            && let Ok(progress) = backend.off(move |b| b.progress(&sections)).await
                        {
                            send(Incoming::Progress(progress));
                        }
                        if let Some(message) = ready_failure {
                            send(Incoming::Failed { what: Failure::Ready, message });
                        }
                    }
                    Err(e) => send(failed(Failure::Queue, &e)),
                }
            }
            Action::SaveQueueView { scope, view } => {
                let saved = backend.off(move |b| b.cache.write(&keys::queue_view(scope.as_deref()), &view)).await;
                if let Err(e) = saved.and_then(|written| written) {
                    send(failed(Failure::Local, &e));
                }
            }
            Action::Open(key) => {
                let _opening = Opening::start(&backend.opening);
                let wanted = key.clone();
                let cached = backend.off(move |b| b.open_cached(&wanted)).await.ok().flatten();
                let painted = cached.is_some();
                if let Some(cached) = cached {
                    send(cached);
                    backend.resume(&key, &send).await;
                }
                let fresh = backend.fetch_review(key.clone()).await;
                let arrived = fresh.is_ok();
                send(fresh.unwrap_or_else(|e| failed(Failure::Open, &e)));
                if arrived && !painted {
                    backend.resume(&key, &send).await;
                }
            }
            Action::Prefetch(plan) => in_turn(plan, AHEAD, |ahead| async { backend.load_ahead(ahead).await }).await,
            Action::RefreshMr(key) => send(backend.fetch_review(key).await.unwrap_or_else(|e| failed(Failure::Poll, &e))),
            Action::PollMr { key, head } => send(backend.poll_review(key, &head).await.unwrap_or_else(|e| failed(Failure::Poll, &e))),
            Action::RefreshDiscussions(key) => send(backend.fetch_discussions(key).await.unwrap_or_else(|e| failed(Failure::Poll, &e))),
            Action::SaveState { key, order, fold, viewed, auto_folded, side_by_side, spot } => {
                let chosen =
                    MrState { fold, viewed_files: viewed, auto_folded: (*auto_folded).clone(), side_by_side, spot, opened_at: None };
                let save = move |b: &Backend| b.save_state(&key, order, chosen);
                if let Err(e) = backend.off(save).await.and_then(|saved| saved) {
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
            Action::Share { target, message, done } => {
                send(crate::share::send(&target, &message).await.map_or_else(|e| failed(Failure::Local, &e), |()| Incoming::Done(done)));
            }
            Action::Yank(url) => send(copy(&url).await.map_or_else(|e| failed(Failure::Local, &e), |()| Incoming::Done("copied".into()))),
            Action::Copy { text, done } => send(copy(&text).await.map_or_else(|e| failed(Failure::Local, &e), |()| Incoming::Done(done))),
            Action::SaveDraft { key, draft } => send(backend.save_draft(key, draft).await.unwrap_or_else(|e| failed(Failure::Draft, &e))),
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
            Action::Post { key, to, body } => {
                send(match post(backend.forge_of(&key), &key, &to, &body).await {
                    Ok(()) => Incoming::Posted { key, to },
                    Err(e) => failed(Failure::Post { key, to }, &e),
                });
            }
            Action::Resolve { key, thread, resolved } => send(backend.forge_of(&key).resolve(&key, &thread, resolved).await.map_or_else(
                |e| failed(Failure::Resolve { thread: thread.clone(), resolved }, &e),
                |()| Incoming::Resolved { key, thread: thread.clone(), resolved },
            )),
            Action::LoadFile { key, path, sha } => {
                let outcome = backend.forge_of(&key).file(&key, &path, &sha).await;
                send(outcome.map_or_else(|e| failed(Failure::Local, &e), |text| Incoming::File { key, path, sha, text }));
            }
            Action::React { key, thread, index, note, emoji, on } => {
                if let Err(e) = backend.forge_of(&key).react(&note, emoji, on).await {
                    send(failed(app::react_failure(thread, index, emoji, on), &e));
                }
            }
            Action::Merge { key, head, plan } => {
                let outcome = backend.forge_of(&key).merge(&key, &head, plan).await;
                send(outcome.map_or_else(|e| failed(Failure::Merge, &e), |()| Incoming::Merged { key }));
            }
            Action::SetDraft { key, draft } => {
                let outcome = backend.forge_of(&key).set_draft(&key, draft).await;
                send(outcome.map_or_else(|e| failed(Failure::SetDraft, &e), |()| Incoming::DraftSet { key, draft }));
            }
            Action::Apply { key, branch, suggestion } => {
                let outcome = backend.forge_of(&key).apply(&key, &branch, &suggestion).await;
                send(outcome.map_or_else(|e| failed(Failure::Apply, &e), |()| Incoming::Applied { key, branch }));
            }
            Action::LoadImage { key, url } => {
                let image = backend.image(&key, &url).await;
                send(Incoming::Image { url, image });
            }
            Action::LoadChecks { key, head } => {
                let outcome = backend.forge_of(&key).checks(&key, &head).await;
                send(outcome.map_or_else(|e| failed(Failure::Checks, &e), |checks| Incoming::Checks { key, checks }));
            }
            Action::LoadOutline { key, base, head, files } => {
                let outcome = backend.outline(&key, &base, &head, files).await;
                send(outcome.map_or_else(|e| failed(Failure::Outline, &e), |changes| Incoming::Outline { key, changes }));
            }
            Action::LoadDeployments { key, branch, head } => {
                if let Ok(deployments) = backend.forge_of(&key).deployments(&key, &branch, &head).await {
                    send(Incoming::Deployments { key, deployments });
                }
            }
            Action::Approve { key, approve } => {
                let outcome = backend.forge_of(&key).approve(&key, approve).await;
                send(outcome.map_or_else(|e| failed(Failure::Approve, &e), |()| Incoming::Approved { key, approve }));
            }
            Action::View { key, path, sha, line, note } => {
                let outcome = backend.view(key, &path, &sha, line, note).await;
                send(outcome.unwrap_or_else(|e| Incoming::Failed { what: Failure::Local, message: format!("{e:#}") }));
            }
            Action::Ask { key, id, request, fresh, label, head } => backend.ask(&key, id, &request, fresh, &label, &head, &send).await,
            Action::LoadAnswers(key) => {
                let wanted = key.clone();
                let answers = backend.off(move |b| b.past_answers(&wanted)).await.unwrap_or_default();
                send(Incoming::PastAnswers { key, answers });
            }
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

/// `:set theme=`: the one setting the TUI writes, read back on the next start.
fn save_theme(name: &str) -> Result<()> {
    let mut config = crate::config::Config::load()?;
    config.tui.theme = Some(name.to_owned());
    config.save()
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

    #[tokio::test]
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
