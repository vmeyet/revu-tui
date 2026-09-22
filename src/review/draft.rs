//! A note that is written but not published: GitLab's draft note, or one the reviewer just typed.
use super::thread::{Anchor, Side, anchor_of};
use crate::api::Position;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    /// Set once GitLab holds it as a draft note.
    pub id: Option<u64>,
    pub anchor: Option<Anchor>,
    /// What GitLab needs to hang the note on a line; None on the MR itself or in a reply.
    pub position: Option<Position>,
    /// The thread this replies to; a reply has no row of its own.
    pub reply_to: Option<String>,
    pub body: String,
    /// Publishing this draft also resolves the thread it replies to.
    pub resolve: bool,
}

impl Draft {
    /// A fresh note on a line, or on the MR when `anchor` is None.
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

    /// One of GitLab's own draft notes, from the fields its answer carries.
    pub fn from_note(id: u64, note: String, position: Option<&Position>, discussion_id: Option<String>, resolve: bool) -> Self {
        Self {
            id: Some(id),
            anchor: position.and_then(anchor_of),
            position: position.cloned(),
            reply_to: discussion_id,
            body: note,
            resolve,
        }
    }

    pub fn with_body(self, body: impl Into<String>) -> Self {
        Self { body: body.into(), ..self }
    }

    /// The same note as GitLab would list it: body, thread and line all equal.
    pub fn same_as(&self, other: &Draft) -> bool {
        self.body == other.body && self.reply_to == other.reply_to && self.anchor == other.anchor
    }

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
    use super::*;
    use crate::api::Discussion;
    use crate::api::types::from_fixture;

    fn position() -> Position {
        let diff: Discussion = from_fixture(include_str!("../api/fixtures/diff_note.json"));
        diff.notes[0].position.clone().unwrap()
    }

    #[test]
    fn a_gitlab_draft_note_keeps_its_id_and_anchor() {
        let draft = Draft::from_note(9, "nit".into(), Some(&position()), None, false);
        assert_eq!(draft.id, Some(9));
        assert_eq!(draft.anchor, Some(Anchor { path: "src/pay/charge.rs".into(), side: Side::New, line: 57 }));
        assert!(draft.is_at("src/pay/charge.rs", Side::New, 57));
        assert!(!draft.is_at("src/pay/charge.rs", Side::Old, 57));
    }

    #[test]
    fn a_reply_never_sits_on_a_line_even_when_gitlab_gives_it_a_position() {
        let draft = Draft::from_note(9, "agreed".into(), Some(&position()), Some("6a9c".into()), true);
        assert_eq!(draft.reply_to.as_deref(), Some("6a9c"));
        assert!(draft.resolve);
        assert!(!draft.is_at("src/pay/charge.rs", Side::New, 57));
    }

    #[test]
    fn a_local_draft_has_no_id_until_gitlab_answers() {
        let draft = Draft::new(None, "overall: looks good");
        assert_eq!((draft.id, draft.anchor.clone(), draft.reply_to.clone()), (None, None, None));
        let on = Draft::on(position(), "nit");
        assert!(on.is_at("src/pay/charge.rs", Side::New, 57));
        assert!(on.same_as(&Draft::from_note(4, "nit".into(), Some(&position()), None, false)));
        assert!(!on.same_as(&on.clone().with_body("other")));
        assert_eq!(draft.with_id(3).id, Some(3));
        assert_eq!(Draft::reply("t1", "yes").reply_to.as_deref(), Some("t1"));
    }

    #[test]
    fn an_image_position_gives_no_anchor() {
        let image = Position { position_type: "image".into(), ..position() };
        assert_eq!(Draft::from_note(1, String::new(), Some(&image), None, false).anchor, None);
    }
}
