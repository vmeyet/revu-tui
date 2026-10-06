//! Everything that changes the MR: drafts, the publish modal, resolving, approving, merging, draft or ready.
use super::{Action, App, Failure, Input, MrKey, Open};
use crate::forge::Position;
use crate::review::{Draft, Row, position, suggestion};
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
            KeyCode::Char('E') => self.compose_here(false),
            KeyCode::Char('R') => self.toggle_resolved(),
            KeyCode::Char('s') => self.compose_here(true),
            KeyCode::Char('A') => self.toggle_approval(),
            KeyCode::Char('M') => self.merge_here(),
            KeyCode::Char('H') => self.toggle_draft(),
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
        self.update_open(|open| Open { select_from: Some(open.selected), ..open });
    }

    pub(super) fn drop_select(&mut self) {
        self.update_open(|open| Open { select_from: None, ..open });
    }

    /// The position for a note here: the cursor's line, or the `V` range around it.
    pub(super) fn position_here(&self) -> Option<Position> {
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

    /// My draft under the pane's cursor.
    fn draft_here(&self) -> Option<usize> {
        self.open.as_ref()?.focused_draft()
    }

    pub(super) fn delete_draft_here(&mut self) -> Vec<Action> {
        let Some(index) = self.draft_here() else { return vec![] };
        self.delete_draft(index)
    }

    fn delete_draft(&mut self, index: usize) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let Some(draft) = open.review.drafts.get(index) else { return vec![] };
        let delete = draft.id.map(|id| Action::DeleteDraft { key: open.key.clone(), id });
        self.update_open(|open| {
            open.with_drafts_changed(|drafts| drafts.into_iter().enumerate().filter(|(i, _)| *i != index).map(|(_, d)| d).collect())
        });
        if let Some(publish) = &self.publish {
            self.publish = Some(Publish { selected: publish.selected.min(self.draft_count()), ..publish.clone() });
        }
        delete.into_iter().collect()
    }

    pub(super) fn edit_draft_here(&mut self) -> bool {
        let Some(index) = self.draft_here() else { return false };
        self.edit_draft(index)
    }

    fn edit_draft(&mut self, index: usize) -> bool {
        let Some(draft) = self.open.as_ref().and_then(|o| o.review.drafts.get(index)) else { return false };
        let Some(id) = draft.draft_id() else { return false };
        let body = draft.body.clone();
        self.open_input(Input::EditDraft { draft: id }, &body);
        true
    }

    /// `E` in the pane: my draft under the cursor in the editor, else a reply to the thread.
    pub(super) fn compose_draft_here(&mut self) -> Vec<Action> {
        let Some(open) = self.open.as_ref() else { return vec![] };
        if let Some(draft) = open.focused_draft().map(|index| &open.review.drafts[index])
            && let Some(id) = draft.draft_id()
        {
            return vec![Action::Compose { input: Input::EditDraft { draft: id }, draft: draft.body.clone() }];
        }
        match open.focused_thread() {
            Some(thread) => vec![Action::Compose { input: Input::Reply { thread }, draft: String::new() }],
            None => vec![],
        }
    }

    /// `E` writes a new thread in the editor; `s` opens the box with a suggestion block for the selected lines.
    fn compose_here(&mut self, suggestion: bool) -> Vec<Action> {
        let Some(position) = self.position_here() else {
            self.toast("move onto a line first");
            return vec![];
        };
        let input = Input::Comment { position: Box::new(position) };
        if !suggestion {
            return vec![Action::Compose { input, draft: String::new() }];
        }
        let lines = self.selected_lines();
        let draft = suggestion::prefill(&lines.iter().map(String::as_str).collect::<Vec<_>>());
        self.drop_select();
        self.open_input(input, &draft);
        vec![]
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
            KeyCode::Esc | KeyCode::Char('q') => self.publish = None,
            KeyCode::Char('j') | KeyCode::Down => self.publish = Some(Publish { selected: (publish.selected + 1).min(last), ..publish }),
            KeyCode::Char('k') | KeyCode::Up => self.publish = Some(Publish { selected: publish.selected.saturating_sub(1), ..publish }),
            KeyCode::Char('a') => self.publish = Some(Publish { approve: !publish.approve, ..publish }),
            KeyCode::Char('d') if publish.selected < last => return self.delete_draft(publish.selected),
            KeyCode::Char('m') if publish.selected < last => return self.move_to_the_mr(publish.selected),
            KeyCode::Char('e') if publish.selected < last => {
                self.publish = None;
                self.edit_draft(publish.selected);
            }
            KeyCode::Char('p') | KeyCode::Enter => return self.publish_now(),
            _ => {}
        }
        vec![]
    }

    /// A draft whose line is gone becomes a note on the MR: the forge draft is replaced by a fresh one.
    fn move_to_the_mr(&mut self, index: usize) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let Some(draft) = open.review.drafts.get(index).filter(|d| d.anchor.is_some()).cloned() else { return vec![] };
        let key = open.key.clone();
        let delete = draft.id.map(|id| Action::DeleteDraft { key: key.clone(), id });
        let moved = draft.on_the_mr().with_local_id(self.new_local_id());
        let save = Action::SaveDraft { key, draft: Box::new(moved.clone()) };
        self.update_open(|open| open.with_draft_replaced(index, moved));
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
            .filter(|d| d.id.is_none())
            .map(|draft| Action::SaveDraft { key: open.key.clone(), draft: Box::new(draft.clone()) })
            .collect()
    }

    pub(super) fn reply_here(&mut self) {
        let Some(thread) = self.open.as_ref().and_then(Open::focused_thread) else {
            self.toast("r replies to a thread; c starts a new one");
            return;
        };
        self.open_input(Input::Reply { thread }, "");
    }

    /// The thread `R` acts on: the pane's, else the first open one on the cursor's line.
    fn thread_to_resolve(&self) -> Option<String> {
        let open = self.open.as_ref()?;
        if self.focus == super::Focus::Side {
            return open.focused_thread();
        }
        let place = open.review.place_of(open.row()?)?;
        let listed = open.review.conversations(&place);
        listed.into_iter().find_map(|c| c.thread)
    }

    /// Flipped on screen at once; GitLab's refusal flips it back.
    pub(super) fn toggle_resolved(&mut self) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let Some(thread) = self.thread_to_resolve().and_then(|id| open.review.thread(&id).cloned()) else {
            self.toast("no thread here");
            return vec![];
        };
        if !thread.resolvable {
            self.toast("this thread cannot be resolved");
            return vec![];
        }
        let resolved = !thread.resolved;
        let action = Action::Resolve { key: open.key.clone(), thread: thread.id.clone(), resolved };
        self.update_open(|open| open.relaid(|review| review.with_resolved(&thread.id, resolved)));
        vec![action]
    }

    pub(super) fn new_local_id(&mut self) -> u64 {
        self.next_draft += 1;
        self.next_draft
    }

    /// The save's answer finds its draft by local id. A draft gone meanwhile, or held already under another id,
    /// is deleted from the forge so publishing never shows it; one edited meanwhile is updated there.
    /// Any other draft under the same id is the one the forge folded this note into: it goes.
    pub(super) fn apply_draft_saved(&mut self, key: &MrKey, sent: &Draft, id: u64, body: &str) -> Vec<Action> {
        let Some(open) = self.open.as_ref().filter(|o| &o.key == key) else { return vec![] };
        let delete = vec![Action::DeleteDraft { key: key.clone(), id }];
        let Some(draft) = open.review.drafts.iter().find(|d| d.local_id == sent.local_id) else { return delete };
        match draft.id {
            None => {}
            Some(held) if held == id => return vec![],
            Some(_) => return delete,
        }
        let saved = draft.clone().held_as(id, refolded(body, &sent.body, &draft.body));
        let update = (saved.body != body).then(|| Action::UpdateDraft { key: key.clone(), id, draft: Box::new(saved.clone()) });
        self.update_open(|open| {
            open.with_drafts_changed(|drafts| {
                drafts
                    .into_iter()
                    .filter(|d| d.id != Some(id))
                    .map(|d| if d.local_id == sent.local_id { saved.clone() } else { d })
                    .collect()
            })
        });
        update.into_iter().collect()
    }

    pub(super) fn apply_published(&mut self, key: &MrKey, approved: bool, count: usize) {
        if self.open.as_ref().is_none_or(|o| &o.key != key) {
            return;
        }
        self.update_open(|open| open.with_drafts_changed(|_| vec![]));
        self.publish = None;
        self.poll.discussions_due = Some(self.now);
        let tail = if approved { " and approved" } else { "" };
        self.toast(format!("published {count} comment{}{tail}", if count == 1 { "" } else { "s" }));
        if approved {
            self.set_approved(key, true);
        }
        self.offer_next();
        let follow = self.reread(key);
        self.composed.extend(follow);
    }

    pub(super) fn apply_resolved(&mut self, key: &MrKey, thread: &str, resolved: bool) {
        if self.open.as_ref().is_none_or(|o| &o.key != key) {
            return;
        }
        self.update_open(|open| open.relaid(|review| review.with_resolved(thread, resolved)));
        let follow = self.reread(key);
        self.composed.extend(follow);
    }

    pub(super) fn set_approved(&mut self, key: &MrKey, approve: bool) {
        let me = self.me.clone();
        self.update_open_of(key, |open| open.relaid(|review| review.with_approved(approve, &me)));
    }

    pub(super) fn apply_write_failure(&mut self, what: Failure, message: String) {
        match what {
            Failure::Draft => self.warn(format!("draft not saved: {message} · r to retry")),
            Failure::Publish => {
                if let Some(publish) = &self.publish {
                    self.publish = Some(Publish { busy: false, ..publish.clone() });
                }
                self.warn(format!("not published: {message}"));
            }
            Failure::Resolve { thread, resolved } => {
                self.update_open(|open| open.relaid(|review| review.with_resolved(&thread, !resolved)));
                self.warn(message);
            }
            Failure::Post { key, to } => self.post_failed(&key, &to, &message),
            Failure::Approve => self.warn(message),
            Failure::Checks => self.checks_failed(message),
            Failure::Outline => self.settle_outline(super::Symbols::Failed(message)),
            Failure::Prose => self.settle_prose(super::Texts::Failed(message)),
            Failure::Apply => self.warn(format!("not applied: {message}")),
            Failure::Merge => self.warn(format!("not merged: {message}")),
            Failure::SetDraft => self.warn(format!("not changed: {message}")),
            Failure::React { thread, index, emoji, on } => self.react_failed(&thread, index, emoji, on, &message),
            Failure::Queue | Failure::Open | Failure::Poll | Failure::Local | Failure::Triage | Failure::Ready => self.warn(message),
        }
    }
}

/// The text the forge should hold once the note sent as `sent` reads `now`: on GitHub a note on the PR itself
/// sits at the end of one shared review text, so only that tail changes.
fn refolded(held: &str, sent: &str, now: &str) -> String {
    match held.strip_suffix(sent) {
        Some(rest) => format!("{rest}{now}"),
        None => now.to_string(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crate::tui::app::test_support::*;

    #[test]
    fn a_draft_the_forge_folds_into_one_it_already_holds_leaves_one_draft() {
        let mut app = with_review();
        let open = app.open.clone().unwrap();
        let sent = crate::review::Draft::new(None, "Also: docs").with_local_id(2);
        let drafts = vec![crate::review::Draft::new(None, "Two nits.").with_id(209).with_local_id(1), sent.clone()];
        app.open = Some(open.relaid(|review| review.with_drafts(drafts)));
        let follow = app.apply_draft_saved(&mr_key(), &sent, 209, "Two nits.\n\nAlso: docs");
        let held: Vec<(Option<u64>, &str)> = app.open.as_ref().unwrap().review.drafts.iter().map(|d| (d.id, d.body.as_str())).collect();
        assert_eq!(held, [(Some(209), "Two nits.\n\nAlso: docs")], "an edit or a delete of it then keeps both texts");
        assert_eq!(follow, vec![]);
    }

    #[test]
    fn a_folded_draft_edited_while_its_save_ran_updates_only_its_own_text() {
        let mut app = with_review();
        let open = app.open.clone().unwrap();
        let sent = crate::review::Draft::new(None, "Also: docs").with_local_id(2);
        let drafts =
            vec![crate::review::Draft::new(None, "Two nits.").with_id(209).with_local_id(1), sent.clone().with_body("Also: docs, please")];
        app.open = Some(open.relaid(|review| review.with_drafts(drafts)));
        let follow = app.apply_draft_saved(&mr_key(), &sent, 209, "Two nits.\n\nAlso: docs");
        let [Action::UpdateDraft { id: 209, draft, .. }] = follow.as_slice() else { panic!("{follow:?}") };
        assert_eq!(draft.body, "Two nits.\n\nAlso: docs, please", "the other note on the PR stays");
    }

    #[test]
    fn v_selects_a_range_for_c_y_and_esc() {
        let mut app = with_review();
        on_line(&mut app);
        press(&mut app, "Vj");
        let actions = press(&mut app, "y");
        let [Action::Yank(text)] = actions.as_slice() else { panic!("{actions:?}") };
        assert_eq!(text, " pub async fn charge(card: &Card, amount: Money) -> Result<Receipt> {\n-    let client = Client::new();");
        assert_eq!(app.open.as_ref().unwrap().select_from, None, "copying drops the selection");
        press(&mut app, "k");
        press(&mut app, "Vjj");
        let open = app.open.as_ref().unwrap();
        assert_eq!(open.selection().count(), 3, "three lines, nothing between them");
        assert!(open.is_selected(open.selected - 1));
        press(&mut app, "c");
        let actions = type_text(&mut app, "fold these");
        let [Action::SaveDraft { draft, .. }] = actions.as_slice() else { panic!("{actions:?}") };
        let position = draft.position.as_ref().unwrap();
        assert_eq!(position.line.new, Some(13));
        let start = position.start.expect("a range");
        assert_eq!((start.old, position.line.new), (Some(12), Some(13)));
        assert_eq!(app.open.as_ref().unwrap().select_from, None, "sending drops the selection");
        press(&mut app, "Vj");
        app.handle_key(code(KeyCode::Esc));
        assert_eq!(app.open.as_ref().unwrap().select_from, None);
        assert_eq!(app.focus, Focus::Review, "esc dropped the selection, nothing else");
    }

    #[test]
    fn r_in_a_thread_replies_as_a_draft_shown_in_the_pane_not_the_diff() {
        let mut app = with_review();
        press(&mut app, "]N");
        app.handle_key(code(KeyCode::Enter));
        press(&mut app, "r");
        assert_eq!(app.input_label(), "reply to nina");
        let actions = type_text(&mut app, "agreed");
        let [Action::SaveDraft { draft, .. }] = actions.as_slice() else { panic!("{actions:?}") };
        assert_eq!(draft.reply_to.as_deref(), Some("c0ffee00c0ffee00"));
        assert_eq!(app.focus, Focus::Side);
        assert!(render(&mut app, 140, 24).contains("you · unsaved ◇"));
    }

    #[test]
    fn my_approval_shows_at_once_lets_me_merge_and_is_read_back_from_the_forge() {
        let mut app = with_review();
        let approvals = |app: &App| app.open.as_ref().unwrap().review.mr.approvals.clone();
        let before = approvals(&app);
        app.apply(Incoming::Approved { key: mr_key(), approve: true });
        let after = approvals(&app);
        assert!(after.user_has_approved && after.approved_by.iter().any(|u| u.username == "nina"));
        assert_eq!(after.approvals_left, before.approvals_left.saturating_sub(1));
        let screen = render(&mut app, 150, 30);
        assert!(screen.contains(&format!("{} of", before.approved_by.len() + 1)), "the header counts it:\n{screen}");
        let asked = app.take_actions();
        assert!(asked.contains(&Action::RefreshMr(mr_key())), "then the forge's count replaces the guess: {asked:?}");
        app.apply(Incoming::Approved { key: mr_key(), approve: false });
        assert_eq!(approvals(&app), before, "taking it back undoes it");
    }

    #[test]
    fn big_r_flips_resolved_at_once_and_a_refusal_flips_it_back() {
        let mut app = with_review();
        press(&mut app, "]N");
        app.handle_key(code(KeyCode::Enter));
        let id = "c0ffee00c0ffee00".to_owned();
        assert!(app.open.as_ref().unwrap().review.thread(&id).unwrap().resolved);
        let actions = press(&mut app, "R");
        assert_eq!(actions, vec![Action::Resolve { key: mr_key(), thread: id.clone(), resolved: false }]);
        assert!(!app.open.as_ref().unwrap().review.thread(&id).unwrap().resolved);
        app.apply(Incoming::Failed { what: Failure::Resolve { thread: id.clone(), resolved: false }, message: "HTTP 403".into() });
        assert!(app.open.as_ref().unwrap().review.thread(&id).unwrap().resolved, "back to resolved");
        assert!(app.live_toast().unwrap().danger);
        press(&mut app, "R");
        app.apply(Incoming::Resolved { key: mr_key(), thread: id.clone(), resolved: false });
        assert!(!app.open.as_ref().unwrap().review.thread(&id).unwrap().resolved);
    }

    #[test]
    fn e_in_the_pane_edits_my_draft_and_d_deletes_it() {
        let mut app = with_saved_draft();
        press(&mut app, "l");
        assert_eq!(app.open.as_ref().unwrap().focused_draft(), Some(0));
        press(&mut app, "e");
        assert_eq!((app.input_label(), app.buffer.text()), ("edit draft".to_owned(), "nit"));
        let actions = type_text(&mut app, " (typo)");
        let [Action::UpdateDraft { key, id: 9, draft }] = actions.as_slice() else { panic!("{actions:?}") };
        assert_eq!(*key, mr_key());
        assert_eq!((draft.body.as_str(), draft.position.is_some()), ("nit (typo)", true), "the position travels with the edit");
        assert_eq!(app.open.as_ref().unwrap().review.drafts[0].body, "nit (typo)");
        assert_eq!(press(&mut app, "d"), vec![Action::DeleteDraft { key: mr_key(), id: 9 }]);
        assert_eq!(app.draft_count(), 0);
        assert_eq!(marker_here(&app), None, "the line is plain again");
    }

    #[test]
    fn an_unsaved_draft_is_posted_again_by_r_and_deleted_without_a_request() {
        let mut app = with_review();
        on_line(&mut app);
        press(&mut app, "c");
        type_text(&mut app, "nit");
        app.apply(Incoming::Failed { what: Failure::Draft, message: "offline".into() });
        assert!(app.live_toast().unwrap().text.contains("r to retry"));
        let actions = press(&mut app, "r");
        assert!(matches!(actions.as_slice(), [Action::RefreshMr(key), Action::SaveDraft { .. }] if *key == mr_key()), "{actions:?}");
        press(&mut app, "l");
        assert_eq!(press(&mut app, "d"), vec![], "GitLab never had it");
        assert_eq!(app.draft_count(), 0);
    }

    /// The review as the forge lists it once the one draft save in `actions` landed as `id`.
    fn listing(actions: &[Action], id: u64) -> Review {
        let [Action::SaveDraft { draft, .. }] = actions else { panic!("{actions:?}") };
        review().with_drafts(vec![crate::review::Draft { id: Some(id), local_id: None, ..draft.as_ref().clone() }])
    }

    fn draft_writes(actions: Vec<Action>) -> Vec<Action> {
        actions
            .into_iter()
            .filter(|a| matches!(a, Action::SaveDraft { .. } | Action::UpdateDraft { .. } | Action::DeleteDraft { .. }))
            .collect()
    }

    #[test]
    fn a_draft_deleted_while_its_save_is_in_flight_is_deleted_on_the_forge_once_the_save_lands() {
        let mut app = with_review();
        on_line(&mut app);
        press(&mut app, "c");
        let save = type_text(&mut app, "nit");
        press(&mut app, "l");
        assert_eq!(press(&mut app, "d"), vec![], "no id to delete yet");
        app.apply(saved(&save, 9));
        assert_eq!(app.take_actions(), vec![Action::DeleteDraft { key: mr_key(), id: 9 }], "the forge would publish it otherwise");
        assert_eq!(app.draft_count(), 0);
    }

    #[test]
    fn a_save_answered_twice_changes_nothing_and_a_twin_on_the_forge_is_deleted() {
        let mut app = with_review();
        on_line(&mut app);
        press(&mut app, "c");
        let save = type_text(&mut app, "nit");
        app.apply(saved(&save, 9));
        app.apply(saved(&save, 9));
        assert_eq!(app.take_actions(), vec![]);
        app.apply(saved(&save, 11));
        assert_eq!(app.take_actions(), vec![Action::DeleteDraft { key: mr_key(), id: 11 }], "two saves raced and both posted");
        assert_eq!(app.open.as_ref().unwrap().review.drafts[0].id, Some(9));
    }

    #[test]
    fn a_save_lands_on_its_own_draft_after_an_earlier_draft_went() {
        let mut app = with_review();
        on_line(&mut app);
        press(&mut app, "c");
        let first = type_text(&mut app, "nit");
        press(&mut app, "jjc");
        let second = type_text(&mut app, "second");
        press(&mut app, "Pd");
        app.handle_key(code(KeyCode::Esc));
        app.apply(saved(&first, 9));
        assert_eq!(app.take_actions(), vec![Action::DeleteDraft { key: mr_key(), id: 9 }]);
        assert_eq!(app.unsaved_drafts(), 1, "the second draft did not take the first one's id");
        app.apply(saved(&second, 10));
        let drafts = &app.open.as_ref().unwrap().review.drafts;
        assert_eq!((drafts.len(), drafts[0].id, drafts[0].body.as_str()), (1, Some(10), "second"));
    }

    #[test]
    fn an_edit_made_while_the_save_is_in_flight_survives_a_refresh_and_reaches_the_forge() {
        let mut app = with_review();
        on_line(&mut app);
        press(&mut app, "c");
        let save = type_text(&mut app, "nit");
        press(&mut app, "le");
        assert_eq!(type_text(&mut app, " (typo)"), vec![], "no id to update yet");
        app.apply(Incoming::Review { key: mr_key(), review: Box::new(listing(&save, 9)), cached: None });
        app.take_actions();
        app.apply(saved(&save, 9));
        let actions = draft_writes(app.take_actions());
        let [Action::UpdateDraft { id: 9, draft, .. }] = actions.as_slice() else { panic!("{actions:?}") };
        assert_eq!(draft.body, "nit (typo)");
        let drafts = &app.open.as_ref().unwrap().review.drafts;
        assert_eq!((drafts.len(), drafts[0].id, drafts[0].body.as_str()), (1, Some(9), "nit (typo)"), "one draft, the new text");
    }

    #[test]
    fn a_refresh_keeps_a_draft_whose_save_failed_until_the_forge_lists_it() {
        let mut app = with_review();
        on_line(&mut app);
        press(&mut app, "c");
        let save = type_text(&mut app, "nit");
        app.apply(Incoming::Failed { what: Failure::Draft, message: "offline".into() });
        app.apply(Incoming::Review { key: mr_key(), review: Box::new(review()), cached: None });
        assert_eq!((app.draft_count(), app.unsaved_drafts()), (1, 1), "the poll did not lose it");
        app.apply(Incoming::Review { key: mr_key(), review: Box::new(listing(&save, 9)), cached: None });
        assert_eq!((app.draft_count(), app.unsaved_drafts()), (1, 0), "the post had landed: one draft, saved");
        assert_eq!(draft_writes(app.take_actions()), vec![]);
    }

    #[test]
    fn the_publish_modal_walks_the_drafts_toggles_approve_and_publishes() {
        let mut app = with_saved_draft();
        press(&mut app, "jjc");
        let save = type_text(&mut app, "second");
        app.apply(saved(&save, 10));
        press(&mut app, "P");
        let publish = app.publish.clone().unwrap();
        assert_eq!((publish.selected, publish.approve, publish.busy), (0, false, false));
        press(&mut app, "jjj");
        assert_eq!(app.publish.as_ref().unwrap().selected, 2, "stops on the publish row");
        press(&mut app, "a");
        assert!(app.publish.as_ref().unwrap().approve);
        let actions = app.handle_key(code(KeyCode::Enter));
        assert_eq!(actions, vec![Action::Publish { key: mr_key(), approve: true, count: 2 }]);
        assert!(app.publish.as_ref().unwrap().busy);
        assert_eq!(press(&mut app, "a"), vec![], "keys wait for the answer");
        app.apply(Incoming::Published { key: mr_key(), approved: true, count: 2 });
        assert_eq!(app.publish, None);
        assert_eq!(app.draft_count(), 0);
        assert!(app.open.as_ref().unwrap().review.mr.approvals.user_has_approved);
        assert_eq!(app.poll.discussions_due, Some(app.now), "threads refresh at once");
        assert_eq!(app.live_toast().unwrap().text, "published 2 comments and approved");
    }

    #[test]
    fn an_edit_lands_on_its_draft_when_a_refresh_moved_it_in_the_list() {
        let mut app = with_saved_draft();
        press(&mut app, "Pe");
        let nit = app.open.as_ref().unwrap().review.drafts[0].clone();
        let held = |id: u64, body: &str, position| {
            crate::review::Draft::held(&crate::forge::Draft { id, body: body.into(), position, reply_to: None, resolve: false })
        };
        let refreshed = review().with_drafts(vec![held(3, "from the web", None), held(9, "nit", nit.position.clone())]);
        app.apply(Incoming::Review { key: mr_key(), review: Box::new(refreshed), cached: None });
        let actions = type_text(&mut app, "!");
        let drafts = &app.open.as_ref().unwrap().review.drafts;
        assert_eq!((drafts[0].body.as_str(), drafts[1].body.as_str()), ("from the web", "nit!"));
        assert!(matches!(actions.as_slice(), [Action::UpdateDraft { id: 9, .. }]), "{actions:?}");
    }

    #[test]
    fn the_publish_modal_edits_deletes_and_survives_a_failure() {
        let mut app = with_saved_draft();
        press(&mut app, "P");
        press(&mut app, "e");
        assert_eq!(app.input_label(), "edit draft");
        assert!(app.publish.is_none(), "the modal steps aside for the compose box in the pane");
        assert!(app.open.as_ref().unwrap().pane.is_some());
        type_text(&mut app, "!");
        press(&mut app, "Pp");
        app.apply(Incoming::Failed { what: Failure::Publish, message: "HTTP 500".into() });
        assert!(!app.publish.as_ref().unwrap().busy);
        assert_eq!(app.open.as_ref().unwrap().review.drafts[0].body, "nit!");
        assert!(app.live_toast().unwrap().text.contains("not published"));
        assert_eq!(press(&mut app, "d"), vec![Action::DeleteDraft { key: mr_key(), id: 9 }]);
        assert_eq!(app.draft_count(), 0);
        app.handle_key(code(KeyCode::Esc));
        assert_eq!(app.publish, None);
        press(&mut app, "P");
        assert_eq!(app.publish, None);
        assert_eq!(app.live_toast().unwrap().text, "no drafts");
    }

    #[test]
    fn publishing_waits_for_unsaved_drafts() {
        let mut app = with_review();
        on_line(&mut app);
        press(&mut app, "c");
        type_text(&mut app, "nit");
        press(&mut app, "P");
        assert_eq!(press(&mut app, "p"), vec![]);
        assert!(app.live_toast().unwrap().danger);
        assert!(!app.publish.as_ref().unwrap().busy);
    }

    #[test]
    fn big_a_approves_then_unapproves() {
        let mut app = with_review();
        assert_eq!(press(&mut app, "A"), vec![Action::Approve { key: mr_key(), approve: true }]);
        app.apply(Incoming::Approved { key: mr_key(), approve: true });
        assert_eq!(app.live_toast().unwrap().text, "approved");
        assert_eq!(press(&mut app, "A"), vec![Action::Approve { key: mr_key(), approve: false }]);
        app.apply(Incoming::Failed { what: Failure::Approve, message: "you cannot approve this MR".into() });
        assert!(app.live_toast().unwrap().danger);
    }

    #[test]
    fn big_e_and_s_open_the_editor_and_what_comes_back_is_a_draft() {
        let mut app = with_review();
        on_line(&mut app);
        let actions = press(&mut app, "E");
        let [Action::Compose { input: Input::Comment { position }, draft }] = actions.as_slice() else { panic!("{actions:?}") };
        assert!(draft.is_empty() && position.line.new == Some(12));
        assert_eq!(press(&mut app, "Vjs"), vec![], "s opens the compose box, prefilled");
        assert_eq!(app.input_label(), "new thread · charge.rs:12–-13");
        let actions = app.handle_key(ctrl('o'));
        let [Action::Compose { draft, .. }] = actions.as_slice() else { panic!("{actions:?}") };
        assert_eq!(
            draft,
            "```suggestion:-0+1\npub async fn charge(card: &Card, amount: Money) -> Result<Receipt> {\n    let client = Client::new();\n```\n"
        );
        assert!(app.input.is_none(), "the editor takes the text over");
        let input = Input::Comment { position: position.clone() };
        app.apply(Incoming::Composed { input: input.clone(), text: None });
        assert_eq!(app.take_actions(), vec![]);
        assert_eq!(app.draft_count(), 0);
        app.apply(Incoming::Composed { input, text: Some("from the editor".into()) });
        let actions = app.take_actions();
        assert!(matches!(actions.as_slice(), [Action::SaveDraft { .. }]), "{actions:?}");
        assert_eq!(app.open.as_ref().unwrap().review.drafts[0].body, "from the editor");
        press(&mut app, "kl");
        assert_eq!(app.open.as_ref().unwrap().focused_draft(), Some(0));
        let actions = press(&mut app, "E");
        assert!(
            matches!(actions.as_slice(), [Action::Compose { input: Input::EditDraft { draft: crate::review::DraftId::Local(1) }, draft }] if draft == "from the editor")
        );
    }

    #[test]
    fn a_draft_whose_line_left_the_diff_is_named_before_publishing_and_m_moves_it_to_the_mr() {
        let mut app = with_saved_draft();
        let open = app.open.clone().unwrap();
        let path = open.review.drafts[0].anchor.as_ref().unwrap().path.clone();
        let gone = crate::review::Draft {
            id: Some(5),
            anchor: Some(crate::review::Anchor { path, side: crate::review::Side::New, line: 9_999 }),
            ..open.review.drafts[0].clone()
        };
        let drafts = vec![open.review.drafts[0].clone(), gone];
        app.open = Some(open.relaid(|review| review.with_drafts(drafts)));
        press(&mut app, "P");
        assert_eq!(app.handle_key(code(KeyCode::Enter)), vec![], "nothing is sent while a draft hangs on a missing line");
        assert_eq!(app.publish.as_ref().unwrap().selected, 1, "the cursor lands on the stranded draft");
        assert!(app.live_toast().unwrap().text.contains("m moves it to the MR"));
        let actions = press(&mut app, "m");
        let [Action::DeleteDraft { id: 5, .. }, Action::SaveDraft { draft, .. }] = actions.as_slice() else { panic!("{actions:?}") };
        assert_eq!((draft.anchor.clone(), draft.position.clone()), (None, None), "the note now sits on the MR");
        let first = app.open.as_ref().unwrap().review.drafts[0].local_id;
        assert!(draft.local_id.is_some() && draft.local_id != first, "the save's answer finds the moved draft, not its neighbour");
        assert_eq!(app.open.as_ref().unwrap().review.stranded(), [] as [usize; 0]);
    }
}
