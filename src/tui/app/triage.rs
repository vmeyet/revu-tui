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

    pub(super) fn apply_read(&mut self, key: MrKey, head: String, reading: Reading) {
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
