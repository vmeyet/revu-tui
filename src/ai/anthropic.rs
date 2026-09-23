//! Claude over raw HTTPS: one streamed Messages API call, with prompt caching on the stable blocks
//! and server-side fallbacks, so a refused question is answered by another model when one can.
use super::Secret;
use serde::Serialize;
use serde_json::{Value, json};
use std::fmt;
use std::time::Duration;

const API_URL: &str = "https://api.anthropic.com";
const VERSION: &str = "2023-06-01";
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
const MAX_TOKENS: u32 = 16_000;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const RETRYABLE: [u16; 5] = [429, 500, 502, 503, 529];
const LONGEST_WAIT: Duration = Duration::from_secs(30);

/// One block of the system prompt; `cached` ones end a cache breakpoint, so the stable parts are paid once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub text: String,
    pub cached: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
}

/// One turn of the conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turn {
    pub role: Role,
    pub text: String,
}

/// Everything one call sends: the system blocks, then the turns so far, the question last.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    pub system: Vec<Block>,
    pub turns: Vec<Turn>,
}

/// What the stream says as it goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Text(String),
    /// A refused model's partial answer is void: another model starts over.
    Restart,
}

/// Why the answer stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    Done,
    /// Cut at the token limit: the answer is partial.
    Cut,
    /// Every model declined; the explanation, when the API gave one.
    Refused(Option<String>),
}

/// Tokens of one call, the cache reads included, so the pane can show what caching saved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub stop: Stop,
    pub usage: Usage,
    /// The model that answered: the configured one, or its fallback.
    pub model: String,
}

/// Why a call failed, in words for a toast; never carries the key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure(pub String);

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone)]
pub struct Claude {
    http: reqwest::Client,
    base: String,
    key: Secret,
    model: String,
}

impl fmt::Debug for Claude {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Claude").field("model", &self.model).field("key", &"<redacted>").finish_non_exhaustive()
    }
}

impl Claude {
    pub fn new(key: Secret, model: &str) -> Self {
        Self::with_base(API_URL, key, model)
    }

    fn with_base(base: &str, key: Secret, model: &str) -> Self {
        let http = reqwest::Client::builder().connect_timeout(CONNECT_TIMEOUT).build().unwrap_or_default();
        Self { http, base: base.trim_end_matches('/').to_owned(), key, model: model.to_owned() }
    }

    /// Streams the answer to `on`, text as it comes, and says how it ended.
    /// A busy API (429, 5xx, 529) is tried once more after the wait it asks for.
    pub async fn stream(&self, ask: &Ask, mut on: impl FnMut(Event)) -> Result<Outcome, Failure> {
        let body = body(&self.model, ask);
        let mut response = self.send(&body).await?;
        if RETRYABLE.contains(&response.status().as_u16()) {
            tokio::time::sleep(wait(&response)).await;
            response = self.send(&body).await?;
        }
        if !response.status().is_success() {
            return Err(refusal_of(response).await);
        }
        let mut sse = Sse::default();
        let mut reading = Reading::new(&self.model);
        while let Some(chunk) = response.chunk().await.map_err(|e| Failure(format!("the answer broke off: {}", e.without_url())))? {
            for event in sse.feed(&chunk) {
                if let Some(said) = reading.take(&event)? {
                    on(said);
                }
            }
        }
        reading.finish()
    }

    async fn send(&self, body: &Value) -> Result<reqwest::Response, Failure> {
        let mut request = self
            .http
            .post(format!("{}/v1/messages", self.base))
            .header("x-api-key", self.key.expose())
            .header("anthropic-version", VERSION)
            .json(body);
        if falls_back(&self.model) {
            request = request.header("anthropic-beta", FALLBACK_BETA);
        }
        request.send().await.map_err(|e| Failure(format!("could not reach Anthropic: {}", e.without_url())))
    }
}

/// The request body: the system blocks with a cache breakpoint on each stable one, then the turns.
pub fn body(model: &str, ask: &Ask) -> Value {
    let system: Vec<Value> = ask
        .system
        .iter()
        .map(|b| {
            if b.cached {
                json!({"type": "text", "text": b.text, "cache_control": {"type": "ephemeral"}})
            } else {
                json!({"type": "text", "text": b.text})
            }
        })
        .collect();
    let messages: Vec<Value> = ask.turns.iter().map(|t| json!({"role": t.role, "content": t.text})).collect();
    let mut body = json!({"model": model, "max_tokens": MAX_TOKENS, "stream": true, "system": system, "messages": messages});
    if falls_back(model) {
        body["fallbacks"] = json!("default");
    }
    body
}

/// Server-side fallbacks exist for the Opus 5 and Fable 5 families; other models stop on a refusal.
fn falls_back(model: &str) -> bool {
    model.starts_with("claude-opus-5") || model.starts_with("claude-fable-5")
}

fn wait(response: &reqwest::Response) -> Duration {
    let seconds = response.headers().get("retry-after").and_then(|v| v.to_str().ok()?.trim().parse::<u64>().ok()).unwrap_or(1);
    Duration::from_secs(seconds).min(LONGEST_WAIT)
}

async fn refusal_of(response: reqwest::Response) -> Failure {
    let status = response.status().as_u16();
    if status == 401 || status == 403 {
        return Failure(format!("Anthropic refused the key (HTTP {status}): run `revu ai login anthropic`"));
    }
    let message = response.json::<Value>().await.ok().and_then(|v| v["error"]["message"].as_str().map(str::to_owned));
    Failure(format!("Anthropic answered HTTP {status}: {}", message.unwrap_or_else(|| "no message".into())))
}

/// Server-sent events, cut into whole events whatever the chunk boundaries: bytes wait until a blank line.
#[derive(Default)]
struct Sse {
    pending: Vec<u8>,
}

impl Sse {
    fn feed(&mut self, chunk: &[u8]) -> Vec<Value> {
        self.pending.extend_from_slice(chunk);
        let mut events = vec![];
        while let Some(end) = find(&self.pending, b"\n\n") {
            let raw: Vec<u8> = self.pending.drain(..end + 2).collect();
            let text = String::from_utf8_lossy(&raw);
            let data: String = text.lines().filter_map(|l| l.strip_prefix("data:")).map(str::trim_start).collect();
            if let Ok(event) = serde_json::from_str(&data) {
                events.push(event);
            }
        }
        events
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// What the stream said so far: the model answering, the stop reason, the tokens.
struct Reading {
    model: String,
    stop: Option<Stop>,
    usage: Usage,
}

impl Reading {
    fn new(model: &str) -> Self {
        Self { model: model.to_owned(), stop: None, usage: Usage::default() }
    }

    fn take(&mut self, event: &Value) -> Result<Option<Event>, Failure> {
        match event["type"].as_str().unwrap_or_default() {
            "message_start" => {
                let message = &event["message"];
                if let Some(model) = message["model"].as_str() {
                    model.clone_into(&mut self.model);
                }
                self.count(&message["usage"]);
                Ok(None)
            }
            "content_block_start" if event["content_block"]["type"] == "fallback" => Ok(Some(Event::Restart)),
            "content_block_delta" if event["delta"]["type"] == "text_delta" => {
                Ok(event["delta"]["text"].as_str().map(|t| Event::Text(t.to_owned())))
            }
            "message_delta" => {
                self.count(&event["usage"]);
                self.stop = event["delta"]["stop_reason"].as_str().map(|reason| stop(reason, &event["delta"]["stop_details"]));
                Ok(None)
            }
            "error" => Err(Failure(format!("Anthropic stopped: {}", event["error"]["message"].as_str().unwrap_or("unknown error")))),
            _ => Ok(None),
        }
    }

    /// Usage arrives in pieces: input and cache counts at the start, output at the end; later numbers win.
    fn count(&mut self, usage: &Value) {
        let field = |name: &str| usage[name].as_u64();
        self.usage = Usage {
            input: field("input_tokens").unwrap_or(self.usage.input),
            output: field("output_tokens").unwrap_or(self.usage.output),
            cache_read: field("cache_read_input_tokens").unwrap_or(self.usage.cache_read),
            cache_write: field("cache_creation_input_tokens").unwrap_or(self.usage.cache_write),
        };
    }

    fn finish(self) -> Result<Outcome, Failure> {
        let stop = self.stop.ok_or_else(|| Failure("the answer ended without a stop reason".into()))?;
        Ok(Outcome { stop, usage: self.usage, model: self.model })
    }
}

fn stop(reason: &str, details: &Value) -> Stop {
    match reason {
        "refusal" => Stop::Refused(details["explanation"].as_str().map(str::to_owned)),
        "max_tokens" => Stop::Cut,
        _ => Stop::Done,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sse(events: &[Value]) -> String {
        events.iter().map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap())).collect()
    }

    fn answer(text: &[&str], stop: &str) -> String {
        let mut events = vec![
            json!({"type": "message_start", "message": {"model": "claude-opus-5", "usage": {"input_tokens": 40, "cache_read_input_tokens": 1200, "cache_creation_input_tokens": 0}}}),
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
            json!({"type": "ping"}),
        ];
        events.extend(text.iter().map(|t| json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": t}})));
        events.push(json!({"type": "content_block_stop", "index": 0}));
        events.push(json!({"type": "message_delta", "delta": {"stop_reason": stop}, "usage": {"output_tokens": 9}}));
        events.push(json!({"type": "message_stop"}));
        sse(&events)
    }

    fn ask() -> Ask {
        Ask {
            system: vec![
                Block { text: "You help a reviewer.".into(), cached: false },
                Block { text: "MR acme/widgets!42".into(), cached: true },
            ],
            turns: vec![Turn { role: Role::User, text: "Explain.".into() }],
        }
    }

    fn claude(server: &MockServer) -> Claude {
        Claude::with_base(&server.uri(), Secret::new("sk-ant-test"), "claude-opus-5")
    }

    #[test]
    fn events_come_out_whole_whatever_the_chunks() {
        let stream = answer(&["Hé", "llo"], "end_turn");
        let bytes = stream.as_bytes();
        for cut in [1, 7, 33, bytes.len() / 2, bytes.len() - 1] {
            let mut sse = Sse::default();
            let mut events = sse.feed(&bytes[..cut]);
            events.extend(sse.feed(&bytes[cut..]));
            assert_eq!(events.len(), 8, "cut at {cut}");
        }
    }

    #[test]
    fn the_body_caches_the_stable_blocks_and_asks_for_fallbacks_on_opus_5() {
        let body = body("claude-opus-5", &ask());
        assert_eq!(body["system"][0].get("cache_control"), None);
        assert_eq!(body["system"][1]["cache_control"], json!({"type": "ephemeral"}));
        assert_eq!(body["messages"], json!([{"role": "user", "content": "Explain."}]));
        assert_eq!((body["stream"].as_bool(), body["fallbacks"].as_str()), (Some(true), Some("default")));
        assert!(body.get("thinking").is_none(), "Opus 5 thinks adaptively when the field is left out");
        assert!(super::body("claude-sonnet-5", &ask()).get("fallbacks").is_none());
    }

    #[tokio::test]
    async fn an_answer_streams_text_and_reports_cache_reads() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", "sk-ant-test"))
            .and(header("anthropic-beta", FALLBACK_BETA))
            .and(body_partial_json(json!({"model": "claude-opus-5", "stream": true})))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(answer(&["Hel", "lo"], "end_turn")),
            )
            .mount(&server)
            .await;
        let mut text = String::new();
        let outcome = claude(&server)
            .stream(&ask(), |e| {
                if let Event::Text(t) = e {
                    text.push_str(&t);
                }
            })
            .await
            .unwrap();
        assert_eq!(text, "Hello");
        assert_eq!(outcome.stop, Stop::Done);
        assert_eq!(outcome.usage, Usage { input: 40, output: 9, cache_read: 1200, cache_write: 0 });
    }

    #[tokio::test]
    async fn a_refusal_ends_with_its_explanation_and_a_fallback_restarts_the_text() {
        let server = MockServer::start().await;
        let refused = sse(&[
            json!({"type": "message_start", "message": {"model": "claude-opus-5", "usage": {"input_tokens": 0}}}),
            json!({"type": "message_delta", "delta": {"stop_reason": "refusal", "stop_details": {"type": "refusal", "category": "cyber", "explanation": "declined"}}, "usage": {"output_tokens": 0}}),
            json!({"type": "message_stop"}),
        ]);
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_string(refused)).mount(&server).await;
        let outcome = claude(&server).stream(&ask(), |_| {}).await.unwrap();
        assert_eq!(outcome.stop, Stop::Refused(Some("declined".into())));
        let fell_back = sse(&[
            json!({"type": "message_start", "message": {"model": "claude-opus-5", "usage": {"input_tokens": 5}}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "partial"}}),
            json!({"type": "content_block_start", "index": 1, "content_block": {"type": "fallback"}}),
            json!({"type": "content_block_delta", "index": 2, "delta": {"type": "text_delta", "text": "whole"}}),
            json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 3}}),
        ]);
        server.reset().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_string(fell_back)).mount(&server).await;
        let mut events = vec![];
        let outcome = claude(&server).stream(&ask(), |e| events.push(e)).await.unwrap();
        assert_eq!(events, vec![Event::Text("partial".into()), Event::Restart, Event::Text("whole".into())]);
        assert_eq!(outcome.stop, Stop::Done);
    }

    #[tokio::test]
    async fn a_busy_api_is_tried_once_more_and_a_bad_key_never_shows() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(529).insert_header("retry-after", "0"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(answer(&["ok"], "end_turn")))
            .mount(&server)
            .await;
        assert_eq!(claude(&server).stream(&ask(), |_| {}).await.unwrap().stop, Stop::Done);
        server.reset().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(401)).mount(&server).await;
        let err = claude(&server).stream(&ask(), |_| {}).await.unwrap_err().to_string();
        assert!(err.contains("revu ai login anthropic") && !err.contains("sk-ant-test"), "{err}");
        assert!(!format!("{:?}", claude(&server)).contains("sk-ant-test"));
    }

    #[tokio::test]
    async fn a_mid_stream_error_and_a_cut_answer_are_told_apart() {
        let server = MockServer::start().await;
        let broken = sse(&[json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}})]);
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_string(broken)).up_to_n_times(1).mount(&server).await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(answer(&["long"], "max_tokens")))
            .mount(&server)
            .await;
        assert!(claude(&server).stream(&ask(), |_| {}).await.unwrap_err().to_string().contains("Overloaded"));
        assert_eq!(claude(&server).stream(&ask(), |_| {}).await.unwrap().stop, Stop::Cut);
    }
}
