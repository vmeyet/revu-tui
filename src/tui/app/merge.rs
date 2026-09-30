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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
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
                |mr| Mr {
                    pipeline: Some(crate::forge::Pipeline { status: PipelineStatus::Failed, web_url: None }),
                    ..approved_and_mine(mr)
                },
                "cannot merge: its pipeline failed",
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
    fn colon_merge_asks_like_big_m() {
        let mut app = with_mr(approved_and_mine);
        press(&mut app, ":merge");
        app.handle_key(code(KeyCode::Enter));
        assert!(matches!(app.confirm, Some(Confirm::Merge { .. })));
    }
}
