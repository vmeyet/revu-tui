//! `S` in the thread pane and `M` in the review: a commit that waits for the reader's `y`.
use super::{Action, App, EntryKind, Open};
use crate::forge::{MergePlan, MrKey, Note, Side, Suggestion};
use crate::review::suggestion;
use crossterm::event::{KeyCode, KeyEvent};

/// A commit waiting for the reader's yes: nothing reaches the branch before it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Confirm {
    /// The suggestion of a note, on the MR's branch.
    Apply { suggestion: Suggestion, branch: String },
    /// The MR `name` (`!42`) into `into`, only while its head is still `head`.
    Merge { key: MrKey, name: String, head: String, into: String, plan: MergePlan },
}

impl Confirm {
    pub fn question(&self) -> String {
        match self {
            Confirm::Apply { suggestion, branch } => {
                format!("commit this suggestion to {} on {branch}? y commits · any other key cancels", suggestion.path)
            }
            Confirm::Merge { name, into, plan, .. } => {
                let branch = if plan.remove_branch { ", delete the branch" } else { "" };
                format!("merge {name} into {into} ({}{branch})? y merges · any other key cancels", plan.method.word())
            }
        }
    }
}

impl App {
    /// `S`: the note's suggestion, anchored where its thread is, waits for `y`.
    pub(super) fn apply_here(&mut self) {
        let Some(open) = &self.open else { return };
        match suggestion_here(open) {
            Ok(suggestion) => self.confirm = Some(Confirm::Apply { suggestion, branch: open.review.mr.source_branch.clone() }),
            Err(reason) => self.toast(reason),
        }
    }

    pub(super) fn handle_confirm_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let Some(confirm) = self.confirm.take() else { return vec![] };
        let Some(open) = &self.open else { return vec![] };
        let yes = key.code == KeyCode::Char('y');
        match confirm {
            Confirm::Apply { .. } | Confirm::Merge { .. } if !yes => {
                self.toast(if matches!(confirm, Confirm::Apply { .. }) { "not applied" } else { "not merged" });
                vec![]
            }
            Confirm::Apply { suggestion, branch } => {
                vec![Action::Apply { key: open.key.clone(), branch, suggestion: Box::new(suggestion) }]
            }
            Confirm::Merge { key, head, plan, .. } => vec![Action::Merge { key, head, plan }],
        }
    }

    /// The branch moved: say so, and read the MR again so the diff shows the new commit.
    pub(super) fn applied(&mut self, branch: &str) -> Vec<Action> {
        self.toast(format!("suggestion committed on {branch}"));
        self.open.as_ref().map(|o| vec![Action::RefreshMr(o.key.clone())]).unwrap_or_default()
    }
}

/// The suggestion of the note under the pane's cursor, or why there is none to apply.
fn suggestion_here(open: &Open) -> Result<Suggestion, &'static str> {
    let (conversations, _, focused) = open.pane_view().ok_or("open a thread first")?;
    let focused = focused.ok_or("open a thread first")?;
    let EntryKind::Note(index) = focused.kind else { return Err("a draft is not published yet: nothing to apply") };
    let thread = conversations.get(focused.conversation).and_then(|c| c.thread.as_deref()).and_then(|id| open.review.thread(id));
    let thread = thread.ok_or("open a thread first")?;
    let note = thread.notes.get(index).ok_or("open a thread first")?;
    let proposal = suggestion::parse(&note.body).ok_or("this note has no suggestion")?;
    let anchor = thread.anchor.as_ref().ok_or("this thread is on the MR, not on a line")?;
    if anchor.side == Side::Old {
        return Err("this suggestion is on a removed line: nothing left to change");
    }
    let applicable = note_applicable(note)?;
    Ok(Suggestion {
        id: applicable,
        path: anchor.path.clone(),
        line: anchor.line,
        above: proposal.above,
        below: proposal.below,
        text: proposal.text,
    })
}

/// GitLab's own id for the suggestion when it lists one; it also says when it cannot apply.
fn note_applicable(note: &Note) -> Result<Option<u64>, &'static str> {
    match note.suggestions.first() {
        None => Ok(None),
        Some(s) if s.applied => Err("this suggestion is already applied"),
        Some(s) if !s.appliable => Err("the forge cannot apply this suggestion any more: the lines moved"),
        Some(s) => Ok(Some(s.id)),
    }
}
