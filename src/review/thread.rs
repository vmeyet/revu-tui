//! A GitLab discussion as the review sees it: where it hangs in the diff, and whether it still can.
use crate::api::{Discussion, Note, Position};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Old,
    New,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Anchor {
    pub path: String,
    pub side: Side,
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Thread {
    pub id: String,
    pub resolvable: bool,
    pub resolved: bool,
    pub anchor: Option<Anchor>,
    /// Anchored, but its line is not in the current diff.
    pub outdated: bool,
    pub notes: Vec<Note>,
}

impl Thread {
    /// None when only system notes remain: nothing a reviewer would read.
    pub fn from_discussion(discussion: Discussion) -> Option<Self> {
        let notes: Vec<Note> = discussion.notes.into_iter().filter(|n| !n.system).collect();
        let first = notes.first()?;
        let anchor = first.position.as_ref().and_then(anchor_of);
        Some(Self {
            id: discussion.id,
            resolvable: first.resolvable,
            resolved: first.resolved.unwrap_or(false),
            anchor,
            outdated: false,
            notes,
        })
    }

    pub fn with_outdated(self, outdated: bool) -> Self {
        Self { outdated, ..self }
    }

    pub fn first(&self) -> &Note {
        &self.notes[0]
    }
}

pub(super) fn anchor_of(position: &Position) -> Option<Anchor> {
    if position.position_type != "text" {
        return None;
    }
    if let (Some(path), Some(line)) = (&position.new_path, position.new_line) {
        return Some(Anchor { path: path.clone(), side: Side::New, line });
    }
    let (path, line) = (position.old_path.as_ref()?, position.old_line?);
    Some(Anchor { path: path.clone(), side: Side::Old, line })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::from_fixture;

    #[test]
    fn diff_note_anchors_on_the_new_side() {
        let thread = Thread::from_discussion(from_fixture(include_str!("../api/fixtures/diff_note.json"))).unwrap();
        assert_eq!(thread.anchor, Some(Anchor { path: "src/pay/charge.rs".into(), side: Side::New, line: 57 }));
        assert!(thread.resolvable && !thread.resolved);
    }

    #[test]
    fn removed_line_note_anchors_on_the_old_side() {
        let thread = Thread::from_discussion(from_fixture(include_str!("fixtures/old_side_note.json"))).unwrap();
        assert_eq!(thread.anchor, Some(Anchor { path: "src/pay/charge.rs".into(), side: Side::Old, line: 13 }));
    }

    #[test]
    fn mr_level_note_has_no_anchor() {
        let thread = Thread::from_discussion(from_fixture(include_str!("../api/fixtures/discussions.json"))).unwrap();
        assert_eq!(thread.anchor, None);
        assert!(!thread.resolvable);
    }

    #[test]
    fn system_notes_are_dropped_and_an_all_system_thread_disappears() {
        let mixed: Discussion = from_fixture(include_str!("fixtures/system_notes.json"));
        let only_system = Discussion { notes: mixed.notes.iter().filter(|n| n.system).cloned().collect(), ..mixed.clone() };
        assert_eq!(Thread::from_discussion(only_system), None);
        let thread = Thread::from_discussion(mixed).unwrap();
        assert_eq!(thread.notes.len(), 1);
        assert_eq!(thread.first().body, "Rebased, please look again.");
    }
}
