//! What the app knows about a merge request, whichever forge holds it. Each backend converts its
//! own wire shapes to these at its edge; nothing above `forge` sees a GitLab or GitHub field.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The one way an MR is addressed: the path of its project (`group/sub/project`, `owner/repo`)
/// and its number there.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MrKey {
    /// The host it lives on, when that is not the one `revu` started with: a queue can hold several.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    pub project: String,
    pub number: u64,
}

impl MrKey {
    pub fn new(project: impl Into<String>, number: u64) -> Self {
        Self { host: None, project: project.into(), number }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: u64,
    pub username: String,
    pub name: String,
}

/// One merge request, with what the header and the approvals need.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mr {
    pub project: String,
    pub number: u64,
    pub title: String,
    pub description: String,
    pub state: String,
    pub draft: bool,
    pub author: User,
    pub source_branch: String,
    pub target_branch: String,
    pub web_url: String,
    pub updated_at: DateTime<Utc>,
    pub refs: Refs,
    pub pipeline: Option<Pipeline>,
    /// As the forge counts it; GitLab caps it at `"1000+"`.
    pub changes_count: Option<String>,
    pub conflicts: bool,
    pub reviewers: Vec<User>,
    pub labels: Vec<String>,
    pub approvals: Approvals,
}

/// The diff a review reads: its base, where the branch started (GitLab tells it apart from the
/// base; elsewhere it is the base), and its head. Notes on lines are made against these.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refs {
    pub base: String,
    pub start: String,
    pub head: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pipeline {
    pub status: String,
    pub web_url: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approvals {
    pub approved: bool,
    pub approvals_left: u32,
    pub user_has_approved: bool,
    pub user_can_approve: bool,
    pub approved_by: Vec<User>,
}

/// One changed file: its unified diff body (hunks from the first `@@`) and how it changed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffFile {
    pub diff: String,
    pub old_path: String,
    pub new_path: String,
    /// File modes when the forge gives them; a mode-only change reads as such.
    #[serde(default)]
    pub a_mode: String,
    #[serde(default)]
    pub b_mode: String,
    #[serde(default)]
    pub new_file: bool,
    #[serde(default)]
    pub renamed_file: bool,
    #[serde(default)]
    pub deleted_file: bool,
    #[serde(default)]
    pub generated_file: bool,
    /// The forge withheld the body: too many lines for it to send.
    #[serde(default)]
    pub too_large: bool,
    #[serde(default)]
    pub collapsed: bool,
}

/// A thread as the forge holds it: its notes in order, the first one carrying where it hangs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Discussion {
    pub id: String,
    pub notes: Vec<Note>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub id: u64,
    pub body: String,
    pub author: User,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Written by the forge itself: "added 2 commits", "approved this merge request".
    pub system: bool,
    pub resolvable: bool,
    pub resolved: bool,
    pub position: Option<Position>,
    /// The suggestions the forge can apply for us, by id; GitHub has none, its suggestions live in the text only.
    #[serde(default)]
    pub suggestions: Vec<Applicable>,
}

/// A suggestion to commit on the MR's branch: GitLab applies it by `id`; elsewhere the lines around
/// `line` of `path` are replaced by `text` (`above` lines before it, `below` after).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub id: Option<u64>,
    pub path: String,
    pub line: u32,
    pub above: u32,
    pub below: u32,
    pub text: String,
}

/// A suggestion the forge applies itself when asked by its id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Applicable {
    pub id: u64,
    pub applied: bool,
    /// False once the lines moved under it or the MR closed.
    pub appliable: bool,
}

/// Which side of the diff a line number counts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Side {
    Old,
    New,
}

/// One diff line by its numbers: a removed line has only `old`, an added one only `new`, a
/// context line both.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineRef {
    pub old: Option<u32>,
    pub new: Option<u32>,
}

impl LineRef {
    /// The side a note on this line hangs on: new whenever the line exists there.
    pub fn side(self) -> Side {
        if self.new.is_some() { Side::New } else { Side::Old }
    }

    /// The number on `side()`.
    pub fn number(self) -> Option<u32> {
        self.new.or(self.old)
    }
}

/// Where a note hangs in the diff: one line, or the lines from `start` to `line`, in one file.
/// Each backend builds its own wire shape from this (GitLab's SHAs and `line_range`, GitHub's
/// `side`/`line`/`commit_id`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub refs: Refs,
    pub old_path: String,
    pub new_path: String,
    /// The line the note sits on: the last line of a range.
    pub line: LineRef,
    /// The first line of a range; `None` for a note on one line.
    pub start: Option<LineRef>,
}

impl Position {
    /// The path of the file on the side the note hangs on.
    pub fn path(&self) -> &str {
        match self.line.side() {
            Side::New => &self.new_path,
            Side::Old => &self.old_path,
        }
    }
}

/// A note of mine the forge holds but has not published yet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    pub id: u64,
    pub body: String,
    /// On a line; `None` on the MR itself or in a reply.
    pub position: Option<Position>,
    /// The thread it answers.
    pub reply_to: Option<String>,
    /// Publishing it also resolves the thread it answers.
    pub resolve: bool,
}

/// What a forge needs to hold a new draft, or to replace one whole.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewDraft {
    pub body: String,
    pub position: Option<Position>,
    pub reply_to: Option<String>,
    pub resolve: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_hangs_on_the_new_side_when_it_exists_there() {
        let context = LineRef { old: Some(12), new: Some(12) };
        let removed = LineRef { old: Some(13), new: None };
        let added = LineRef { old: None, new: Some(14) };
        assert_eq!((context.side(), context.number()), (Side::New, Some(12)));
        assert_eq!((removed.side(), removed.number()), (Side::Old, Some(13)));
        assert_eq!((added.side(), added.number()), (Side::New, Some(14)));
    }
}
