//! A macOS notification when an MR lands in To review while the TUI runs: one per MR, and one
//! notification per queue answer however many arrived together.
use super::{Action, App, MrKey};
use std::collections::HashSet;

/// What To review held, for the scope it was read in: the first answer of a scope only records.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    pub scope: Option<String>,
    pub keys: HashSet<MrKey>,
}

impl App {
    /// After a fresh queue answer: the MRs To review did not hold before, in one notification.
    pub(super) fn notify_new(&mut self) -> Option<Action> {
        let sections = self.sections.as_ref()?;
        let now: HashSet<MrKey> = sections.to_review.iter().map(crate::forge::QueueMr::key).collect();
        let scope = self.scope();
        let before = self.seen.replace(Seen { scope: scope.clone(), keys: now.clone() });
        let before = before.filter(|b| b.scope == scope)?;
        if !self.notify {
            return None;
        }
        let arrived: Vec<_> = sections.to_review.iter().filter(|mr| !before.keys.contains(&mr.key())).collect();
        match arrived.as_slice() {
            [] => None,
            [mr] => Some(Action::Notify {
                title: "revu · to review".into(),
                body: format!("{}{} {} · {}", self.hosts.kind_of(&mr.key()).sigil(), mr.number, mr.title, mr.author),
            }),
            many => Some(Action::Notify {
                title: format!("revu · {} MRs to review", many.len()),
                body: many
                    .iter()
                    .map(|mr| format!("{}{} {}", self.hosts.kind_of(&mr.key()).sigil(), mr.number, mr.title))
                    .collect::<Vec<_>>()
                    .join("\n"),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crate::tui::app::test_support::*;

    fn announce(app: &mut App, sections: crate::forge::Sections) -> Vec<Action> {
        app.apply(Incoming::Queue { scope: app.scope(), me: "nina".into(), sections, opened: HashMap::new(), cached: false });
        app.take_actions().into_iter().filter(|a| matches!(a, Action::Notify { .. })).collect()
    }

    #[test]
    fn an_mr_landing_in_to_review_is_announced_once() {
        let mut app = app();
        let full = sections();
        let mut before = full.clone();
        let newcomer = before.to_review.remove(0);
        assert_eq!(announce(&mut app, before.clone()), vec![], "the first answer only records");
        let notices = announce(&mut app, full.clone());
        let [Action::Notify { title, body }] = notices.as_slice() else { panic!("{notices:?}") };
        assert_eq!(title, "revu · to review");
        assert_eq!(body, &format!("!{} {} · {}", newcomer.number, newcomer.title, newcomer.author));
        assert_eq!(announce(&mut app, full), vec![], "never twice for one MR");
    }

    #[test]
    fn several_arrivals_share_one_notification_and_off_means_off() {
        let mut app = app();
        let full = sections();
        let empty = crate::forge::Sections { to_review: vec![], ..full.clone() };
        announce(&mut app, empty.clone());
        let mut more = full.clone();
        more.to_review.push(crate::forge::QueueMr { number: 77, title: "chore: bump".into(), ..full.to_review[0].clone() });
        let notices = announce(&mut app, more);
        let [Action::Notify { title, body }] = notices.as_slice() else { panic!("{notices:?}") };
        assert_eq!(title, "revu · 2 MRs to review");
        assert_eq!(body.lines().count(), 2);
        let mut quiet = App::new(Settings { notify: false, ..settings() });
        announce(&mut quiet, empty);
        assert_eq!(announce(&mut quiet, full), vec![]);
    }
}
