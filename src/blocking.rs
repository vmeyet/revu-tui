//! Blocking work, such as cache file I/O, kept off the threads that drive the network.
use anyhow::{Context as _, Result};

/// Runs `work` on tokio's blocking pool; a task that panicked or was cancelled is an error.
pub(crate) async fn run<T: Send + 'static>(work: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(work).await.context("a background task stopped")?
}
