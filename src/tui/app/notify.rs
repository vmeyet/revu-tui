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
