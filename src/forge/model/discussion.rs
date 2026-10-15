//! Threads and their notes, suggestions, and my drafts not yet published.
use super::{Position, Reaction, ThreadId, User};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A thread as the forge holds it: its notes in order, the first one carrying where it hangs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Discussion {
    pub(crate) id: ThreadId,
    pub(crate) notes: Vec<Note>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Note {
    pub(crate) id: u64,
    pub(crate) body: String,
    pub(crate) author: User,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
    /// Written by the forge itself: "added 2 commits", "approved this merge request".
    pub(crate) system: bool,
    pub(crate) resolvable: bool,
    pub(crate) resolved: bool,
    pub(crate) position: Option<Position>,
    /// The suggestions the forge can apply for us, by id; GitHub has none, its suggestions live in the text only.
    #[serde(default)]
    pub(crate) suggestions: Vec<Applicable>,
    /// The reactions on it, one entry per emoji that has any.
    #[serde(default)]
    pub(crate) reactions: Vec<Reaction>,
    /// The forge's GraphQL id for the note, which reacting needs: `gid://gitlab/DiffNote/…`, GitHub's node id.
    #[serde(default)]
    pub(crate) node: Option<String>,
}

/// A suggestion to commit on the MR's branch: GitLab applies it by `id`; elsewhere the lines around
/// `line` of `path` are replaced by `text` (`above` lines before it, `below` after).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Suggestion {
    pub(crate) id: Option<u64>,
    pub(crate) path: String,
    pub(crate) line: u32,
    pub(crate) above: u32,
    pub(crate) below: u32,
    pub(crate) text: String,
}

/// A suggestion the forge applies itself when asked by its id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Applicable {
    pub(crate) id: u64,
    pub(crate) applied: bool,
    /// False once the lines moved under it or the MR closed.
    pub(crate) appliable: bool,
}

/// A note of mine the forge holds but has not published yet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Draft {
    pub(crate) id: u64,
    pub(crate) body: String,
    /// On a line; `None` on the MR itself or in a reply.
    pub(crate) position: Option<Position>,
    /// The thread it answers.
    pub(crate) reply_to: Option<ThreadId>,
    /// Publishing it also resolves the thread it answers.
    pub(crate) resolve: bool,
}

/// What a forge needs to hold a new draft, or to replace one whole.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NewDraft {
    pub(crate) body: String,
    pub(crate) position: Option<Position>,
    pub(crate) reply_to: Option<ThreadId>,
    pub(crate) resolve: bool,
}
