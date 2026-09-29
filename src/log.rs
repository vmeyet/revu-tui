//! The log file, `revu.log` in the cache root; never the terminal, which the TUI owns.
//! `REVU_LOG` sets the level. A request line holds its method, host, path, status and time:
//! never a header, so never a token, and never a query, which may carry a signed link's secret.
use crate::cache::Cache;
use std::time::Instant;
use tracing::level_filters::LevelFilter;

const FILE: &str = "revu.log";
const LEVEL: &str = "REVU_LOG";
const DEFAULT_LEVEL: LevelFilter = LevelFilter::WARN;
const MAX_BYTES: u64 = 2 * 1024 * 1024;
const HTTP: &str = "revu::http";

/// Logs to the file for the rest of the process; a file that cannot open leaves revu unlogged, not stopped.
pub fn start() {
    let level = level(std::env::var(LEVEL).ok().as_deref());
    if level == LevelFilter::OFF {
        return;
    }
    let Ok(file) = Cache::shared().append(FILE, MAX_BYTES) else { return };
    let _ = tracing::subscriber::set_global_default(subscriber(std::sync::Mutex::new(file), level));
}

/// Only revu's own events: the HTTP libraries below log frames and headers at their debug levels.
fn subscriber<W>(writer: W, level: LevelFilter) -> impl tracing::Subscriber + Send + Sync
where
    W: for<'w> tracing_subscriber::fmt::MakeWriter<'w> + Send + Sync + 'static,
{
    use tracing_subscriber::layer::SubscriberExt;
    let only_revu = tracing_subscriber::filter::Targets::new().with_target(env!("CARGO_CRATE_NAME"), level);
    tracing_subscriber::fmt().with_writer(writer).with_ansi(false).with_max_level(level).finish().with(only_revu)
}

/// A level `REVU_LOG` names (`error` to `trace`, or `off`); anything else keeps the default.
fn level(variable: Option<&str>) -> LevelFilter {
    variable.and_then(|v| v.trim().parse().ok()).unwrap_or(DEFAULT_LEVEL)
}

/// Sends `request` like `RequestBuilder::send`, and logs its line once it is answered or not.
pub(crate) async fn send(request: reqwest::RequestBuilder) -> reqwest::Result<reqwest::Response> {
    let (client, request) = request.build_split();
    let request = request?;
    let line = Line::start(request.method(), request.url());
    let sent = client.execute(request).await;
    line.write(sent.as_ref().ok().map(|r| r.status().as_u16()));
    sent
}

/// One request on the clock.
struct Line {
    method: String,
    target: String,
    started: Instant,
}

impl Line {
    fn start(method: &reqwest::Method, url: &url::Url) -> Self {
        Self { method: method.to_string(), target: format!("{}{}", url.host_str().unwrap_or("?"), url.path()), started: Instant::now() }
    }

    /// `status` is `None` when no answer came back at all.
    fn write(self, status: Option<u16>) {
        let Self { method, target, started } = self;
        let ms = started.elapsed().as_millis();
        match status {
            Some(status) if status < 400 => tracing::info!(target: HTTP, "{method} {target} {status} {ms}ms"),
            Some(status) => tracing::warn!(target: HTTP, "{method} {target} {status} {ms}ms"),
            None => tracing::warn!(target: HTTP, "{method} {target} no answer {ms}ms"),
        }
    }
}

/// Reading back what the running test logged. One global subscriber writes each thread's lines
/// to that thread's buffer: scoped subscribers race over tracing's shared callsite cache.
#[cfg(test)]
pub(crate) mod capture {
    use std::cell::RefCell;
    use tracing::level_filters::LevelFilter;

    thread_local! {
        static LOGGED: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    }

    /// Forgets what this thread logged so far.
    pub fn start() {
        static GLOBAL: std::sync::Once = std::sync::Once::new();
        GLOBAL.call_once(|| {
            let _ = tracing::subscriber::set_global_default(super::subscriber(|| ThisThread, LevelFilter::TRACE));
        });
        LOGGED.with_borrow_mut(Vec::clear);
    }

    /// What this thread logged since [`start`].
    pub fn text() -> String {
        LOGGED.with_borrow(|bytes| String::from_utf8_lossy(bytes).into_owned())
    }

    struct ThisThread;

    impl std::io::Write for ThisThread {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            LOGGED.with_borrow_mut(|logged| logged.extend_from_slice(bytes));
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn the_level_comes_from_the_variable_else_warn() {
        assert_eq!(level(Some("debug")), LevelFilter::DEBUG);
        assert_eq!(level(Some(" off ")), LevelFilter::OFF);
        assert_eq!(level(Some("loud")), LevelFilter::WARN);
        assert_eq!(level(None), LevelFilter::WARN);
    }

    #[test]
    fn a_request_line_names_method_host_path_and_status_without_the_query() {
        capture::start();
        let url = |raw: &str| url::Url::parse(raw).unwrap();
        Line::start(&reqwest::Method::GET, &url("https://gitlab.com/api/v4/user?private_token=glpat-xxxx")).write(Some(200));
        Line::start(&reqwest::Method::POST, &url("https://api.anthropic.com/v1/messages")).write(None);
        let text = capture::text();
        assert!(text.contains("INFO") && text.contains("GET gitlab.com/api/v4/user 200 "), "{text}");
        assert!(text.contains("WARN") && text.contains("POST api.anthropic.com/v1/messages no answer "), "{text}");
        assert!(!text.contains("glpat") && !text.contains('?'), "{text}");
    }
}
