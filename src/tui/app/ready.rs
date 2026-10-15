//! `H` in the review: my own MR turns ready for review when it is a draft, a draft when it is ready.
use super::{Action, App};
use crate::forge::{MrKey, Readiness};

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
        let readiness = if mr.draft { Readiness::Ready } else { Readiness::Draft };
        vec![Action::SetReadiness { key: open.key.clone(), readiness }]
    }

    /// The forge took it: say which state it is in now, and read the MR and the queue again.
    pub(super) fn readiness_set(&mut self, key: &MrKey, readiness: Readiness) -> Vec<Action> {
        self.toast(format!("{}{} is {}", self.hosts.kind_of(key).sigil(), key.number, readiness.described()));
        self.reread(key)
    }
}

#[cfg(test)]
mod tests {
    use crate::tui::app::test_support::*;

    #[test]
    fn big_h_flips_my_mr_between_draft_and_ready_and_reads_it_again() {
        let mut ready = with_mr(|mr| Mr { mine: true, ..mr });
        assert_eq!(press(&mut ready, "H"), vec![Action::SetReadiness { key: mr_key(), readiness: Readiness::Draft }]);
        let mut draft = with_mr(|mr| Mr { mine: true, draft: true, ..mr });
        assert_eq!(press(&mut draft, "H"), vec![Action::SetReadiness { key: mr_key(), readiness: Readiness::Ready }]);
        draft.apply(Incoming::ReadinessSet { key: mr_key(), readiness: Readiness::Ready });
        let follow = draft.take_actions();
        assert!(follow.contains(&Action::RefreshMr(mr_key())), "{follow:?}");
        assert!(follow.iter().any(|a| matches!(a, Action::LoadQueue { .. })), "the queue row reads again: {follow:?}");
        assert_eq!(draft.live_toast().unwrap().text, "!42 is ready for review");
    }

    #[test]
    fn big_h_says_why_on_an_mr_that_is_not_mine() {
        let mut app = with_mr(|mr| mr);
        assert_eq!(press(&mut app, "H"), vec![]);
        assert_eq!(app.live_toast().unwrap().text, "cannot change !42: it is not yours");
    }

    #[test]
    fn colon_ready_flips_like_big_h() {
        let mut app = with_mr(|mr| Mr { mine: true, ..mr });
        press(&mut app, ":ready");
        assert_eq!(app.handle_key(code(KeyCode::Enter)), vec![Action::SetReadiness { key: mr_key(), readiness: Readiness::Draft }]);
    }
}
