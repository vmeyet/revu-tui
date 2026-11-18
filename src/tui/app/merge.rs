//! `M` in the review: merge my own MR once the forge says it can merge, after a `y` that names the method.
use super::{Action, App, Confirm};
use crate::forge::{MergeVerdict, MrKey};

impl App {
    /// `M`: the question when the MR may merge, else why not.
    /// A forge still checking is read again at once, so the next `M` decides on its answer.
    pub(super) fn merge_here(&mut self) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let mr = &open.review.mr;
        if let Some(reason) = mr.merge_refusal() {
            let read_again = if mr.merge_verdict == MergeVerdict::Checking { vec![Action::RefreshMr(open.key.clone())] } else { vec![] };
            self.warn(format!("cannot merge: {reason}"));
            return read_again;
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

#[cfg(test)]
mod tests {
    use crate::tui::app::test_support::*;

    #[test]
    fn big_m_asks_first_names_the_method_and_merges_only_on_y() {
        let mut app = with_mr(approved_and_mine);
        assert_eq!(press(&mut app, "M"), vec![]);
        assert!(render(&mut app, 150, 20).contains("merge !42 into main (squash, delete the branch)? y merges"));
        assert_eq!(press(&mut app, "n"), vec![], "any other key cancels");
        assert_eq!(app.live_toast().unwrap().text, "not merged");
        press(&mut app, "M");
        let actions = press(&mut app, "y");
        let [Action::Merge { key, head, plan }] = actions.as_slice() else { panic!("{actions:?}") };
        assert_eq!((key, head.as_str(), plan.method), (&mr_key(), "bbbb", crate::forge::MergeMethod::Squash));
        app.apply(Incoming::Merged { key: mr_key() });
        let follow = app.take_actions();
        assert!(follow.contains(&Action::RefreshMr(mr_key())), "{follow:?}");
        assert!(follow.iter().any(|a| matches!(a, Action::LoadQueue { .. })), "the queue reads again so the MR leaves Mine: {follow:?}");
        assert!(app.live_toast().unwrap().text.contains("merged !42"));
    }

    #[test]
    fn big_m_says_why_it_will_not_merge() {
        type Change = fn(Mr) -> Mr;
        let cases: [(Change, &str); 3] = [
            (|mr| mr, "cannot merge: only your own MRs merge from revu"),
            (|mr| Mr { mine: true, ..mr }, "cannot merge: it needs 1 more approval"),
            (
                |mr| Mr { merge_verdict: crate::forge::MergeVerdict::UnresolvedThreads, ..approved_and_mine(mr) },
                "cannot merge: its threads must be resolved first",
            ),
        ];
        for (change, reason) in cases {
            let mut app = with_mr(change);
            assert_eq!(press(&mut app, "M"), vec![]);
            assert!(app.confirm.is_none());
            assert_eq!(app.live_toast().unwrap().text, reason);
        }
    }

    #[test]
    fn big_m_on_an_mr_the_forge_is_still_checking_says_to_try_again_and_reads_it_again() {
        let mut app = with_mr(|mr| Mr { merge_verdict: crate::forge::MergeVerdict::Checking, ..approved_and_mine(mr) });
        assert_eq!(press(&mut app, "M"), vec![Action::RefreshMr(mr_key())]);
        assert!(app.confirm.is_none());
        assert_eq!(app.live_toast().unwrap().text, "cannot merge: the forge is still checking whether it can merge: try again in a moment");
    }
}
