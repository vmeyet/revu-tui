//! Jev's marks in the app: which MRs to ask about, what came back, and how a row wears it.
use super::{Action, App, MrKey};
use crate::ai::triage::{self, Reading, Verdict};
use crate::forge::QueueMr;

/// The mark before a queue row's title, most pressing first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// The last note asks me something.
    WaitsOnMe,
    /// Someone is blocked on this review.
    Urgent,
    /// Many files and ideas: plan the time.
    Sprawling,
}

impl App {
    /// After a fresh queue: every MR Jev has not seen as it is now, once.
    pub(super) fn ask_triage(&mut self) {
        if !self.triage {
            return;
        }
        let Some(sections) = &self.sections else { return };
        let stale: Vec<QueueMr> = [&sections.to_review, &sections.mine, &sections.watching, &sections.open]
            .into_iter()
            .flatten()
            .filter(|mr| triage::stale(&self.verdicts, mr) && !self.triage_asked.contains(&mr.key()))
            .cloned()
            .collect();
        for mr in stale {
            self.triage_asked.insert(mr.key());
            self.composed.push(Action::Triage(Box::new(mr)));
        }
    }

    /// After a fresh review: the open MR's reading, unless Jev already read this head commit.
    pub(super) fn ask_reading(&mut self) {
        let Some(open) = self.open.as_ref().filter(|_| self.triage) else { return };
        let head = open.review.mr.refs.head.clone();
        if self.readings.get(&open.key).is_some_and(|(read, _)| *read == head) {
            return;
        }
        self.composed.push(Action::Read {
            key: open.key.clone(),
            head,
            waits: triage::waits_state(&open.review, &self.me),
            files: triage::file_states(&open.review),
        });
    }

    pub(super) fn apply_triaged(&mut self, key: MrKey, verdict: Verdict) {
        self.triage_asked.remove(&key);
        self.verdicts.insert(key, verdict);
    }

    pub(super) fn apply_read(&mut self, key: MrKey, head: crate::forge::Sha, reading: Reading) {
        self.readings.insert(key, (head, reading));
    }

    /// Jev could not answer: say so once (`message` is its notice) and keep the plain views for the rest of the session.
    pub(super) fn triage_failed(&mut self, message: &str) {
        if self.triage {
            self.triage = false;
            self.warn(message.to_owned());
        }
    }

    /// Whether Jev has said anything yet: the queue grows its mark column only then.
    pub fn triaged(&self) -> bool {
        !self.verdicts.is_empty() || !self.readings.is_empty()
    }

    pub fn mark(&self, mr: &QueueMr) -> Option<Mark> {
        let key = mr.key();
        let waits = self.readings.get(&key).is_some_and(|(_, r)| r.waits_on_me);
        let verdict = self.verdicts.get(&key).filter(|v| v.fresh_for(mr));
        let urgent = verdict.is_some_and(Verdict::urgent);
        let sprawling = verdict.is_some_and(|v| v.size == triage::Size::Sprawling);
        [(waits, Mark::WaitsOnMe), (urgent, Mark::Urgent), (sprawling, Mark::Sprawling)]
            .into_iter()
            .find_map(|(on, mark)| on.then_some(mark))
    }

    /// How urgent Jev found an MR; one it has not judged sits in the middle, so it neither leads nor sinks.
    pub(super) fn urgency(&self, mr: &QueueMr) -> f64 {
        self.verdicts.get(&mr.key()).filter(|v| v.fresh_for(mr)).map_or(1.5, |v| v.urgency)
    }

    /// Each file's risk in the open MR, as Jev read it.
    pub fn risk(&self, path: &str) -> Option<triage::Risk> {
        let open = self.open.as_ref()?;
        self.readings.get(&open.key)?.1.risks.get(path).copied()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crate::tui::app::test_support::*;

    fn triaging() -> App {
        let mut app = App::new(Settings { triage: true, ..settings() });
        app.today = today();
        app
    }

    fn verdict_for(mr: &crate::forge::QueueMr, urgency: f64, size: crate::ai::triage::Size) -> crate::ai::triage::Verdict {
        crate::ai::triage::Verdict { urgency, size, seen: mr.updated_at }
    }

    #[test]
    fn a_fresh_queue_asks_jev_once_per_mr_and_not_at_all_when_it_is_off() {
        let mut off = app();
        off.apply(Incoming::Queue { scope: None, me: "nina".into(), sections: sections(), opened: HashMap::new(), cached: false });
        assert!(off.take_actions().is_empty(), "nothing leaves while Jev is off");
        let mut app = triaging();
        app.apply(Incoming::Queue { scope: None, me: "nina".into(), sections: sections(), opened: HashMap::new(), cached: true });
        assert!(app.take_actions().is_empty(), "a cached queue asks nothing");
        let fresh = || Incoming::Queue { scope: None, me: "nina".into(), sections: sections(), opened: HashMap::new(), cached: false };
        app.apply(fresh());
        let asked = app.take_actions();
        assert!(!asked.is_empty() && asked.iter().all(|a| matches!(a, Action::Triage(_))), "{asked:?}");
        app.apply(fresh());
        assert!(app.take_actions().is_empty(), "an MR being asked about is not asked twice");
    }

    #[test]
    fn verdicts_mark_rows_and_lead_the_review_section_by_urgency() {
        let mut app = triaging();
        app.apply(Incoming::Queue { scope: None, me: "nina".into(), sections: sections(), opened: HashMap::new(), cached: false });
        let _ = app.take_actions();
        let to_review = app.sections.clone().unwrap().to_review;
        let last = to_review.last().unwrap().clone();
        app.apply(Incoming::Triaged { key: last.key(), verdict: verdict_for(&last, 2.9, crate::ai::triage::Size::Focused) });
        assert_eq!(app.mark(&last), Some(Mark::Urgent));
        let first_row = app.queue_rows().into_iter().find_map(|r| match r {
            QueueRow::Mr(mr) | QueueRow::Stacked(mr) => Some(mr.key()),
            QueueRow::Section { .. } | QueueRow::Author { .. } | QueueRow::Stack { .. } => None,
        });
        assert_eq!(first_row, Some(last.key()), "the urgent MR leads To review");
        let moved = crate::forge::QueueMr { updated_at: last.updated_at + chrono::TimeDelta::hours(1), ..last.clone() };
        assert_eq!(app.mark(&moved), None, "a verdict on an older state marks nothing");
        let screen = render(&mut app, 100, 16);
        let meta = screen.lines().position(|l| l.contains(&format!("!{} ·", last.number))).unwrap();
        let ends_with_mark = |line: &str| line.split("││").next().unwrap_or("").trim_end_matches([' ', '│']).ends_with('!');
        assert!(ends_with_mark(screen.lines().nth(meta - 1).unwrap()), "the mark sits at the end of the title line:\n{screen}");
        app.queue_layout = crate::config::QueueLayout::Compact;
        let screen = render(&mut app, 100, 16);
        let row = screen.lines().find(|l| l.contains(&format!("!{} ", last.number))).unwrap();
        assert!(ends_with_mark(row), "{screen}");
    }

    #[test]
    fn a_fresh_review_asks_for_a_reading_once_per_head_and_it_tints_the_tree() {
        let mut app = triaging();
        app.apply(Incoming::Queue { scope: None, me: "nina".into(), sections: sections(), opened: HashMap::new(), cached: false });
        let _ = app.take_actions();
        app.queue_move(0);
        app.handle_key(code(KeyCode::Enter));
        app.apply(Incoming::Review { key: mr_key(), review: Box::new(review()), cached: None });
        let asked: Vec<Action> = app.take_actions().into_iter().filter(|a| !matches!(a, Action::LoadDeployments { .. })).collect();
        let [Action::Read { head, files, .. }] = asked.as_slice() else { panic!("{asked:?}") };
        assert!(!files.is_empty());
        let path = files[0].0.clone();
        let reading = crate::ai::triage::Reading {
            waits_on_me: true,
            risks: std::collections::BTreeMap::from([(path.clone(), crate::ai::triage::Risk::Security)]),
        };
        app.apply(Incoming::Read { key: mr_key(), head: head.clone(), reading });
        assert_eq!(app.risk(&path), Some(crate::ai::triage::Risk::Security));
        app.apply(Incoming::Review { key: mr_key(), review: Box::new(review()), cached: None });
        assert!(app.take_actions().is_empty(), "the same head is not read twice");
        let queued = app.sections.clone().unwrap().to_review.into_iter().find(|mr| mr.key() == mr_key()).unwrap();
        assert_eq!(app.mark(&queued), Some(Mark::WaitsOnMe), "waiting on me outranks every other mark");
        press(&mut app, "t");
        let buffer = cells(&mut app, 140, 30);
        let name = path.rsplit('/').next().unwrap();
        assert_eq!(cell_of(&buffer, name).fg, app.theme.danger, "a security file reads red in the tree");
    }

    #[test]
    fn jev_failing_says_so_once_and_stops_asking() {
        let mut app = triaging();
        app.apply(Incoming::Failed { what: Failure::Triage, message: "⚠ typesafe unavailable: quota exhausted".into() });
        assert!(!app.triage);
        assert!(app.live_toast().is_some_and(|t| t.text.contains("quota")));
        app.apply(Incoming::Failed { what: Failure::Triage, message: "again".into() });
        assert!(app.live_toast().is_some_and(|t| t.text.contains("quota")), "the second failure stays quiet");
        app.apply(Incoming::Queue { scope: None, me: "nina".into(), sections: sections(), opened: HashMap::new(), cached: false });
        assert!(app.take_actions().is_empty());
    }
}
