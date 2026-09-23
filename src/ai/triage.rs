//! Jev's triage: typed questions about each queue MR and each file of the open one.
//! Everything here is pure: the state Jev sees, the questions, and how answers become marks.
use super::typesafe::{Answers, Judge, Question, Unavailable};
use crate::forge::{MrKey, QueueMr};
use crate::review::Review;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const DESCRIPTION_CHARS: usize = 2000;
const FILE_CHARS: usize = 1500;
const LAST_NOTES: usize = 3;
/// The urgency level from which a queue row wears `!`: halfway into "blocking someone".
const URGENT_FROM: f64 = 2.5;
const WAITS_FROM: f64 = 0.6;

const URGENCY: (&str, Question) = (
    "urgency",
    Question::Score(
        "How soon does this merge request need its review, judging by its title, description, age, pipeline, labels and open threads?",
        &["can wait", "this week", "today", "blocking someone"],
    ),
);

const SIZE: (&str, Question) = (
    "size",
    Question::Choice(
        "How big is this merge request for a reviewer, judging by its files and line counts?",
        &[
            ("trivial", "a few lines, one obvious idea"),
            ("focused", "one idea across a few files"),
            ("large", "one idea across many files, or several small ones"),
            ("sprawling", "many files and ideas; hard to review in one sitting"),
        ],
    ),
);

const WAITS_ON_ME: (&str, Question) = (
    "waits_on_me",
    Question::Noul("Is the latest note a question or a change request addressed to the reviewer, and still unanswered by them?"),
);

const RISK: (&str, Question) = (
    "risk",
    Question::Score(
        "How risky is this file's change for production, judging by its path and its changed lines?",
        &["cosmetic", "logic", "data or schema", "security or auth"],
    ),
);

/// How big an MR reads to a reviewer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Size {
    Trivial,
    Focused,
    Large,
    Sprawling,
}

/// What Jev said about one queue MR, for the `updated_at` it saw.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    /// The probability-weighted level, 0 "can wait" to 3 "blocking someone".
    pub urgency: f64,
    pub size: Size,
    pub seen: DateTime<Utc>,
}

impl Verdict {
    pub fn urgent(&self) -> bool {
        self.urgency >= URGENT_FROM
    }

    /// Still true for `mr`: it has not moved since Jev looked.
    pub fn fresh_for(&self, mr: &QueueMr) -> bool {
        self.seen == mr.updated_at
    }
}

/// How much a file's change could hurt, lowest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    Cosmetic,
    Logic,
    Data,
    Security,
}

impl Risk {
    fn from_level(level: f64) -> Self {
        match level.round() {
            l if l <= 0.0 => Risk::Cosmetic,
            l if l <= 1.0 => Risk::Logic,
            l if l <= 2.0 => Risk::Data,
            _ => Risk::Security,
        }
    }
}

/// What Jev said about an open MR at one head commit: whether it waits on me, and each file's risk.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Reading {
    pub waits_on_me: bool,
    pub risks: BTreeMap<String, Risk>,
}

pub fn queue_questions() -> [(&'static str, Question); 2] {
    [URGENCY, SIZE]
}

pub fn waits_questions() -> [(&'static str, Question); 1] {
    [WAITS_ON_ME]
}

pub fn risk_questions() -> [(&'static str, Question); 1] {
    [RISK]
}

/// The state Jev reads for one queue MR; `now` dates it, so ages stay honest.
pub fn queue_state(mr: &QueueMr, now: DateTime<Utc>) -> Value {
    json!({
        "title": mr.title,
        "description": cut(&mr.description, DESCRIPTION_CHARS),
        "draft": mr.draft,
        "age_days": (now - mr.created_at).num_days(),
        "idle_days": (now - mr.updated_at).num_days(),
        "pipeline": mr.pipeline,
        "conflicts": mr.conflicts,
        "labels": mr.labels,
        "unresolved_threads": mr.unresolved,
        "files": mr.files,
        "additions": mr.additions,
        "deletions": mr.deletions,
    })
}

pub fn verdict(answers: &Answers, mr: &QueueMr) -> Result<Verdict, Unavailable> {
    let size = match answers.choice(SIZE.0)? {
        "trivial" => Size::Trivial,
        "focused" => Size::Focused,
        "large" => Size::Large,
        "sprawling" => Size::Sprawling,
        other => return Err(Unavailable(format!("unknown size `{other}`"))),
    };
    Ok(Verdict { urgency: answers.score(URGENCY.0)?, size, seen: mr.updated_at })
}

/// The last notes of the MR, oldest first, each saying whether I wrote it.
pub fn waits_state(review: &Review, me: &str) -> Option<Value> {
    let mut notes: Vec<_> = review.threads.iter().flat_map(|t| t.notes.iter()).filter(|n| !n.system).collect();
    notes.sort_by_key(|n| n.created_at);
    let last: Vec<Value> = notes
        .iter()
        .rev()
        .take(LAST_NOTES)
        .rev()
        .map(|n| json!({"by_reviewer": n.author.username == me, "author": n.author.username, "text": cut(&n.body, 600)}))
        .collect();
    (!last.is_empty()).then(|| json!({"reviewer": me, "mr_author": review.mr.author.username, "last_notes": last}))
}

/// One state per text file: its path and the start of its changed lines.
pub fn file_states(review: &Review) -> Vec<(String, Value)> {
    review
        .files
        .iter()
        .filter(|f| !f.binary && !f.hunks.is_empty())
        .map(|f| {
            let lines: String = f.hunks.iter().flat_map(|h| &h.lines).map(|l| format!("{}{}\n", sign(l.kind), l.text)).collect();
            (f.new_path.clone(), json!({"path": f.new_path, "change": cut(&lines, FILE_CHARS)}))
        })
        .collect()
}

pub fn waits(answers: &Answers) -> Result<bool, Unavailable> {
    Ok(answers.noul(WAITS_ON_ME.0)? >= WAITS_FROM)
}

pub fn risk(answers: &Answers) -> Result<Risk, Unavailable> {
    Ok(Risk::from_level(answers.score(RISK.0)?))
}

/// Whether a queue MR needs Jev again: never asked, or moved since.
pub fn stale(verdicts: &std::collections::HashMap<MrKey, Verdict>, mr: &QueueMr) -> bool {
    verdicts.get(&mr.key()).is_none_or(|v| !v.fresh_for(mr))
}

/// Asks Jev about one queue MR.
pub async fn judge_mr(judge: &impl Judge, mr: &QueueMr, now: DateTime<Utc>) -> Result<Verdict, Unavailable> {
    let answers = judge.ask(&queue_state(mr, now), &queue_questions()).await?;
    verdict(&answers, mr)
}

/// Asks Jev about an open MR: whether it waits on me (when it has notes), and each file's risk, all at once.
pub async fn judge_open(judge: &impl Judge, waits: Option<Value>, files: Vec<(String, Value)>) -> Result<Reading, Unavailable> {
    let waits_on_me = async {
        match &waits {
            Some(state) => self::waits(&judge.ask(state, &waits_questions()).await?),
            None => Ok(false),
        }
    };
    let risks = futures_util::future::join_all(files.iter().map(|(path, state)| async move {
        let answers = judge.ask(state, &risk_questions()).await?;
        Ok::<_, Unavailable>((path.clone(), risk(&answers)?))
    }));
    let (waits_on_me, risks) = tokio::join!(waits_on_me, risks);
    Ok(Reading { waits_on_me: waits_on_me?, risks: risks.into_iter().collect::<Result<_, _>>()? })
}

fn sign(kind: crate::diff::LineKind) -> char {
    match kind {
        crate::diff::LineKind::Added => '+',
        crate::diff::LineKind::Removed => '-',
        crate::diff::LineKind::Context => ' ',
    }
}

fn cut(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn answers(raw: Value) -> Answers {
        serde_json::from_value(raw).unwrap()
    }

    fn mr() -> QueueMr {
        crate::forge::gitlab::fixture::queue(include_str!("../forge/gitlab/fixtures/queue.json")).review_requested[0].clone()
    }

    #[test]
    fn a_queue_answer_becomes_a_verdict_bound_to_what_jev_saw() {
        let mr = mr();
        let judged = verdict(&answers(json!({"urgency": {"score": 2.7}, "size": {"choice": "sprawling"}})), &mr).unwrap();
        assert!(judged.urgent());
        assert_eq!(judged.size, Size::Sprawling);
        assert!(judged.fresh_for(&mr));
        let moved = QueueMr { updated_at: mr.updated_at + chrono::TimeDelta::minutes(1), ..mr.clone() };
        assert!(!judged.fresh_for(&moved), "an MR that moved is asked again");
        assert!(verdict(&answers(json!({"urgency": {"score": 1.0}})), &mr).is_err(), "a missing answer is unavailable");
    }

    #[test]
    fn risk_rounds_to_the_nearest_level_and_waits_needs_a_clear_yes() {
        let at = |level: f64| risk(&answers(json!({"risk": {"score": level}}))).unwrap();
        assert_eq!((at(0.3), at(0.6), at(1.9), at(2.6)), (Risk::Cosmetic, Risk::Logic, Risk::Data, Risk::Security));
        assert!(waits(&answers(json!({"waits_on_me": {"noul": 0.8}}))).unwrap());
        assert!(!waits(&answers(json!({"waits_on_me": {"noul": 0.5}}))).unwrap());
    }

    #[test]
    fn the_queue_state_carries_facts_not_the_forge_token() {
        let mr = QueueMr { description: "x".repeat(5000), ..mr() };
        let state = queue_state(&mr, mr.updated_at + chrono::TimeDelta::days(3));
        assert_eq!(state["idle_days"], 3);
        assert!(state["description"].as_str().unwrap().chars().count() <= DESCRIPTION_CHARS + 1);
        assert!(state.get("web_url").is_none());
    }

    #[tokio::test]
    async fn an_open_mr_is_read_file_by_file_and_one_failure_fails_the_reading() {
        use crate::ai::typesafe::stub::Stub;
        let stub = Stub(|state| {
            if state.get("path").is_some() {
                let level = if state["path"].as_str().unwrap().contains("auth") { 3.0 } else { 0.0 };
                return Ok(json!({"risk": {"score": level}}));
            }
            Ok(json!({"waits_on_me": {"noul": 0.9}}))
        });
        let files =
            vec![("src/auth.rs".to_owned(), json!({"path": "src/auth.rs"})), ("README.md".to_owned(), json!({"path": "README.md"}))];
        let reading = judge_open(&stub, Some(json!({"last_notes": []})), files.clone()).await.unwrap();
        assert!(reading.waits_on_me);
        assert_eq!(reading.risks["src/auth.rs"], Risk::Security);
        assert_eq!(reading.risks["README.md"], Risk::Cosmetic);
        assert!(!judge_open(&stub, None, vec![]).await.unwrap().waits_on_me, "no notes, nothing waits");
        let broken = Stub(|_| Err(Unavailable("quota".into())));
        assert!(judge_open(&broken, None, files).await.is_err());
    }

    #[test]
    fn the_open_mr_state_keeps_the_last_three_notes_and_every_text_file() {
        let review = crate::review::tests::review();
        let files = file_states(&review);
        assert!(!files.is_empty());
        assert!(files.iter().all(|(path, state)| state["path"] == path.as_str() && state["change"].as_str().is_some()));
        if let Some(state) = waits_state(&review, "nina") {
            assert!(state["last_notes"].as_array().unwrap().len() <= LAST_NOTES);
        }
    }
}
