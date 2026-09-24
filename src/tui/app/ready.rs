//! `H` in the review: my own MR turns ready for review when it is a draft, a draft when it is ready.
use super::{Action, App};
use crate::forge::MrKey;

impl App {
    /// `H`: the other state for my open MR, else why not.
    pub(super) fn toggle_draft(&mut self) -> Vec<Action> {
        let Some(open) = &self.open else {
            self.toast("open an MR first");
            return vec![];
        };
        let mr = &open.review.mr;
        if let Some(reason) = mr.draft_refusal() {
            self.warn(format!("cannot change {}{}: {reason}", self.hosts.kind_of(&open.key).sigil(), mr.number));
            return vec![];
        }
        vec![Action::SetDraft { key: open.key.clone(), draft: !mr.draft }]
    }

    /// The forge took it: say which state it is in now, and read the MR and the queue again.
    pub(super) fn draft_set(&mut self, key: &MrKey, draft: bool) -> Vec<Action> {
        let state = if draft { "a draft" } else { "ready for review" };
        self.toast(format!("{}{} is {state}", self.hosts.kind_of(key).sigil(), key.number));
        self.reread(key)
    }
}
