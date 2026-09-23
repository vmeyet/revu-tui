//! A note that is written but not published: one the forge holds as a draft, or one the reviewer just typed.
use super::thread::{Anchor, Side, anchor_of};
use crate::forge::{self, Position};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    /// Set once the forge holds it as a draft.
    pub id: Option<u64>,
    pub anchor: Option<Anchor>,
    /// What the forge needs to hang the note on a line; None on the MR itself or in a reply.
    pub position: Option<Position>,
    /// The thread this replies to; a reply has no row of its own.
    pub reply_to: Option<String>,
    pub body: String,
    /// Publishing this draft also resolves the thread it replies to.
    pub resolve: bool,
}

impl Draft {
    /// A fresh note on a line, or on the MR when `anchor` is None.
    #[cfg(test)]
    pub fn new(anchor: Option<Anchor>, body: impl Into<String>) -> Self {
        Self { id: None, anchor, position: None, reply_to: None, body: body.into(), resolve: false }
    }

    /// A fresh note on the line `position` names.
    pub fn on(position: Position, body: impl Into<String>) -> Self {
        Self { id: None, anchor: anchor_of(&position), position: Some(position), reply_to: None, body: body.into(), resolve: false }
    }

    pub fn reply(thread: &str, body: impl Into<String>) -> Self {
        Self { id: None, anchor: None, position: None, reply_to: Some(thread.to_owned()), body: body.into(), resolve: false }
    }

    /// One the forge already holds.
    pub fn held(draft: &forge::Draft) -> Self {
        Self {
            id: Some(draft.id),
            anchor: draft.position.as_ref().and_then(anchor_of),
            position: draft.position.clone(),
            reply_to: draft.reply_to.clone(),
            body: draft.body.clone(),
            resolve: draft.resolve,
        }
    }

    /// What the forge needs to hold this draft, or to replace it whole.
    pub fn payload(&self) -> forge::NewDraft {
        forge::NewDraft { body: self.body.clone(), position: self.position.clone(), reply_to: self.reply_to.clone(), resolve: self.resolve }
    }

    /// The same text as a note on the MR itself, not yet held by the forge.
    pub fn on_the_mr(self) -> Self {
        Self { id: None, anchor: None, position: None, ..self }
    }

    pub fn with_body(self, body: impl Into<String>) -> Self {
        Self { body: body.into(), ..self }
    }

    /// The same note as the forge would list it: body, thread and line all equal.
    pub fn same_as(&self, other: &Draft) -> bool {
        self.body == other.body && self.reply_to == other.reply_to && self.anchor == other.anchor
    }

    #[cfg(test)]
    pub fn with_id(self, id: u64) -> Self {
        Self { id: Some(id), ..self }
    }

    /// Whether this draft sits in the diff at `(path, side, line)`.
    pub fn is_at(&self, path: &str, side: Side, line: u32) -> bool {
        self.reply_to.is_none() && self.anchor.as_ref().is_some_and(|a| a.path == path && a.side == side && a.line == line)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::forge::gitlab::fixture::discussion;

    fn position() -> Position {
        let diff = discussion(include_str!("../forge/gitlab/fixtures/diff_note.json"));
        diff.notes[0].position.clone().unwrap()
    }

    fn held(id: u64, body: &str, position: Option<Position>, reply_to: Option<&str>, resolve: bool) -> Draft {
        Draft::held(&forge::Draft { id, body: body.into(), position, reply_to: reply_to.map(str::to_owned), resolve })
    }

    #[test]
    fn a_held_draft_keeps_its_id_and_anchor() {
        let draft = held(9, "nit", Some(position()), None, false);
        assert_eq!(draft.id, Some(9));
        assert_eq!(draft.anchor, Some(Anchor { path: "src/pay/charge.rs".into(), side: Side::New, line: 57 }));
        assert!(draft.is_at("src/pay/charge.rs", Side::New, 57));
        assert!(!draft.is_at("src/pay/charge.rs", Side::Old, 57));
    }

    #[test]
    fn a_reply_never_sits_on_a_line_even_when_the_forge_gives_it_a_position() {
        let draft = held(9, "agreed", Some(position()), Some("6a9c"), true);
        assert_eq!(draft.reply_to.as_deref(), Some("6a9c"));
        assert!(draft.resolve);
        assert!(!draft.is_at("src/pay/charge.rs", Side::New, 57));
    }

    #[test]
    fn a_local_draft_has_no_id_until_the_forge_answers() {
        let draft = Draft::new(None, "overall: looks good");
        assert_eq!((draft.id, draft.anchor.clone(), draft.reply_to.clone()), (None, None, None));
        let on = Draft::on(position(), "nit");
        assert!(on.is_at("src/pay/charge.rs", Side::New, 57));
        assert!(on.same_as(&held(4, "nit", Some(position()), None, false)));
        assert!(!on.same_as(&on.clone().with_body("other")));
        assert_eq!(draft.with_id(3).id, Some(3));
        assert_eq!(Draft::reply("t1", "yes").reply_to.as_deref(), Some("t1"));
    }

    #[test]
    fn the_payload_carries_the_whole_draft() {
        let payload = Draft::on(position(), "nit").payload();
        assert_eq!((payload.body.as_str(), payload.position, payload.reply_to, payload.resolve), ("nit", Some(position()), None, false));
    }
}
