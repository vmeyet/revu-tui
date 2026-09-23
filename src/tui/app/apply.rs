//! `S` in the thread pane: commit the suggestion of the note under the cursor, after a `y`.
use super::{Action, App, EntryKind, Open};
use crate::forge::{Note, Side, Suggestion};
use crate::review::suggestion;
use crossterm::event::{KeyCode, KeyEvent};

/// A commit waiting for the reader's yes: nothing reaches the branch before it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confirm {
    pub suggestion: Suggestion,
    pub branch: String,
}

impl Confirm {
    pub fn question(&self) -> String {
        format!("commit this suggestion to {} on {}? y commits · any other key cancels", self.suggestion.path, self.branch)
    }
}

impl App {
    /// `S`: the note's suggestion, anchored where its thread is, waits for `y`.
    pub(super) fn apply_here(&mut self) {
        let Some(open) = &self.open else { return };
        match suggestion_here(open) {
            Ok(suggestion) => self.confirm = Some(Confirm { suggestion, branch: open.review.mr.source_branch.clone() }),
            Err(reason) => self.toast(reason),
        }
    }

    pub(super) fn handle_confirm_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let Some(confirm) = self.confirm.take() else { return vec![] };
        let Some(open) = &self.open else { return vec![] };
        if key.code != KeyCode::Char('y') {
            self.toast("not applied");
            return vec![];
        }
        vec![Action::Apply { key: open.key.clone(), branch: confirm.branch, suggestion: Box::new(confirm.suggestion) }]
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
