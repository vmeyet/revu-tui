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
    use crate::tui::app::test_support::*;

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
        assert_eq!(plan(&sections, "me", None, 0), [] as [Ahead; 0]);
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

    fn planned(actions: &[Action]) -> Option<Vec<u64>> {
        actions.iter().find_map(|a| match a {
            Action::Prefetch(plan) => Some(plan.iter().map(|ahead| ahead.key.number).collect()),
            _ => None,
        })
    }

    fn fresh_queue() -> Incoming {
        Incoming::Queue { scope: None, me: "nina".into(), sections: sections(), opened: HashMap::new(), cached: false }
    }

    #[test]
    fn a_fresh_queue_loads_ahead_what_needs_me_and_a_cached_one_does_not() {
        let mut app = App::new(Settings { prefetch: 5, ..settings() });
        app.apply(Incoming::Queue { scope: None, me: "nina".into(), sections: sections(), opened: HashMap::new(), cached: true });
        assert_eq!(planned(&app.take_actions()), None, "a cached answer is not news");
        app.apply(fresh_queue());
        let plan = planned(&app.take_actions()).expect("a plan");
        assert!(!plan.is_empty() && plan.len() <= 5, "{plan:?}");
    }

    #[test]
    fn loading_ahead_waits_for_an_opening_then_goes() {
        let mut app = App::new(Settings { prefetch: 5, ..settings() });
        app.opening = Some(mr_key());
        app.apply(fresh_queue());
        assert_eq!(planned(&app.take_actions()), None, "the MR being opened goes first");
        app.apply(Incoming::Review { key: mr_key(), review: Box::new(review()), cached: None });
        let plan = planned(&app.take_actions()).expect("the held plan goes out once the MR arrived");
        assert!(!plan.contains(&mr_key().number), "the open MR is not loaded again");
    }

    #[test]
    fn loading_ahead_is_off_at_zero_and_when_requests_run_low() {
        let mut off = App::new(Settings { prefetch: 0, ..settings() });
        off.apply(fresh_queue());
        assert_eq!(planned(&off.take_actions()), None);
        let mut low = App::new(Settings { prefetch: 5, ..settings() });
        low.rate = crate::forge::RateLimit { remaining: Some(150), wait: None };
        low.apply(fresh_queue());
        assert_eq!(planned(&low.take_actions()), None);
    }
}
