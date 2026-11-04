//! Claude's answers and Jev's verdicts and readings, each kept in the cache once given.
use super::{Homes, read_cache, write_cache};
use crate::ai::anthropic::{self, Ask, Claude};
use crate::ai::triage::{self, Verdict};
use crate::ai::typesafe::{TypeSafe, Unavailable};
use crate::cache::keys;
use crate::forge::{MrKey, Sha};
use crate::tui::app::{Incoming, Part};
use chrono::Utc;
use std::sync::Arc;

#[derive(Clone)]
pub(in crate::tui) struct Ai {
    pub(super) homes: Arc<Homes>,
    /// Jev, when `[ai.typesafe]` is on and a key was found.
    pub(super) jev: Option<TypeSafe>,
    /// Claude, when `[ai.anthropic]` is on and a key was found.
    pub(super) claude: Option<Claude>,
}

impl Ai {
    /// Claude's answer, piece by piece as it streams.
    pub(in crate::tui) async fn ask(&self, key: &MrKey, id: u64, request: &Ask, send: &impl Fn(Incoming)) {
        let part = |part: Part| Incoming::Answer { key: key.clone(), id, part };
        let Some(claude) = &self.claude else {
            send(part(Part::Failed("Claude is off".into())));
            return;
        };
        let streamed = claude
            .stream(request, |event| match event {
                anthropic::Event::Text(more) => send(part(Part::Text(more))),
                anthropic::Event::Restart => send(part(Part::Restart)),
            })
            .await;
        match streamed {
            Ok(outcome) => send(part(Part::Done(outcome))),
            Err(failure) => send(part(Part::Failed(failure.0))),
        }
    }

    /// Jev's verdict on a queue MR, from the cache while the MR has not moved.
    pub(in crate::tui) async fn triage(&self, mr: &crate::forge::QueueMr) -> Result<Incoming, Unavailable> {
        let key = mr.key();
        let wanted = key.clone();
        let cached: Option<Verdict> = read_cache(&self.homes, move |homes| homes.cache_of(&wanted).read(&keys::verdict(&wanted))).await;
        let verdict = if let Some(verdict) = cached.filter(|v| v.fresh_for(mr)) {
            verdict
        } else {
            let jev = self.jev.as_ref().ok_or_else(|| Unavailable("switched off".into()))?;
            let verdict = triage::judge_mr(jev, mr, Utc::now()).await?;
            let (wanted, saved) = (key.clone(), verdict.clone());
            write_cache("caching the verdict", &self.homes, move |homes| homes.cache_of(&wanted).write(&keys::verdict(&wanted), &saved))
                .await;
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
        let cached = read_cache(&self.homes, move |homes| homes.cache_of(&wanted).read(&keys::reading(&wanted, &at))).await;
        let reading = if let Some(reading) = cached {
            reading
        } else {
            let jev = self.jev.as_ref().ok_or_else(|| Unavailable("switched off".into()))?;
            let reading = triage::judge_open(jev, waits, files).await?;
            let (wanted, at, saved) = (key.clone(), head.clone(), reading.clone());
            write_cache("caching the reading", &self.homes, move |homes| {
                homes.cache_of(&wanted).write(&keys::reading(&wanted, &at), &saved)
            })
            .await;
            reading
        };
        Ok(Incoming::Read { key, head, reading })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::gitlab::fixture::key;
    use crate::tui::backend::test_support::{homes_in, queue};
    use std::collections::BTreeMap;

    fn ai_in(dir: &std::path::Path) -> Ai {
        Ai { homes: homes_in(dir).into(), jev: None, claude: None }
    }

    #[tokio::test]
    async fn jev_is_asked_once_per_mr_state_and_answers_from_the_cache_after() {
        let dir = tempfile::tempdir().unwrap();
        let ai = ai_in(dir.path());
        let mr = queue().review_requested[0].clone();
        let verdict = Verdict { urgency: 2.8, size: triage::Size::Large, seen: mr.updated_at };
        ai.homes.cache.write(&keys::verdict(&mr.key()), &verdict).unwrap();
        let Ok(Incoming::Triaged { verdict: cached, .. }) = ai.triage(&mr).await else { panic!("the cache answers") };
        assert_eq!(cached, verdict);
        let moved = crate::forge::QueueMr { updated_at: mr.updated_at + chrono::TimeDelta::minutes(5), ..mr };
        assert!(ai.triage(&moved).await.is_err(), "a moved MR needs Jev, which is off here");
        let reading = triage::Reading { waits_on_me: true, risks: BTreeMap::new() };
        ai.homes.cache.write(&keys::reading(&key(), &"abc".into()), &reading).unwrap();
        let Ok(Incoming::Read { reading: read, .. }) = ai.read(key(), "abc".into(), None, vec![]).await else { panic!("cached") };
        assert!(read.waits_on_me);
    }

    #[tokio::test]
    async fn an_answer_streams_in_pieces_then_ends() {
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
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let claude = Claude::with_base(&server.uri(), crate::ai::Secret::new("sk-ant-test"), "claude-opus-5").unwrap();
        let ai = Ai { claude: Some(claude), ..ai_in(dir.path()) };
        let request = Ask { system: vec![], turns: vec![anthropic::Turn { role: anthropic::Role::User, text: "ok?".into() }] };
        let heard = std::sync::Mutex::new(vec![]);
        let listen = |incoming: Incoming| heard.lock().unwrap().push(incoming);
        ai.ask(&key(), 1, &request, &listen).await;
        let heard = heard.into_inner().unwrap();
        assert!(matches!(&heard[0], Incoming::Answer { part: Part::Text(text), .. } if text == "Looks fine."), "{heard:?}");
        assert!(matches!(&heard[1], Incoming::Answer { part: Part::Done(_), .. }), "{heard:?}");
    }
}
