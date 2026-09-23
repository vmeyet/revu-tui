//! The MRs waiting on me, as every forge answers them, and how they sort into the sidebar.
use super::MrKey;
use super::rules::{self, Reason, Rules};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Queue {
    pub me: String,
    /// The project the queue is scoped to; `None` for every project.
    #[serde(default)]
    pub project: Option<String>,
    pub review_requested: Vec<QueueMr>,
    pub authored: Vec<QueueMr>,
    pub assigned: Vec<QueueMr>,
    /// Every open MR of `project`, mine or not.
    #[serde(default)]
    pub open: Vec<QueueMr>,
}

/// One queue row: enough to draw it, badge it and open it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueMr {
    /// The host it came from, when the queue merges several; `None` for the one `revu` started with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    pub number: u64,
    pub project: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub draft: bool,
    pub web_url: String,
    pub updated_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub source_branch: String,
    pub target_branch: String,
    pub conflicts: bool,
    pub author: String,
    pub author_name: String,
    pub approved: bool,
    pub approved_by: Vec<String>,
    /// Approvals still missing before the forge lets it merge; `None` when the forge does not say.
    #[serde(default)]
    pub approvals_left: Option<u32>,
    pub reviewers: Vec<ReviewerState>,
    /// Upper case, as GitLab's GraphQL spells it: `SUCCESS`, `FAILED`, `RUNNING`, `PENDING`, `CANCELED`…
    pub pipeline: Option<String>,
    pub additions: u32,
    pub deletions: u32,
    pub files: u32,
    pub unresolved: u32,
    pub labels: Vec<String>,
    pub notes: u32,
    /// Everyone who commented, the author included.
    #[serde(default)]
    pub commenters: Vec<String>,
    /// Why the "needs me" rules moved it, or sorted it last; set when the sections are built.
    #[serde(skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub reason: Option<Reason>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewerState {
    pub username: String,
    pub state: ReviewState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewState {
    Unreviewed,
    Reviewed,
    RequestedChanges,
    Approved,
    ReviewStarted,
    Unapproved,
}

/// The queue sorted into the sidebar sections, each MR in exactly one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Sections {
    pub to_review: Vec<QueueMr>,
    pub mine: Vec<QueueMr>,
    pub watching: Vec<QueueMr>,
    /// The rest of the project's open MRs, only when the queue is scoped to one.
    pub open: Vec<QueueMr>,
    /// Other people's draft MRs from Watching and Open: not ready, so out of the way.
    pub drafts: Vec<QueueMr>,
    pub done: Vec<QueueMr>,
    /// What the "needs me" rules moved out of To review, Watching and Open, each with its reason.
    pub other: Vec<QueueMr>,
}

impl Sections {
    /// Several hosts' sections as one sidebar: each section holds every host's rows, newest first.
    pub fn merge(parts: Vec<Sections>) -> Sections {
        let mut merged = Sections::default();
        for part in parts {
            merged.to_review.extend(part.to_review);
            merged.mine.extend(part.mine);
            merged.watching.extend(part.watching);
            merged.open.extend(part.open);
            merged.drafts.extend(part.drafts);
            merged.done.extend(part.done);
            merged.other.extend(part.other);
        }
        for section in [
            &mut merged.to_review,
            &mut merged.mine,
            &mut merged.watching,
            &mut merged.open,
            &mut merged.drafts,
            &mut merged.done,
            &mut merged.other,
        ] {
            section.sort_by_key(|mr| std::cmp::Reverse(mr.updated_at));
        }
        merged
    }
}

impl Sections {
    /// Every row, section after section.
    pub fn all(&self) -> impl Iterator<Item = &QueueMr> {
        [&self.to_review, &self.mine, &self.watching, &self.open, &self.drafts, &self.done, &self.other].into_iter().flatten()
    }

    /// Rows from more than one host share the queue: only then does a row need its host's tag.
    /// A queue scoped to a checkout holds one host's rows even when several hosts are logged in.
    pub fn mixes_hosts(&self) -> bool {
        let mut hosts = self.all().map(|mr| mr.host.as_deref());
        hosts.next().is_some_and(|first| hosts.any(|host| host != first))
    }
}

impl QueueMr {
    pub fn my_state(&self, me: &str) -> Option<ReviewState> {
        self.reviewers.iter().find(|r| r.username == me).map(|r| r.state)
    }

    fn reviewed_by(&self, me: &str) -> bool {
        matches!(self.my_state(me), Some(ReviewState::Approved | ReviewState::Reviewed))
    }

    pub fn key(&self) -> MrKey {
        MrKey { host: self.host.clone(), ..MrKey::new(self.project.clone(), self.number) }
    }
}

impl Queue {
    /// Every row marked as coming from `host`, so opening it reaches the right forge.
    pub fn on_host(self, host: &str) -> Self {
        let tag = |rows: Vec<QueueMr>| rows.into_iter().map(|mr| QueueMr { host: Some(host.to_owned()), ..mr }).collect();
        Self {
            review_requested: tag(self.review_requested),
            authored: tag(self.authored),
            assigned: tag(self.assigned),
            open: tag(self.open),
            ..self
        }
    }

    /// The sections as the forge sorted them, before any "needs me" rule.
    #[cfg(test)]
    pub fn sections(&self, watch_labels: &[String]) -> Sections {
        self.sections_with(watch_labels, &Rules::off(), Utc::now())
    }

    /// The sections, then the "needs me" rules: what does not need me leaves To review, Watching
    /// and Open for Other (a draft for Drafts), and what others already review sorts last.
    pub fn sections_with(&self, watch_labels: &[String], rules: &Rules, now: DateTime<Utc>) -> Sections {
        let sorted = self.forge_sections(watch_labels);
        if !rules.enabled {
            return sorted;
        }
        let judged = |rows: Vec<QueueMr>| -> Vec<QueueMr> {
            rows.into_iter().map(|mr| QueueMr { reason: rules::judge(&mr, &self.me, now, rules), ..mr }).collect()
        };
        let mut other = Vec::new();
        let mut drafts = judged(sorted.drafts);
        let mut keep = |rows: Vec<QueueMr>| -> Vec<QueueMr> {
            let (moved, kept): (Vec<_>, Vec<_>) =
                judged(rows).into_iter().partition(|mr| mr.reason.as_ref().is_some_and(Reason::moves_out));
            for mr in moved {
                if mr.reason == Some(Reason::Draft) { drafts.push(mr) } else { other.push(mr) }
            }
            kept
        };
        let to_review = keep(sorted.to_review);
        let watching = keep(sorted.watching);
        let open = keep(sorted.open);
        other.sort_by_key(|mr| std::cmp::Reverse(mr.updated_at));
        Sections { to_review, watching, open, drafts, other, ..sorted }
    }

    fn forge_sections(&self, watch_labels: &[String]) -> Sections {
        let me = self.me.as_str();
        let in_scope = |mr: &&QueueMr| self.project.as_ref().is_none_or(|p| &mr.project == p);
        let (done, to_review): (Vec<_>, Vec<_>) = self.review_requested.iter().filter(in_scope).cloned().partition(|mr| mr.reviewed_by(me));
        let mine: Vec<QueueMr> = self.authored.iter().filter(in_scope).cloned().collect();
        let mut seen: HashSet<MrKey> = to_review.iter().chain(&mine).chain(&done).map(QueueMr::key).collect();
        let labelled = self.review_requested.iter().chain(&self.authored).filter(|mr| mr.labels.iter().any(|l| watch_labels.contains(l)));
        let watching: Vec<QueueMr> =
            self.assigned.iter().chain(labelled).filter(in_scope).filter(|mr| seen.insert(mr.key())).cloned().collect();
        let open: Vec<QueueMr> = self.open.iter().filter(|mr| seen.insert(mr.key())).cloned().collect();
        let (watching_drafts, watching): (Vec<_>, Vec<_>) = watching.into_iter().partition(|mr| mr.draft);
        let (open_drafts, open): (Vec<_>, Vec<_>) = open.into_iter().partition(|mr| mr.draft);
        let drafts = watching_drafts.into_iter().chain(open_drafts).collect();
        Sections { to_review, mine, watching, open, drafts, done, other: vec![] }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::forge::gitlab::fixture;

    fn queue() -> Queue {
        fixture::queue(include_str!("gitlab/fixtures/queue.json"))
    }

    fn scoped() -> Queue {
        fixture::queue_in(include_str!("gitlab/fixtures/queue_scoped.json"), "acme/widgets")
    }

    fn day(d: u32) -> DateTime<Utc> {
        use chrono::TimeZone;
        Utc.with_ymd_and_hms(2026, 9, d, 12, 0, 0).unwrap()
    }

    fn numbers(rows: &[QueueMr]) -> Vec<u64> {
        rows.iter().map(|mr| mr.number).collect()
    }

    fn reasons(rows: &[QueueMr]) -> Vec<String> {
        rows.iter().map(|mr| mr.reason.as_ref().map_or_else(|| "-".to_owned(), ToString::to_string)).collect()
    }

    /// The scoped fixture with the Open MRs changed by `change`, judged on 23 September.
    fn judged(change: impl Fn(QueueMr) -> QueueMr) -> Sections {
        let queue = scoped();
        let open = queue.open.into_iter().map(&change).collect();
        Queue { open, ..scoped() }.sections_with(&[], &Rules::default(), day(23))
    }

    #[test]
    fn with_nothing_to_say_the_rules_change_nothing() {
        let plain = scoped().sections(&[]);
        let ruled = scoped().sections_with(&[], &Rules::default(), day(23));
        assert_eq!(numbers(&ruled.open), numbers(&plain.open));
        assert_eq!(numbers(&ruled.to_review), numbers(&plain.to_review));
        assert!(ruled.other.is_empty());
        assert_eq!(reasons(&ruled.drafts), ["draft"], "a draft says why it waits apart");
    }

    #[test]
    fn stale_failed_and_approved_mrs_leave_open_for_other_with_their_reason() {
        let stale = judged(|mr| if mr.number == 51 { QueueMr { updated_at: day(1), ..mr } } else { mr });
        assert_eq!(numbers(&stale.open), [] as [u64; 0]);
        assert_eq!((numbers(&stale.other), reasons(&stale.other)), (vec![51], vec!["stale 22d".to_owned()]));
        let failed = judged(|mr| if mr.number == 51 { QueueMr { pipeline: Some("FAILED".into()), ..mr } } else { mr });
        assert_eq!(reasons(&failed.other), ["pipeline failed"]);
        let approved =
            judged(|mr| if mr.number == 51 { QueueMr { approved_by: vec!["omar".into()], approvals_left: Some(0), ..mr } } else { mr });
        assert_eq!(reasons(&approved.other), ["1 approval, needs none"]);
    }

    #[test]
    fn what_others_review_stays_in_its_section_with_a_reason() {
        let reviewed = judged(|mr| QueueMr { notes: 5, commenters: vec![mr.author.clone(), "sam".into(), "kim".into()], ..mr });
        assert_eq!(numbers(&reviewed.open), [51]);
        assert_eq!(reasons(&reviewed.open), ["reviewed by 2"]);
        assert!(reviewed.other.is_empty());
    }

    #[test]
    fn a_draft_asking_me_joins_drafts_and_mine_never_moves() {
        let queue = scoped();
        let review_requested =
            queue.review_requested.into_iter().map(|mr| if mr.number == 42 { QueueMr { draft: true, ..mr } } else { mr }).collect();
        let authored = queue.authored.into_iter().map(|mr| QueueMr { updated_at: day(1), pipeline: Some("FAILED".into()), ..mr }).collect();
        let sections = Queue { review_requested, authored, ..scoped() }.sections_with(&[], &Rules::default(), day(23));
        assert!(numbers(&sections.drafts).contains(&42));
        assert!(!numbers(&sections.to_review).contains(&42));
        assert_eq!(numbers(&sections.mine), [41], "my own MR stays mine, stale and failed alike");
    }

    #[test]
    fn switched_off_the_forge_sections_come_back() {
        let off = Rules { enabled: false, ..Rules::default() };
        let queue = Queue { open: scoped().open.into_iter().map(|mr| QueueMr { updated_at: day(1), ..mr }).collect(), ..scoped() };
        let sections = queue.sections_with(&[], &off, day(23));
        assert_eq!(numbers(&sections.open), [51]);
        assert!(sections.other.is_empty() && sections.all().all(|mr| mr.reason.is_none()));
    }

    #[test]
    fn rows_from_another_host_carry_it_into_their_key() {
        let tagged = queue().on_host("github.com");
        let mr = &tagged.review_requested[0];
        assert_eq!(mr.key().host.as_deref(), Some("github.com"));
        assert_ne!(mr.key(), queue().review_requested[0].key(), "the same number on two hosts is two MRs");
    }

    #[test]
    fn sections_mix_hosts_only_when_rows_come_from_two_of_them() {
        let here = queue().sections(&[]);
        assert!(!here.mixes_hosts());
        assert!(!queue().on_host("github.com").sections(&[]).mixes_hosts(), "one other host alone is still one host");
        assert!(Sections::merge(vec![here, queue().on_host("github.com").sections(&[])]).mixes_hosts());
        assert!(!Sections::default().mixes_hosts());
    }

    #[test]
    fn merged_sections_keep_every_host_newest_first() {
        let here = queue().sections(&[]);
        let there = queue().on_host("github.com").sections(&[]);
        let merged = Sections::merge(vec![here.clone(), there]);
        assert_eq!(merged.to_review.len(), here.to_review.len() * 2);
        assert!(merged.mine.windows(2).all(|w| w[0].updated_at >= w[1].updated_at));
        assert_eq!(merged.to_review.iter().filter(|mr| mr.host.is_some()).count(), here.to_review.len());
    }
}
