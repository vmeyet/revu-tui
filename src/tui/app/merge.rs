//! `M` in the review: merge my own approved MR, after a `y` that names the method.
use super::{Action, App, Confirm};
use crate::forge::MrKey;

impl App {
    /// `M`: the question when the MR may merge, else why not.
    pub(super) fn merge_here(&mut self) -> Vec<Action> {
        let Some(open) = &self.open else {
            self.toast("open an MR first");
            return vec![];
        };
        let mr = &open.review.mr;
        if let Some(reason) = mr.merge_refusal() {
            self.warn(format!("cannot merge: {reason}"));
            return vec![];
        }
        let name = format!("{}{}", self.hosts.kind_of(&open.key).sigil(), mr.number);
        self.confirm = Some(Confirm::Merge {
            key: open.key.clone(),
            name,
            head: mr.refs.head.clone(),
            into: mr.target_branch.clone(),
            plan: mr.merge,
        });
        vec![]
    }

    /// The forge merged it: say so, and read the MR and the queue again so it leaves Mine.
    pub(super) fn merged(&mut self, key: &MrKey) -> Vec<Action> {
        self.toast(format!("merged {}{}", self.hosts.kind_of(key).sigil(), key.number));
        self.reread(key)
    }

    /// The MR changed on the forge: read it and the queue again so the header and the rows agree.
    pub(super) fn reread(&mut self, key: &MrKey) -> Vec<Action> {
        let mut actions = vec![Action::RefreshMr(key.clone())];
        actions.extend(self.refresh_queue());
        actions
    }
}
