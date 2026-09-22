//! The pure state machine: keys in, actions out, incoming answers applied. No clock, no network.
mod feedback;
mod incoming;
mod input;
mod keys;
mod queue;
mod review;
mod state;
#[cfg(test)]
mod tests;
mod write;

pub use feedback::Toast;
pub use queue::{Badge, QueueRow};
pub use review::Open;
pub use state::{App, Settings};
pub use write::Publish;

use crate::api::{Discussion, Sections};
use crate::diff::fold::FoldState;
use crate::review::{Draft, Review};
use chrono::{DateTime, Utc};
use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

/// `(project_id, iid)`: the one way an MR is addressed inside the app.
pub type MrKey = (u64, u64);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Focus {
    #[default]
    Queue,
    Review,
    Side,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    LoadQueue,
    /// Paint from the cache at once, then fetch the MR, its diffs and its discussions.
    Open(MrKey),
    RefreshMr(MrKey),
    RefreshDiscussions(MrKey),
    SaveState {
        key: MrKey,
        fold: FoldState,
        viewed: BTreeSet<String>,
    },
    OpenUrl(String),
    Yank(String),
    /// Post the draft at `index` of the open review; the answer carries the id GitLab gave it.
    SaveDraft {
        key: MrKey,
        index: usize,
        draft: Box<Draft>,
    },
    UpdateDraft {
        key: MrKey,
        id: u64,
        body: String,
    },
    DeleteDraft {
        key: MrKey,
        id: u64,
    },
    /// Every draft at once, then the approval when asked.
    Publish {
        key: MrKey,
        approve: bool,
        count: usize,
    },
    Resolve {
        key: MrKey,
        thread: String,
        resolved: bool,
    },
    Approve {
        key: MrKey,
        approve: bool,
    },
    /// Run by the event loop itself, never a background task: the editor takes the terminal.
    Compose {
        input: Input,
        draft: String,
    },
}

/// What the input row is for while it is open.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// A new note on the line (or range) `position` names.
    Comment {
        position: Box<crate::api::Position>,
    },
    Reply {
        thread: String,
    },
    EditDraft {
        index: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    Queue,
    Open,
    Poll,
    Local,
    Draft { index: usize },
    Publish,
    Resolve { thread: String, resolved: bool },
    Approve,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    Queue {
        sections: Sections,
        opened: HashMap<MrKey, DateTime<Utc>>,
    },
    /// `cached` is how old the cache entry was; `None` means it just came from GitLab.
    Review {
        key: MrKey,
        review: Box<Review>,
        cached: Option<Duration>,
    },
    Discussions {
        key: MrKey,
        discussions: Vec<Discussion>,
    },
    Done(String),
    DraftSaved {
        key: MrKey,
        index: usize,
        id: u64,
    },
    Published {
        key: MrKey,
        approved: bool,
        count: usize,
    },
    Resolved {
        key: MrKey,
        thread: String,
        resolved: bool,
    },
    Approved {
        key: MrKey,
        approve: bool,
    },
    /// What came back from the editor; `None` when the user backed out.
    Composed {
        input: Input,
        text: Option<String>,
    },
    Failed {
        what: Failure,
        message: String,
    },
}
