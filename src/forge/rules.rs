//! The "needs me" rules: which queue MRs are worth my attention now, and why the others are not.
//! Plain rules over what the forge already answers: instant, free, and each one says why.
use super::queue::{QueueMr, ReviewState};
use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

/// `[queue.rules]`: every threshold the rules use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Rules {
    /// `false` leaves every section as the forge sorted it.
    pub enabled: bool,
    /// An MR with no activity for more days than this moves to Other.
    pub stale_days: u32,
    /// This many comments from others, and none from me, and someone already reviews it.
    pub reviewed_comments: u32,
    /// Words in a title, or in the first paragraph of a description, that say "not yet".
    pub not_ready: Vec<String>,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            enabled: true,
            stale_days: 14,
            reviewed_comments: 3,
            not_ready: ["wip", "do not review", "don't review", "not ready"].map(str::to_owned).to_vec(),
        }
    }
}

impl Rules {
    /// No rule at all: the sections as the forge answered them.
    #[cfg(test)]
    pub fn off() -> Self {
        Self { enabled: false, ..Self::default() }
    }

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Thresholds that would silence everything, or match every title, fail at load time.
    pub fn check(&self) -> Result<()> {
        if self.stale_days == 0 {
            bail!("`queue.rules.stale_days` must be at least 1");
        }
        if self.reviewed_comments == 0 {
            bail!("`queue.rules.reviewed_comments` must be at least 1");
        }
        if let Some(blank) = self.not_ready.iter().position(|w| w.trim().is_empty()) {
            bail!("`queue.rules.not_ready` entry {} is empty: it would match every title", blank + 1);
        }
        Ok(())
    }
}

/// Why the rules moved an MR out of the sections that need me, or pushed it down its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    Stale { days: i64 },
    ApprovedEnough { approvals: usize },
    ReviewedBy { people: usize },
    Draft,
    PipelineFailed,
    NotReadyWord { word: String, place: Place },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Title,
    Description,
}

impl Reason {
    /// Reviewed by others keeps its section and only sorts last; every other reason moves the MR out.
    pub fn moves_out(&self) -> bool {
        !matches!(self, Reason::ReviewedBy { .. })
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reason::Stale { days } => write!(f, "stale {days}d"),
            Reason::ApprovedEnough { approvals: 1 } => f.write_str("1 approval, needs none"),
            Reason::ApprovedEnough { approvals } => write!(f, "{approvals} approvals, needs none"),
            Reason::ReviewedBy { people } => write!(f, "reviewed by {people}"),
            Reason::Draft => f.write_str("draft"),
            Reason::PipelineFailed => f.write_str("pipeline failed"),
            Reason::NotReadyWord { word, place: Place::Title } => write!(f, "\"{word}\" in title"),
            Reason::NotReadyWord { word, place: Place::Description } => write!(f, "\"{word}\" in description"),
        }
    }
}

impl Serialize for Reason {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// The first reason that applies to someone else's MR, most decisive first.
/// "Not ready" holds everywhere; stale and approved enough spare an MR that asks me by name,
/// since a review request is the one signal nobody sends by accident.
pub fn judge(mr: &QueueMr, me: &str, now: DateTime<Utc>, rules: &Rules) -> Option<Reason> {
    if !rules.enabled || mr.author == me {
        return None;
    }
    let asks_me = matches!(mr.my_state(me), Some(ReviewState::Unreviewed | ReviewState::ReviewStarted | ReviewState::Unapproved));
    not_ready(mr, rules)
        .or_else(|| (!asks_me).then(|| stale(mr, now, rules)).flatten())
        .or_else(|| (!asks_me).then(|| approved_enough(mr)).flatten())
        .or_else(|| reviewed_by_others(mr, me, rules))
}

fn not_ready(mr: &QueueMr, rules: &Rules) -> Option<Reason> {
    if mr.draft {
        return Some(Reason::Draft);
    }
    if mr.pipeline.as_deref() == Some("FAILED") {
        return Some(Reason::PipelineFailed);
    }
    let first_paragraph = mr.description.split("\n\n").next().unwrap_or("");
    rules.not_ready.iter().find_map(|word| {
        let place = if says(&mr.title, word) {
            Place::Title
        } else if says(first_paragraph, word) {
            Place::Description
        } else {
            return None;
        };
        Some(Reason::NotReadyWord { word: word.clone(), place })
    })
}

fn stale(mr: &QueueMr, now: DateTime<Utc>, rules: &Rules) -> Option<Reason> {
    let days = (now - mr.updated_at).num_days();
    (days > i64::from(rules.stale_days)).then_some(Reason::Stale { days })
}

/// At least one approval and none left to give: the forge would let it merge without me.
fn approved_enough(mr: &QueueMr) -> Option<Reason> {
    let approvals = mr.approved_by.len();
    (approvals > 0 && mr.approvals_left == Some(0)).then_some(Reason::ApprovedEnough { approvals })
}

/// Enough comments, from people other than the author, and none from me: someone has it in hand.
fn reviewed_by_others(mr: &QueueMr, me: &str, rules: &Rules) -> Option<Reason> {
    if mr.commenters.iter().any(|c| c == me) || mr.notes < rules.reviewed_comments {
        return None;
    }
    let people = mr.commenters.iter().filter(|c| **c != mr.author).count();
    (people > 0).then_some(Reason::ReviewedBy { people })
}

/// `word` appears in `text` as a whole word or phrase, whatever the case: "wip" matches
/// "[WIP] fix" but not "wiping".
fn says(text: &str, word: &str) -> bool {
    let (text, word) = (text.to_lowercase(), word.trim().to_lowercase());
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    text.match_indices(&word).any(|(at, found)| {
        let before = text[..at].chars().next_back().is_none_or(|c| !is_word(c));
        let after = text[at + found.len()..].chars().next().is_none_or(|c| !is_word(c));
        before && after
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::forge::queue::ReviewerState;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 23, 12, 0, 0).unwrap()
    }

    fn mr() -> QueueMr {
        QueueMr {
            host: None,
            number: 42,
            project: "acme/widgets".into(),
            title: "feat: charge cards at checkout".into(),
            description: "Charges the card once.\n\nNot ready for the second part yet.".into(),
            draft: false,
            web_url: "https://gitlab.com/acme/widgets/-/merge_requests/42".into(),
            updated_at: now() - chrono::Duration::days(1),
            created_at: now() - chrono::Duration::days(3),
            source_branch: "feat/charge".into(),
            target_branch: "main".into(),
            conflicts: false,
            author: "nina".into(),
            author_name: "Nina".into(),
            approved: false,
            approved_by: vec![],
            approvals_left: Some(1),
            reviewers: vec![],
            pipeline: Some("SUCCESS".into()),
            additions: 10,
            deletions: 2,
            files: 1,
            unresolved: 0,
            labels: vec![],
            notes: 0,
            commenters: vec![],
            reason: None,
        }
    }

    fn reason(mr: &QueueMr) -> Option<String> {
        judge(mr, "me", now(), &Rules::default()).map(|r| r.to_string())
    }

    #[test]
    fn a_fresh_ready_mr_needs_me() {
        assert_eq!(reason(&mr()), None);
    }

    #[test]
    fn each_rule_says_why() {
        let cases: Vec<(QueueMr, &str)> = vec![
            (QueueMr { updated_at: now() - chrono::Duration::days(21), ..mr() }, "stale 21d"),
            (QueueMr { approved_by: vec!["omar".into(), "lea".into()], approvals_left: Some(0), ..mr() }, "2 approvals, needs none"),
            (QueueMr { approved_by: vec!["omar".into()], approvals_left: Some(0), ..mr() }, "1 approval, needs none"),
            (QueueMr { notes: 4, commenters: vec!["nina".into(), "omar".into(), "lea".into(), "sam".into()], ..mr() }, "reviewed by 3"),
            (QueueMr { draft: true, ..mr() }, "draft"),
            (QueueMr { pipeline: Some("FAILED".into()), ..mr() }, "pipeline failed"),
            (QueueMr { title: "[WIP] charge cards".into(), ..mr() }, "\"wip\" in title"),
            (QueueMr { description: "Do not review until the API lands.".into(), ..mr() }, "\"do not review\" in description"),
        ];
        for (mr, why) in cases {
            assert_eq!(reason(&mr).as_deref(), Some(why), "{}", mr.title);
        }
    }

    #[test]
    fn edge_cases_leave_the_mr_alone() {
        let cases: Vec<(QueueMr, &str)> = vec![
            (QueueMr { updated_at: now() - chrono::Duration::days(14), ..mr() }, "exactly the stale limit"),
            (QueueMr { approvals_left: Some(0), ..mr() }, "no approval needed but nobody approved"),
            (QueueMr { approved_by: vec!["omar".into()], approvals_left: Some(1), ..mr() }, "one more approval needed"),
            (QueueMr { approved_by: vec!["omar".into()], approvals_left: None, ..mr() }, "the forge does not say"),
            (QueueMr { notes: 2, commenters: vec!["omar".into(), "lea".into()], ..mr() }, "under the comment threshold"),
            (QueueMr { notes: 5, commenters: vec!["omar".into(), "me".into()], ..mr() }, "I already commented"),
            (QueueMr { notes: 5, commenters: vec!["nina".into()], ..mr() }, "only the author talks"),
            (QueueMr { title: "fix: stop wiping the cart".into(), ..mr() }, "wip inside a word"),
            (QueueMr { description: "Part one.\n\nNot ready: part two, later.".into(), ..mr() }, "not ready past the first paragraph"),
            (QueueMr { pipeline: Some("RUNNING".into()), ..mr() }, "running is not failed"),
        ];
        for (mr, case) in cases {
            assert_eq!(reason(&mr), None, "{case}");
        }
    }

    #[test]
    fn not_ready_wins_over_the_others_and_mine_is_never_judged() {
        let busy = QueueMr { draft: true, updated_at: now() - chrono::Duration::days(30), ..mr() };
        assert_eq!(reason(&busy).as_deref(), Some("draft"));
        let mine = QueueMr { author: "me".into(), draft: true, ..mr() };
        assert_eq!(reason(&mine), None);
    }

    #[test]
    fn a_review_request_to_me_pins_it_against_stale_and_approved_but_not_against_not_ready() {
        let asked = |state| QueueMr { reviewers: vec![ReviewerState { username: "me".into(), state }], ..mr() };
        let old_and_approved = |m: QueueMr| QueueMr {
            updated_at: now() - chrono::Duration::days(40),
            approved_by: vec!["omar".into()],
            approvals_left: Some(0),
            ..m
        };
        assert_eq!(reason(&old_and_approved(asked(ReviewState::Unreviewed))), None);
        assert_eq!(reason(&old_and_approved(asked(ReviewState::Approved))).as_deref(), Some("stale 40d"), "already done by me");
        assert_eq!(reason(&QueueMr { draft: true, ..asked(ReviewState::Unreviewed) }).as_deref(), Some("draft"));
    }

    #[test]
    fn switched_off_nothing_moves() {
        let draft = QueueMr { draft: true, ..mr() };
        assert_eq!(judge(&draft, "me", now(), &Rules::off()), None);
    }

    #[test]
    fn only_reviewed_by_others_keeps_its_section() {
        assert!(!Reason::ReviewedBy { people: 2 }.moves_out());
        assert!(Reason::Stale { days: 30 }.moves_out());
        assert!(Reason::Draft.moves_out());
    }

    #[test]
    fn thresholds_that_would_silence_everything_are_refused() {
        assert!(Rules::default().check().is_ok());
        assert!(Rules { stale_days: 0, ..Rules::default() }.check().unwrap_err().to_string().contains("stale_days"));
        assert!(Rules { reviewed_comments: 0, ..Rules::default() }.check().unwrap_err().to_string().contains("reviewed_comments"));
        let blank = Rules { not_ready: vec!["wip".into(), "  ".into()], ..Rules::default() };
        assert!(blank.check().unwrap_err().to_string().contains("entry 2"));
    }

    #[test]
    fn words_match_whole_and_case_blind() {
        assert!(says("[WIP] Charge", "wip"));
        assert!(says("Don't review yet", "don't review"));
        assert!(!says("wiping", "wip"));
        assert!(!says("swipe", "wip"));
        assert!(says("wip", "WIP"));
    }
}
