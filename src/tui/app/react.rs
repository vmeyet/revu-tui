//! `+` in the thread pane: react to the note under the cursor with one of the eight reactions
//! both forges share. The count moves at once; a refusal puts it back.
use super::{Action, App, EntryKind, Failure};
use crate::forge::{Emoji, Note};
use crossterm::event::{KeyCode, KeyEvent};

/// The picker open on note `note` of thread `thread`, `selected` the reaction `enter` gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pick {
    pub thread: String,
    pub note: usize,
    pub selected: usize,
}

impl App {
    /// `+`: the picker on the focused note, or why there is none.
    pub(super) fn open_react(&mut self) {
        let Some(open) = &self.open else { return };
        let Some((conversations, _, Some(focused))) = open.pane_view() else {
            self.toast("open a thread first");
            return;
        };
        let EntryKind::Note(note) = focused.kind else {
            self.toast("a draft is not published yet: nothing to react to");
            return;
        };
        let Some(thread) = conversations.get(focused.conversation).and_then(|c| c.thread.clone()) else { return };
        self.react = Some(Pick { thread, note, selected: 0 });
    }

    pub(super) fn handle_react_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let Some(pick) = self.react.take() else { return vec![] };
        let last = Emoji::ALL.len() - 1;
        let chosen = match key.code {
            KeyCode::Char(c @ '1'..='8') => Some(c as usize - '1' as usize),
            KeyCode::Enter => Some(pick.selected),
            KeyCode::Char('l') | KeyCode::Right => {
                self.react = Some(Pick { selected: (pick.selected + 1).min(last), ..pick.clone() });
                None
            }
            KeyCode::Char('h') | KeyCode::Left => {
                self.react = Some(Pick { selected: pick.selected.saturating_sub(1), ..pick.clone() });
                None
            }
            _ => None,
        };
        chosen.map(|index| self.toggle_reaction(&pick, Emoji::ALL[index])).unwrap_or_default()
    }

    /// Mine on or off, shown at once, then asked of the forge.
    fn toggle_reaction(&mut self, pick: &Pick, emoji: Emoji) -> Vec<Action> {
        let Some(open) = self.open.clone() else { return vec![] };
        let Some(note) = open.review.thread(&pick.thread).and_then(|t| t.notes.get(pick.note)).cloned() else { return vec![] };
        let on = !note.reactions.iter().any(|r| r.emoji == emoji && r.mine);
        self.open = Some(open.with_review(open.review.with_reaction(&pick.thread, pick.note, emoji, on)));
        vec![Action::React { key: open.key.clone(), thread: pick.thread.clone(), index: pick.note, note: Box::new(note), emoji, on }]
    }

    /// The forge refused: the count goes back to what it was.
    pub(super) fn react_failed(&mut self, thread: &str, index: usize, emoji: Emoji, on: bool, message: &str) {
        if let Some(open) = self.open.clone() {
            self.open = Some(open.with_review(open.review.with_reaction(thread, index, emoji, !on)));
        }
        self.warn(format!("no reaction: {message}"));
    }

    /// The picker as the status line shows it: my reactions stand out, the selected one is framed.
    pub fn react_prompt(&self) -> Option<Vec<(String, bool, bool)>> {
        let pick = self.react.as_ref()?;
        let note: Option<&Note> = self.open.as_ref()?.review.thread(&pick.thread).and_then(|t| t.notes.get(pick.note));
        let mine = |emoji: Emoji| note.is_some_and(|n| n.reactions.iter().any(|r| r.emoji == emoji && r.mine));
        Some(
            Emoji::ALL
                .iter()
                .enumerate()
                .map(|(i, &emoji)| {
                    let face = if self.ascii { emoji.text() } else { emoji.glyph() };
                    (format!("{} {face}", i + 1), mine(emoji), i == pick.selected)
                })
                .collect(),
        )
    }
}

/// The reaction the failure was about, for `Failure::React`.
pub fn failure(thread: String, index: usize, emoji: Emoji, on: bool) -> Failure {
    Failure::React { thread, index, emoji, on }
}
