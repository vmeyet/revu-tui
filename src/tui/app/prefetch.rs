//! Loading ahead: the MRs that need me go into the cache before they are opened, so `enter` paints at once.
use super::{Action, App, MrKey};
use crate::forge::{QueueMr, Sections};
use chrono::{DateTime, Utc};

/// Below this many requests left, nothing is loaded ahead: the window is kept for what the reader asks.
pub const SPARE: u64 = 200;

/// One MR to load ahead, with the last change the queue saw on it: a cache at least that new is kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ahead {
    pub key: MrKey,
    pub updated_at: DateTime<Utc>,
}

/// The MRs worth loading before they are asked for, in the order the reader meets them: ready
/// ones, then review requests, then mine. What I already approved and the open MR are left out.
pub fn plan(sections: &Sections, me: &str, open: Option<&MrKey>, limit: usize) -> Vec<Ahead> {
    let needs_me = sections.ready.iter().chain(&sections.to_review).filter(|mr| !mr.approved_by.iter().any(|who| who == me));
    let mut picked: Vec<Ahead> = vec![];
    for mr in needs_me.chain(&sections.mine) {
        let ahead = ahead(mr);
        if picked.len() == limit {
            break;
        }
        if Some(&ahead.key) != open && !picked.iter().any(|p| p.key == ahead.key) {
            picked.push(ahead);
        }
    }
    picked
}

fn ahead(mr: &QueueMr) -> Ahead {
    Ahead { key: mr.key(), updated_at: mr.updated_at }
}

impl App {
    /// Names the MRs to load ahead, unless it is off, an MR is being opened (it goes first), or
    /// the rate limit runs low. A plan held back by an opening goes out once that MR arrived.
    pub(super) fn prefetch(&mut self) {
        if self.prefetch_limit == 0 {
            return;
        }
        if self.opening.is_some() {
            self.prefetch_due = true;
            return;
        }
        self.prefetch_due = false;
        if self.rate.remaining.is_some_and(|left| left < SPARE) {
            return;
        }
        let Some(sections) = &self.sections else { return };
        let plan = plan(sections, &self.me, self.open.as_ref().map(|o| &o.key), self.prefetch_limit);
        if !plan.is_empty() {
            self.composed.push(Action::Prefetch(plan));
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn mr(number: u64, updated: i64) -> QueueMr {
        let base = crate::forge::gitlab::fixture::queue(include_str!("../../forge/gitlab/fixtures/queue.json")).review_requested[0].clone();
        QueueMr { number, updated_at: DateTime::from_timestamp(updated, 0).unwrap(), approved_by: vec![], ..base }
    }

    fn numbers(plan: &[Ahead]) -> Vec<u64> {
        plan.iter().map(|a| a.key.number).collect()
    }

    #[test]
    fn ready_comes_first_then_review_requests_then_mine_up_to_the_limit() {
        let sections = Sections {
            ready: vec![mr(1, 10)],
            to_review: vec![mr(2, 20), mr(3, 30)],
            mine: vec![mr(4, 40), mr(5, 50)],
            ..Sections::default()
        };
        assert_eq!(numbers(&plan(&sections, "me", None, 5)), [1, 2, 3, 4, 5]);
        assert_eq!(numbers(&plan(&sections, "me", None, 3)), [1, 2, 3]);
        assert!(plan(&sections, "me", None, 0).is_empty());
    }

    #[test]
    fn approved_open_and_repeated_mrs_are_left_out() {
        let mut approved = mr(2, 20);
        approved.approved_by = vec!["me".into()];
        let sections = Sections {
            ready: vec![mr(1, 10), mr(3, 30)],
            to_review: vec![approved, mr(1, 10)],
            mine: vec![mr(4, 40)],
            ..Sections::default()
        };
        let open = mr(3, 30).key();
        assert_eq!(numbers(&plan(&sections, "me", Some(&open), 5)), [1, 4]);
    }

    #[test]
    fn each_plan_keeps_the_time_the_queue_saw() {
        let sections = Sections { to_review: vec![mr(7, 70)], ..Sections::default() };
        assert_eq!(plan(&sections, "me", None, 1)[0].updated_at, DateTime::from_timestamp(70, 0).unwrap());
    }
}
