//! The MRs waiting on me, as every forge answers them, and how they sort into the sidebar.
use super::MrKey;
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
    pub reviewers: Vec<ReviewerState>,
    /// Upper case, as GitLab's GraphQL spells it: `SUCCESS`, `FAILED`, `RUNNING`, `PENDING`, `CANCELED`…
    pub pipeline: Option<String>,
    pub additions: u32,
    pub deletions: u32,
    pub files: u32,
    pub unresolved: u32,
    pub labels: Vec<String>,
    pub notes: u32,
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
    pub done: Vec<QueueMr>,
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
            merged.done.extend(part.done);
        }
        for section in [&mut merged.to_review, &mut merged.mine, &mut merged.watching, &mut merged.open, &mut merged.done] {
            section.sort_by_key(|mr| std::cmp::Reverse(mr.updated_at));
        }
        merged
    }
}

impl Sections {
    /// Every row, section after section.
    pub fn all(&self) -> impl Iterator<Item = &QueueMr> {
        [&self.to_review, &self.mine, &self.watching, &self.open, &self.done].into_iter().flatten()
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

    pub fn sections(&self, watch_labels: &[String]) -> Sections {
        let me = self.me.as_str();
        let in_scope = |mr: &&QueueMr| self.project.as_ref().is_none_or(|p| &mr.project == p);
        let (done, to_review): (Vec<_>, Vec<_>) = self.review_requested.iter().filter(in_scope).cloned().partition(|mr| mr.reviewed_by(me));
        let mine: Vec<QueueMr> = self.authored.iter().filter(in_scope).cloned().collect();
        let mut seen: HashSet<MrKey> = to_review.iter().chain(&mine).chain(&done).map(QueueMr::key).collect();
        let labelled = self.review_requested.iter().chain(&self.authored).filter(|mr| mr.labels.iter().any(|l| watch_labels.contains(l)));
        let watching = self.assigned.iter().chain(labelled).filter(in_scope).filter(|mr| seen.insert(mr.key())).cloned().collect();
        let open = self.open.iter().filter(|mr| seen.insert(mr.key())).cloned().collect();
        Sections { to_review, mine, watching, open, done }
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
