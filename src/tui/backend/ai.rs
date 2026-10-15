//! Claude's answers and Jev's verdicts and readings, each kept in the cache once given.
use super::Backend;
use crate::ai::anthropic::{self, Ask, Outcome, Stop, Usage};
use crate::ai::triage::{self, Verdict};
use crate::ai::typesafe::Unavailable;
use crate::cache::keys;
use crate::forge::{MrKey, Sha};
use crate::tui::app::{self, Incoming, Part};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// An answer kept in the cache: asking the same thing about the same diff paints it at once, and
/// `a h` lists it. Answers kept before the label was saved have none and are not listed.
#[derive(Serialize, Deserialize)]
pub(super) struct SavedAnswer {
    text: String,
    model: String,
    usage: Usage,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    asked_at: Option<DateTime<Utc>>,
    #[serde(default)]
    head: Option<Sha>,
    #[serde(default)]
    request: Option<Ask>,
}

impl SavedAnswer {
    fn past(self) -> Option<app::PastAnswer> {
        let outcome = Outcome { stop: Stop::Done, usage: self.usage, model: self.model };
        Some(app::PastAnswer {
            label: self.label?,
            asked_at: self.asked_at?,
            head: self.head?,
            request: self.request?,
            text: self.text,
            outcome,
        })
    }
}

impl Backend {
    /// The answers kept for `key` that say what they were, newest first.
    pub(in crate::tui) fn past_answers(&self, key: &MrKey) -> Vec<app::PastAnswer> {
        let cache = self.cache_of(key);
        let (dir, prefix) = keys::answers(key);
        let mut answers: Vec<app::PastAnswer> =
            cache.keys_in(&dir, prefix).iter().filter_map(|entry| cache.read::<SavedAnswer>(entry)?.past()).collect();
        answers.sort_by_key(|a| std::cmp::Reverse(a.asked_at));
        answers
    }

    /// Claude's answer, piece by piece as it streams, or at once from the cache unless `fresh`.
    /// Only a finished answer is kept, so a cut or refused one is asked again next time.
    #[allow(clippy::too_many_arguments, reason = "the label and head only go into what is kept")]
    pub(in crate::tui) async fn ask(
        &self,
        key: &MrKey,
        id: u64,
        request: &Ask,
        fresh: bool,
        label: &str,
        head: &Sha,
        send: &impl Fn(Incoming),
    ) {
        let part = |part: Part| Incoming::Answer { key: key.clone(), id, part };
        let Some(claude) = &self.claude else {
            send(part(Part::Failed("Claude is off".into())));
            return;
        };
        let cache_key = keys::answer(key, &anthropic::body(claude.model(), request).to_string());
        let (wanted, reading) = (key.clone(), cache_key.clone());
        let saved = self.read_cache(move |b| b.cache_of(&wanted).read::<SavedAnswer>(&reading)).await;
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
                    let saved = SavedAnswer {
                        text,
                        model: outcome.model.clone(),
                        usage: outcome.usage,
                        label: Some(label.to_owned()),
                        asked_at: Some(Utc::now()),
                        head: Some(head.clone()),
                        request: Some(request.clone()),
                    };
                    let wanted = key.clone();
                    self.write_cache("caching Claude's answer", move |b| b.cache_of(&wanted).write(&cache_key, &saved)).await;
                }
                send(part(Part::Done { outcome, cached_text: None }));
            }
            Err(failure) => send(part(Part::Failed(failure.0))),
        }
    }

    /// Jev's verdict on a queue MR, from the cache while the MR has not moved.
    pub(in crate::tui) async fn triage(&self, mr: &crate::forge::QueueMr) -> Result<Incoming, Unavailable> {
        let key = mr.key();
        let wanted = key.clone();
        let cached: Option<Verdict> = self.read_cache(move |b| b.cache_of(&wanted).read(&keys::verdict(&wanted))).await;
        let verdict = if let Some(verdict) = cached.filter(|v| v.fresh_for(mr)) {
            verdict
        } else {
            let jev = self.jev.as_ref().ok_or_else(|| Unavailable("switched off".into()))?;
            let verdict = triage::judge_mr(jev, mr, Utc::now()).await?;
            let (wanted, saved) = (key.clone(), verdict.clone());
            self.write_cache("caching the verdict", move |b| b.cache_of(&wanted).write(&keys::verdict(&wanted), &saved)).await;
            verdict
        };
        Ok(Incoming::Triaged { key, verdict })
    }

    /// Jev's reading of an open MR at `head`, from the cache once read.
    pub(in crate::tui) async fn read(
        &self,
        key: MrKey,
        head: Sha,
        waits: Option<serde_json::Value>,
        files: Vec<(String, serde_json::Value)>,
    ) -> Result<Incoming, Unavailable> {
        let (wanted, at) = (key.clone(), head.clone());
        let cached = self.read_cache(move |b| b.cache_of(&wanted).read(&keys::reading(&wanted, &at))).await;
        let reading = if let Some(reading) = cached {
            reading
        } else {
            let jev = self.jev.as_ref().ok_or_else(|| Unavailable("switched off".into()))?;
            let reading = triage::judge_open(jev, waits, files).await?;
            let (wanted, at, saved) = (key.clone(), head.clone(), reading.clone());
            self.write_cache("caching the reading", move |b| b.cache_of(&wanted).write(&keys::reading(&wanted, &at), &saved)).await;
            reading
        };
        Ok(Incoming::Read { key, head, reading })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::ai::anthropic::Claude;
    use crate::cache::Cache;
    use crate::forge::gitlab::fixture::key;
    use crate::tui::backend::test_support::{backend_on_nothing, queue};
    use std::collections::BTreeMap;

    #[test]
    fn kept_answers_list_newest_first_and_skip_those_saved_without_a_label() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend { cache: Cache::in_dir(dir.path()), ..backend_on_nothing() };
        let mr = key();
        let saved = |label: Option<&str>, hours: i64| SavedAnswer {
            text: "text".into(),
            model: "claude-opus-5".into(),
            usage: Usage::default(),
            label: label.map(str::to_owned),
            asked_at: Some(Utc::now() - chrono::Duration::hours(hours)),
            head: Some("b2".into()),
            request: Some(Ask { system: vec![], turns: vec![] }),
        };
        let cache = backend.cache_of(&mr);
        cache.write(&keys::answer(&mr, "old"), &saved(Some("explain · a.rs"), 5)).unwrap();
        cache.write(&keys::answer(&mr, "new"), &saved(Some("summary"), 1)).unwrap();
        cache.write(&keys::answer(&mr, "before"), &serde_json::json!({"text": "t", "model": "m", "usage": Usage::default()})).unwrap();
        let labels: Vec<String> = backend.past_answers(&mr).into_iter().map(|a| a.label).collect();
        assert_eq!(labels, ["summary", "explain · a.rs"]);
    }

    #[tokio::test]
    async fn jev_is_asked_once_per_mr_state_and_answers_from_the_cache_after() {
        let dir = tempfile::tempdir().unwrap();
        let backend = Backend { cache: Cache::in_dir(dir.path()), ..backend_on_nothing() };
        let mr = queue().review_requested[0].clone();
        let verdict = Verdict { urgency: 2.8, size: triage::Size::Large, seen: mr.updated_at };
        backend.cache.write(&keys::verdict(&mr.key()), &verdict).unwrap();
        let Ok(Incoming::Triaged { verdict: cached, .. }) = backend.triage(&mr).await else { panic!("the cache answers") };
        assert_eq!(cached, verdict);
        let moved = crate::forge::QueueMr { updated_at: mr.updated_at + chrono::TimeDelta::minutes(5), ..mr };
        assert!(backend.triage(&moved).await.is_err(), "a moved MR needs Jev, which is off here");
        let reading = triage::Reading { waits_on_me: true, risks: BTreeMap::new() };
        backend.cache.write(&keys::reading(&key(), &"abc".into()), &reading).unwrap();
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
        let claude = Claude::with_base(&server.uri(), crate::ai::Secret::new("sk-ant-test"), "claude-opus-5").unwrap();
        let backend = Backend { cache: Cache::in_dir(dir.path()), claude: Some(claude), ..backend_on_nothing() };
        let request = Ask { system: vec![], turns: vec![anthropic::Turn { role: anthropic::Role::User, text: "ok?".into() }] };
        let heard = std::sync::Mutex::new(vec![]);
        let listen = |incoming: Incoming| heard.lock().unwrap().push(incoming);
        backend.ask(&key(), 1, &request, false, "explain", &"b2".into(), &listen).await;
        backend.ask(&key(), 2, &request, false, "explain", &"b2".into(), &listen).await;
        backend.ask(&key(), 3, &request, true, "explain", &"b2".into(), &listen).await;
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
}
