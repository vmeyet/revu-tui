//! A note that is written but not published: one the forge holds as a draft, or one the reviewer just typed.
use super::thread::{Anchor, Side, anchor_of};
use crate::forge::{self, Position};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    /// Set once the forge holds it as a draft.
    pub id: Option<u64>,
    /// Set on a draft written in this session, so the forge's late answer finds it wherever the list moved.
    pub local_id: Option<u64>,
    pub anchor: Option<Anchor>,
    /// What the forge needs to hang the note on a line; None on the MR itself or in a reply.
    pub position: Option<Position>,
    /// The thread this replies to; a reply has no row of its own.
    pub reply_to: Option<String>,
    pub body: String,
    /// Publishing this draft also resolves the thread it replies to.
    pub resolve: bool,
}

/// A draft named whatever its place in the list: by the id this session gave it, else by the forge's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftId {
    Local(u64),
    Forge(u64),
}

impl Draft {
    /// A fresh note on a line, or on the MR when `anchor` is None.
    #[cfg(test)]
    pub fn new(anchor: Option<Anchor>, body: impl Into<String>) -> Self {
        Self { id: None, local_id: None, anchor, position: None, reply_to: None, body: body.into(), resolve: false }
    }

    /// A fresh note on the line `position` names.
    pub fn on(position: Position, body: impl Into<String>) -> Self {
        Self {
            id: None,
            local_id: None,
            anchor: anchor_of(&position),
            position: Some(position),
            reply_to: None,
            body: body.into(),
            resolve: false,
        }
    }

    pub fn reply(thread: &str, body: impl Into<String>) -> Self {
        Self {
            id: None,
            local_id: None,
            anchor: None,
            position: None,
            reply_to: Some(thread.to_owned()),
            body: body.into(),
            resolve: false,
        }
    }

    /// One the forge already holds.
    pub fn held(draft: &forge::Draft) -> Self {
        Self {
            id: Some(draft.id),
            local_id: None,
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

    /// Held by the forge under `id`, with the text the forge holds.
    pub fn held_as(self, id: u64, body: impl Into<String>) -> Self {
        Self { id: Some(id), body: body.into(), ..self }
    }

    /// The same note as the forge would list it: body, thread and line all equal.
    pub fn same_as(&self, other: &Draft) -> bool {
        self.body == other.body && self.reply_to == other.reply_to && self.anchor == other.anchor
    }

    #[cfg(test)]
    pub fn with_id(self, id: u64) -> Self {
        Self { id: Some(id), ..self }
    }

    pub fn with_local_id(self, local_id: u64) -> Self {
        Self { local_id: Some(local_id), ..self }
    }

    pub fn draft_id(&self) -> Option<DraftId> {
        self.local_id.map(DraftId::Local).or(self.id.map(DraftId::Forge))
    }

    pub fn is(&self, id: DraftId) -> bool {
        match id {
            DraftId::Local(local) => self.local_id == Some(local),
            DraftId::Forge(forge) => self.id == Some(forge),
        }
    }

    /// Whether this draft sits in the diff at `(path, side, line)`.
    pub fn is_at(&self, path: &str, side: Side, line: u32) -> bool {
        self.reply_to.is_none() && self.anchor.as_ref().is_some_and(|a| a.path == path && a.side == side && a.line == line)
    }
}

/// The forge's fresh drafts plus the ones this session wrote that it does not hold yet: a refresh never loses a draft.
/// A written draft the forge now lists takes its id instead of showing twice; every draft keeps its local id.
pub fn carry(held: &[Draft], old: &[Draft]) -> Vec<Draft> {
    let unsaved: Vec<&Draft> = old.iter().filter(|d| d.id.is_none()).collect();
    let listed = |draft: &Draft| held.iter().find(|h| h.same_as(draft)).and_then(|h| h.id);
    let local_of = |held: &Draft| old.iter().find(|d| d.id.is_some() && d.id == held.id).and_then(|d| d.local_id);
    let kept = held.iter().filter(|h| !unsaved.iter().any(|d| d.same_as(h))).map(|h| Draft { local_id: local_of(h), ..h.clone() });
    kept.chain(unsaved.iter().map(|d| Draft { id: listed(d), ..(*d).clone() })).collect()
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

    #[test]
    fn a_refresh_keeps_the_drafts_the_forge_does_not_hold_and_adopts_the_ones_it_now_lists() {
        let web = held(3, "from the web", None, None, false);
        let landed = Draft::on(position(), "nit").with_local_id(1);
        let offline = Draft::reply("t1", "agreed").with_local_id(2);
        let saved = held(5, "typo", None, None, false);
        let fresh = [web.clone(), saved.clone(), held(7, "nit", Some(position()), None, false)];
        let carried = carry(&fresh, &[web.clone(), saved.clone().with_local_id(4), landed.clone(), offline.clone()]);
        assert_eq!(
            carried,
            vec![web, saved.with_local_id(4), landed.with_id(7), offline],
            "one copy of the landed draft, and every draft still findable by its local id"
        );
    }
}
