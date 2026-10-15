//! The draft writes on their way to the forge: publishing waits for them, and one the forge refuses puts the draft back.
use super::{Action, App, Input, MrKey};
use crate::review::{Draft, DraftId};

/// The draft writes of one MR the forge has not answered, and whether it refused one since the drafts were last read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DraftWrites {
    running: usize,
    refused: bool,
}

fn draft_write_of(action: &Action) -> Option<&MrKey> {
    match action {
        Action::SaveDraft { key, .. } | Action::UpdateDraft { key, .. } | Action::DeleteDraft { key, .. } => Some(key),
        _ => None,
    }
}

/// `held` back in the list: in its place when a refresh brought it back already, else last.
fn put_back(drafts: Vec<Draft>, held: &Draft) -> Vec<Draft> {
    let is_held = |draft: &Draft| held.id.is_some_and(|id| draft.is(DraftId::Forge(id)));
    if !drafts.iter().any(is_held) {
        return drafts.into_iter().chain([held.clone()]).collect();
    }
    drafts.into_iter().map(|draft| if is_held(&draft) { held.clone() } else { draft }).collect()
}

impl App {
    /// Every action leaves the app through `handle_key` or `take_actions`, which pass it here: its draft writes count until answered.
    pub(super) fn sending(&mut self, actions: Vec<Action>) -> Vec<Action> {
        for key in actions.iter().filter_map(draft_write_of) {
            self.draft_writes.entry(key.clone()).or_default().running += 1;
        }
        actions
    }

    pub(super) fn draft_write_answered(&mut self, key: &MrKey) {
        if let Some(writes) = self.draft_writes.get_mut(key) {
            writes.running = writes.running.saturating_sub(1);
        }
    }

    pub(super) fn drafts_read(&mut self, key: &MrKey) {
        if let Some(writes) = self.draft_writes.get_mut(key) {
            writes.refused = false;
        }
    }

    /// Why publishing must wait: a draft write still on its way, or one the forge refused since the drafts were read.
    pub(super) fn publish_blocked(&self, key: &MrKey) -> Option<&'static str> {
        let writes = self.draft_writes.get(key).copied().unwrap_or_default();
        if writes.running > 0 {
            return Some("a draft change is still on its way · publish again in a moment");
        }
        writes.refused.then_some("a draft change did not reach the forge · r reads the drafts again, then publish")
    }

    /// The draft reads as the forge holds it again; a refused edit's text waits in the box.
    pub(super) fn draft_write_refused(&mut self, key: &MrKey, held: &Draft, wanted: Option<String>, warning: String) {
        self.draft_write_answered(key);
        self.draft_writes.entry(key.clone()).or_default().refused = true;
        self.update_open_of(key, |open| open.with_drafts_changed(|drafts| put_back(drafts, held)));
        self.warn(warning);
        if let (Some(draft), Some(text)) = (held.draft_id(), wanted) {
            self.offer_again(key, Input::EditDraft { draft }, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::tui::app::test_support::*;

    fn publish_keys(app: &mut App) -> Vec<Action> {
        let actions = press(app, "Pp");
        app.handle_key(code(KeyCode::Esc));
        actions
    }

    fn deleted(actions: &[Action]) -> Incoming {
        let [Action::DeleteDraft { key, held, .. }] = actions else { panic!("{actions:?}") };
        Incoming::Failed { what: Failure::DraftDelete { key: key.clone(), held: held.clone() }, message: "offline".into() }
    }

    #[test]
    fn a_delete_the_forge_refused_puts_the_draft_back_and_publishing_waits_for_the_drafts_read_again() {
        let mut app = with_saved_draft();
        press(&mut app, "l");
        let delete = press(&mut app, "d");
        assert_eq!(app.draft_count(), 0);
        app.apply(deleted(&delete));
        assert_eq!(
            app.open.as_ref().unwrap().review.drafts.iter().map(|d| (d.id, d.body.as_str())).collect::<Vec<_>>(),
            [(Some(9), "nit")]
        );
        assert_eq!(app.live_toast().unwrap().text, "draft not deleted: offline · it is back, d deletes it again");
        assert_eq!(publish_keys(&mut app), vec![], "the forge would publish what it still holds");
        assert!(app.live_toast().unwrap().text.contains("r reads the drafts again"));
        let held = app.open.as_ref().unwrap().review.drafts.clone();
        app.apply(Incoming::Review { key: mr_key(), review: Box::new(review().with_drafts(held)), cached: None });
        assert_eq!(publish_keys(&mut app), vec![Action::Publish { key: mr_key(), kind: ReviewKind::Comment, count: 1 }]);
    }

    #[test]
    fn a_reopen_at_the_same_head_keeps_a_draft_put_back_and_counts_as_reading_the_drafts_again() {
        let mut app = with_saved_draft();
        press(&mut app, "l");
        let delete = press(&mut app, "d");
        app.apply(deleted(&delete));
        let held = app.open.as_ref().unwrap().review.drafts.clone();
        app.apply(Incoming::Threads { key: mr_key(), mr: Box::new(mr()), discussions: discussions(), drafts: held.clone() });
        assert_eq!(app.open.as_ref().unwrap().review.drafts, held, "the draft the forge still holds stays");
        assert_eq!(publish_keys(&mut app), vec![Action::Publish { key: mr_key(), kind: ReviewKind::Comment, count: 1 }]);
    }

    #[test]
    fn publishing_waits_while_a_delete_is_on_its_way() {
        let mut app = with_saved_draft();
        press(&mut app, "jjc");
        let save = type_text(&mut app, "second");
        app.apply(saved(&save, 10));
        press(&mut app, "l");
        press(&mut app, "d");
        assert_eq!(publish_keys(&mut app), vec![]);
        assert!(app.live_toast().unwrap().text.contains("still on its way"));
        app.apply(Incoming::DraftChanged { key: mr_key() });
        assert_eq!(publish_keys(&mut app), vec![Action::Publish { key: mr_key(), kind: ReviewKind::Comment, count: 1 }]);
    }

    #[test]
    fn an_edit_the_forge_refused_shows_the_held_text_and_keeps_the_new_one_in_the_box() {
        let mut app = with_saved_draft();
        press(&mut app, "le");
        let update = type_text(&mut app, " (typo)");
        let [Action::UpdateDraft { key, id: 9, draft, held }] = update.as_slice() else { panic!("{update:?}") };
        assert_eq!((draft.body.as_str(), held.body.as_str()), ("nit (typo)", "nit"));
        let wanted = draft.body.clone();
        app.apply(Incoming::Failed {
            what: Failure::DraftUpdate { key: key.clone(), held: held.clone(), wanted },
            message: "offline".into(),
        });
        assert_eq!(app.open.as_ref().unwrap().review.drafts[0].body, "nit", "the forge still holds the old text");
        assert_eq!((app.input_label(), app.buffer.text()), ("edit draft".to_owned(), "nit (typo)"));
        assert!(app.live_toast().unwrap().text.starts_with("draft not changed: offline"));
        let again = app.handle_key(code(KeyCode::Enter));
        assert!(matches!(again.as_slice(), [Action::UpdateDraft { id: 9, draft, .. }] if draft.body == "nit (typo)"), "{again:?}");
    }

    #[test]
    fn an_edit_refused_while_another_box_is_open_waits_for_e_on_that_draft() {
        let mut app = with_saved_draft();
        press(&mut app, "le");
        let update = type_text(&mut app, "!");
        let [Action::UpdateDraft { key, held, draft, .. }] = update.as_slice() else { panic!("{update:?}") };
        let refused = Failure::DraftUpdate { key: key.clone(), held: held.clone(), wanted: draft.body.clone() };
        press(&mut app, "hc");
        let writing = app.input_label();
        assert!(writing.starts_with("new thread"), "{writing}");
        app.apply(Incoming::Failed { what: refused, message: "offline".into() });
        assert_eq!(app.input_label(), writing, "the box being written stays");
        app.handle_key(code(KeyCode::Esc));
        press(&mut app, "le");
        assert_eq!((app.input_label(), app.buffer.text()), ("edit draft".to_owned(), "nit!"));
    }
}
