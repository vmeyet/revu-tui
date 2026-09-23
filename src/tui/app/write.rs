//! Everything that changes the MR: drafts, the publish modal, resolving, approving.
use super::{Action, App, Failure, Input, MrKey, Open};
use crate::forge::Position;
use crate::review::{Row, position, suggestion};
use crossterm::event::{KeyCode, KeyEvent};

/// The publish modal: the cursor walks the drafts and ends on the publish row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Publish {
    pub selected: usize,
    pub approve: bool,
    /// Sent, waiting for GitLab: keys are ignored until it answers.
    pub busy: bool,
}

impl App {
    pub(super) fn handle_write_key(&mut self, key: KeyEvent) -> Option<Vec<Action>> {
        let actions = match key.code {
            KeyCode::Char('c') => self.comment_here(),
            KeyCode::Char('C') => self.comment_old_side(),
            KeyCode::Char('V') => {
                self.start_select();
                vec![]
            }
            KeyCode::Char('d') => self.delete_draft_here(),
            KeyCode::Char('E') => self.compose_here(false),
            KeyCode::Char('s') => self.compose_here(true),
            KeyCode::Char('A') => self.toggle_approval(),
            KeyCode::Char('P') => {
                self.open_publish();
                vec![]
            }
            KeyCode::Esc if self.open.as_ref().is_some_and(|o| o.select_from.is_some()) => {
                self.drop_select();
                vec![]
            }
            KeyCode::Char('y') if self.open.as_ref().is_some_and(|o| o.select_from.is_some()) => {
                let text = self.selected_lines().join("\n");
                self.drop_select();
                vec![Action::Yank(text)]
            }
            _ => return None,
        };
        Some(actions)
    }

    fn comment_here(&mut self) -> Vec<Action> {
        match self.position_here() {
            Some(position) => self.open_input(Input::Comment { position: Box::new(position) }, ""),
            None => self.toast("move onto a line first"),
        }
        vec![]
    }

    /// `C` on a changed pair: the note goes on the removed line instead of the added one.
    fn comment_old_side(&mut self) -> Vec<Action> {
        let place = self.open.as_ref().filter(|o| o.select_from.is_none()).and_then(|open| match open.row() {
            Some(Row::Pair { file, hunk, removed, .. }) => position::for_line(&open.review, *file, *hunk, *removed),
            _ => None,
        });
        match place {
            Some(position) => self.open_input(Input::Comment { position: Box::new(position) }, ""),
            None => self.toast("C comments on the old side of a changed pair"),
        }
        vec![]
    }

    fn start_select(&mut self) {
        let Some(open) = &self.open else { return };
        if !matches!(open.row(), Some(Row::Line { .. } | Row::Pair { .. })) {
            self.toast("move onto a line first");
            return;
        }
        self.open = Some(Open { select_from: Some(open.selected), ..open.clone() });
    }

    pub(super) fn drop_select(&mut self) {
        if let Some(open) = &self.open {
            self.open = Some(Open { select_from: None, ..open.clone() });
        }
    }

    /// The position for a note here: the cursor's line, or the `V` range around it.
    fn position_here(&self) -> Option<Position> {
        let open = self.open.as_ref()?;
        let lines: Vec<(usize, usize, usize)> = open
            .selection()
            .filter_map(|i| match open.rows.get(i) {
                Some(Row::Line { file, hunk, index } | Row::Pair { file, hunk, added: index, .. }) => Some((*file, *hunk, *index)),
                _ => None,
            })
            .collect();
        match (lines.first(), lines.last(), open.select_from) {
            (Some(&at), _, None) => position::for_line(&open.review, at.0, at.1, at.2),
            (Some(&first), Some(&last), Some(_)) if first == last => position::for_line(&open.review, first.0, first.1, first.2),
            (Some(&first), Some(&last), Some(_)) => position::for_range(&open.review, first, last),
            _ => None,
        }
    }

    /// The diff lines the selection covers, sign included, as `y` and `s` see them.
    fn selected_lines(&self) -> Vec<String> {
        let Some(open) = &self.open else { return vec![] };
        open.selection()
            .filter_map(|i| match open.rows.get(i) {
                Some(Row::Line { file, hunk, index } | Row::Pair { file, hunk, added: index, .. }) => {
                    let line = &open.review.files[*file].hunks[*hunk].lines[*index];
                    let sign = match line.kind {
                        crate::diff::LineKind::Added => '+',
                        crate::diff::LineKind::Removed => '-',
                        crate::diff::LineKind::Context => ' ',
                    };
                    Some(format!("{sign}{}", line.text))
                }
                _ => None,
            })
            .collect()
    }

    fn draft_here(&self) -> Option<usize> {
        match self.open.as_ref()?.row() {
            Some(Row::Draft { index }) => Some(*index),
            _ => None,
        }
    }

    fn delete_draft_here(&mut self) -> Vec<Action> {
        let Some(index) = self.draft_here() else { return vec![] };
        self.delete_draft(index)
    }

    fn delete_draft(&mut self, index: usize) -> Vec<Action> {
        let Some(open) = self.open.clone() else { return vec![] };
        let Some(draft) = open.review.drafts.get(index).cloned() else { return vec![] };
        let drafts: Vec<_> = open.review.drafts.iter().enumerate().filter(|(i, _)| *i != index).map(|(_, d)| d.clone()).collect();
        self.open = Some(open.with_review(open.review.with_drafts(drafts)));
        if let Some(publish) = &self.publish {
            self.publish = Some(Publish { selected: publish.selected.min(self.draft_count()), ..publish.clone() });
        }
        match draft.id {
            Some(id) => vec![Action::DeleteDraft { key: open.key.clone(), id }],
            None => vec![],
        }
    }

    pub(super) fn edit_draft_here(&mut self) -> bool {
        let Some(index) = self.draft_here() else { return false };
        self.edit_draft(index)
    }

    fn edit_draft(&mut self, index: usize) -> bool {
        let Some(body) = self.open.as_ref().and_then(|o| o.review.drafts.get(index)).map(|d| d.body.clone()) else { return false };
        self.open_input(Input::EditDraft { index }, &body);
        true
    }

    /// `E` writes the note in the editor; `s` starts it as a suggestion block for the selected lines.
    fn compose_here(&mut self, suggestion: bool) -> Vec<Action> {
        if let (Some(index), false, Some(open)) = (self.draft_here(), suggestion, self.open.as_ref()) {
            let body = open.review.drafts[index].body.clone();
            return vec![Action::Compose { input: Input::EditDraft { index }, draft: body }];
        }
        let Some(position) = self.position_here() else {
            self.toast("move onto a line first");
            return vec![];
        };
        let draft = if suggestion {
            let lines = self.selected_lines();
            suggestion::prefill(&lines.iter().map(String::as_str).collect::<Vec<_>>())
        } else {
            String::new()
        };
        vec![Action::Compose { input: Input::Comment { position: Box::new(position) }, draft }]
    }

    pub(super) fn toggle_approval(&mut self) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let approve = !open.review.mr.approvals.user_has_approved;
        vec![Action::Approve { key: open.key.clone(), approve }]
    }

    pub(super) fn draft_count(&self) -> usize {
        self.open.as_ref().map_or(0, |o| o.review.drafts.len())
    }

    pub fn unsaved_drafts(&self) -> usize {
        self.open.as_ref().map_or(0, |o| o.review.drafts.iter().filter(|d| d.id.is_none()).count())
    }

    pub(super) fn open_publish(&mut self) {
        if self.draft_count() == 0 {
            self.toast("no drafts");
            return;
        }
        self.publish = Some(Publish::default());
    }

    pub(super) fn handle_publish_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let Some(publish) = self.publish.clone() else { return vec![] };
        if publish.busy {
            return vec![];
        }
        let last = self.draft_count();
        match key.code {
            KeyCode::Esc => self.publish = None,
            KeyCode::Char('j') | KeyCode::Down => self.publish = Some(Publish { selected: (publish.selected + 1).min(last), ..publish }),
            KeyCode::Char('k') | KeyCode::Up => self.publish = Some(Publish { selected: publish.selected.saturating_sub(1), ..publish }),
            KeyCode::Char('a') => self.publish = Some(Publish { approve: !publish.approve, ..publish }),
            KeyCode::Char('d') if publish.selected < last => return self.delete_draft(publish.selected),
            KeyCode::Char('m') if publish.selected < last => return self.move_to_the_mr(publish.selected),
            KeyCode::Char('e') if publish.selected < last => {
                self.edit_draft(publish.selected);
            }
            KeyCode::Char('p') | KeyCode::Enter => return self.publish_now(),
            _ => {}
        }
        vec![]
    }

    /// A draft whose line is gone becomes a note on the MR: the forge draft is replaced by a fresh one.
    fn move_to_the_mr(&mut self, index: usize) -> Vec<Action> {
        let Some(open) = self.open.clone() else { return vec![] };
        let Some(draft) = open.review.drafts.get(index).cloned() else { return vec![] };
        if draft.anchor.is_none() {
            return vec![];
        }
        let moved = draft.clone().on_the_mr();
        let mut drafts = open.review.drafts.clone();
        drafts[index] = moved.clone();
        self.open = Some(open.with_review(open.review.with_drafts(drafts)));
        let delete = draft.id.map(|id| Action::DeleteDraft { key: open.key.clone(), id });
        let save = Action::SaveDraft { key: open.key.clone(), index, draft: Box::new(moved) };
        delete.into_iter().chain([save]).collect()
    }

    fn publish_now(&mut self) -> Vec<Action> {
        let (Some(open), Some(publish)) = (&self.open, &self.publish) else { return vec![] };
        if self.unsaved_drafts() > 0 {
            self.warn("some drafts are not saved yet · r to retry");
            return vec![];
        }
        if let Some(&index) = open.review.stranded().first() {
            let place =
                open.review.drafts[index].anchor.as_ref().map(|a| format!("{}:{}", a.path.rsplit('/').next().unwrap_or(&a.path), a.line));
            self.publish = Some(Publish { selected: index, ..publish.clone() });
            self.warn(format!("the line of the draft on {} left the diff · m moves it to the MR", place.unwrap_or_default()));
            return vec![];
        }
        let action = Action::Publish { key: open.key.clone(), approve: publish.approve, count: open.review.drafts.len() };
        self.publish = Some(Publish { busy: true, ..publish.clone() });
        vec![action]
    }

    /// Drafts GitLab does not hold yet, posted again; a post that already landed is skipped by the backend.
    pub(super) fn retry_unsaved(&self) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        open.review
            .drafts
            .iter()
            .enumerate()
            .filter(|(_, d)| d.id.is_none())
            .map(|(index, draft)| Action::SaveDraft { key: open.key.clone(), index, draft: Box::new(draft.clone()) })
            .collect()
    }

    pub(super) fn reply_here(&mut self) {
        let Some(thread) = self.open.as_ref().and_then(|o| o.thread.clone()) else { return };
        self.open_input(Input::Reply { thread }, "");
    }

    /// Flipped on screen at once; GitLab's refusal flips it back.
    pub(super) fn toggle_resolved(&mut self) -> Vec<Action> {
        let Some(open) = self.open.clone() else { return vec![] };
        let Some(thread) = open.thread.clone().and_then(|id| open.review.thread(&id).cloned()) else { return vec![] };
        if !thread.resolvable {
            self.toast("this thread cannot be resolved");
            return vec![];
        }
        let resolved = !thread.resolved;
        self.open = Some(open.with_review(open.review.with_resolved(&thread.id, resolved)));
        vec![Action::Resolve { key: open.key.clone(), thread: thread.id, resolved }]
    }

    pub(super) fn apply_draft_saved(&mut self, key: &MrKey, index: usize, id: u64) {
        let Some(open) = self.open.clone().filter(|o| &o.key == key) else { return };
        let mut drafts = open.review.drafts.clone();
        match drafts.get_mut(index) {
            Some(draft) if draft.id.is_none() => draft.id = Some(id),
            _ => return,
        }
        self.open = Some(open.with_review(open.review.with_drafts(drafts)));
    }

    pub(super) fn apply_published(&mut self, key: &MrKey, approved: bool, count: usize) {
        let Some(open) = self.open.clone().filter(|o| &o.key == key) else { return };
        let review = open.review.with_drafts(vec![]);
        self.open = Some(Open { thread: open.thread.clone(), ..open.with_review(review) });
        self.publish = None;
        self.poll.discussions_due = Some(self.now);
        let tail = if approved { " and approved" } else { "" };
        self.toast(format!("published {count} comment{}{tail}", if count == 1 { "" } else { "s" }));
        if approved {
            self.set_approved(key, true);
        }
    }

    pub(super) fn apply_resolved(&mut self, key: &MrKey, thread: &str, resolved: bool) {
        let Some(open) = self.open.clone().filter(|o| &o.key == key) else { return };
        self.open = Some(open.with_review(open.review.with_resolved(thread, resolved)));
    }

    pub(super) fn set_approved(&mut self, key: &MrKey, approve: bool) {
        let Some(open) = self.open.clone().filter(|o| &o.key == key) else { return };
        let mut review = open.review.clone();
        review.mr.approvals.user_has_approved = approve;
        self.open = Some(open.with_review(review));
    }

    pub(super) fn apply_write_failure(&mut self, what: Failure, message: String) {
        match what {
            Failure::Draft { .. } => self.warn(format!("draft not saved: {message} · r to retry")),
            Failure::Publish => {
                if let Some(publish) = &self.publish {
                    self.publish = Some(Publish { busy: false, ..publish.clone() });
                }
                self.warn(format!("not published: {message}"));
            }
            Failure::Resolve { thread, resolved } => {
                if let Some(open) = self.open.clone() {
                    self.open = Some(open.with_review(open.review.with_resolved(&thread, !resolved)));
                }
                self.warn(message);
            }
            Failure::Approve => self.warn(message),
            Failure::Queue | Failure::Open | Failure::Poll | Failure::Local => self.warn(message),
        }
    }
}
