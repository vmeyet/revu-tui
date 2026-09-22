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

impl QueueMr {
    pub fn my_state(&self, me: &str) -> Option<ReviewState> {
        self.reviewers.iter().find(|r| r.username == me).map(|r| r.state)
    }

    fn reviewed_by(&self, me: &str) -> bool {
        matches!(self.my_state(me), Some(ReviewState::Approved | ReviewState::Reviewed))
    }

    pub fn key(&self) -> MrKey {
        MrKey::new(self.project.clone(), self.number)
    }
}

impl Queue {
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
