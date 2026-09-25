//! `+` in the thread pane, or on a diff line with a thread: react to a note with one of the eight reactions
//! both forges share, or on GitLab with any emoji found by name after `/`. The count moves at once;
//! a refusal puts it back.
use super::{Action, App, EntryKind, Failure};
use crate::forge::{Emoji, Note};
use crossterm::event::{KeyCode, KeyEvent};

/// The picker open on note `note` of thread `thread`, `selected` the reaction `enter` gives.
/// With a `search` (GitLab only), the choices are the emoji whose name matches it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pick {
    pub thread: String,
    pub note: usize,
    pub selected: usize,
    pub search: Option<Search>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Search {
    pub query: String,
    pub found: Vec<Emoji>,
}

/// How many choices the status line shows at once.
pub const PAGE: usize = 8;

impl Pick {
    fn on(thread: String, note: usize) -> Self {
        Self { thread, note, selected: 0, search: None }
    }

    fn choices(&self) -> &[Emoji] {
        self.search.as_ref().map_or(&Emoji::ALL, |s| &s.found)
    }
}

impl Search {
    fn of(query: String) -> Self {
        let names = Emoji::ALL.into_iter().chain(Emoji::others()).map(|e| (e.gitlab().to_owned(), e));
        let found = crate::fuzzy::rank(&query, names).into_iter().map(|(_, e)| e).collect();
        Self { query, found }
    }
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
        self.react = Some(Pick::on(thread, note));
    }

    /// `+` on a diff line with a thread: the picker on its first note, as the pane would open it,
    /// with the diff keeping the focus so zen stays on it.
    pub(super) fn open_react_here(&mut self) -> bool {
        let Some(open) = &self.open else { return false };
        let place = open.row().filter(|row| open.is_marked(row)).and_then(|row| open.review.place_of(row));
        let Some(thread) = place.and_then(|place| open.review.conversations(&place).into_iter().find_map(|c| c.thread)) else {
            return false;
        };
        self.react = Some(Pick::on(thread, 0));
        true
    }

    /// Whether the open MR's forge takes any emoji, not only the eight.
    fn any_emoji(&self) -> bool {
        self.open.as_ref().is_some_and(|o| self.hosts.kind_of(&o.key).any_emoji())
    }

    pub(super) fn handle_react_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let Some(pick) = self.react.take() else { return vec![] };
        let last = pick.choices().len().saturating_sub(1);
        let moved = |selected: usize| Some(Pick { selected, ..pick.clone() });
        let (next, chosen) = match (key.code, &pick.search) {
            (KeyCode::Enter, _) => (None, pick.choices().get(pick.selected).copied()),
            (KeyCode::Right, _) | (KeyCode::Char('l'), None) => (moved((pick.selected + 1).min(last)), None),
            (KeyCode::Left, _) | (KeyCode::Char('h'), None) => (moved(pick.selected.saturating_sub(1)), None),
            (KeyCode::Char(c @ '1'..='8'), None) => (None, Some(Emoji::ALL[c as usize - '1' as usize])),
            (KeyCode::Char('/'), None) if self.any_emoji() => {
                (Some(Pick { selected: 0, search: Some(Search::of(String::new())), ..pick.clone() }), None)
            }
            (KeyCode::Backspace, Some(search)) => {
                let shorter = search.query.char_indices().next_back().map(|(end, _)| Search::of(search.query[..end].to_owned()));
                (Some(Pick { selected: 0, search: shorter, ..pick.clone() }), None)
            }
            (KeyCode::Char(c), Some(search)) => {
                (Some(Pick { selected: 0, search: Some(Search::of(format!("{}{c}", search.query))), ..pick.clone() }), None)
            }
            _ => (None, None),
        };
        self.react = next;
        chosen.map(|emoji| self.toggle_reaction(&pick, emoji)).unwrap_or_default()
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

    /// The picker as the status line shows it: the search typed so far, then the page of choices
    /// around the selected one, my reactions standing out, the selected one framed.
    pub fn react_prompt(&self) -> Option<Prompt> {
        let pick = self.react.as_ref()?;
        let note: Option<&Note> = self.open.as_ref()?.review.thread(&pick.thread).and_then(|t| t.notes.get(pick.note));
        let mine = |emoji: Emoji| note.is_some_and(|n| n.reactions.iter().any(|r| r.emoji == emoji && r.mine));
        let first = pick.selected - pick.selected % PAGE;
        let label = |i: usize, emoji: Emoji| {
            let face = if self.ascii { emoji.text() } else { emoji.glyph() };
            match &pick.search {
                None => format!("{} {face}", i + 1),
                Some(_) if self.ascii => face.to_owned(),
                Some(_) => format!("{face} {}", emoji.gitlab()),
            }
        };
        let choices = pick.choices().iter().enumerate().skip(first).take(PAGE);
        Some(Prompt {
            search: pick.search.as_ref().map(|s| s.query.clone()),
            more: self.any_emoji(),
            choices: choices.map(|(i, &emoji)| (label(i, emoji), mine(emoji), i == pick.selected)).collect(),
        })
    }
}

/// What the status line shows while the picker is open.
pub struct Prompt {
    /// The name typed so far, once `/` started a search.
    pub search: Option<String>,
    /// Whether `/` searches every emoji: the forge takes any.
    pub more: bool,
    /// Each choice's label, whether it is mine, and whether it is selected.
    pub choices: Vec<(String, bool, bool)>,
}

/// The reaction the failure was about, for `Failure::React`.
pub fn failure(thread: String, index: usize, emoji: Emoji, on: bool) -> Failure {
    Failure::React { thread, index, emoji, on }
}
