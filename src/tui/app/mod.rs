//! The pure state machine: keys in, actions out, incoming answers applied. No clock, no network.
mod brief;
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

pub use brief::Brief;
pub use feedback::Toast;
pub use queue::{Badge, QueueRow};
pub use review::Open;
pub use state::{App, Settings};
pub use write::Publish;

use crate::diff::fold::FoldState;
pub use crate::forge::MrKey;
use crate::forge::{Discussion, Position, Sections};
use crate::review::{Draft, Review};
use chrono::{DateTime, Utc};
use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Focus {
    #[default]
    Queue,
    Review,
    Side,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// The queue for `scope` (a project path, or `None` for every project); `from_cache` paints
    /// the last answer for that scope first, so a switch never shows the other list.
    LoadQueue {
        scope: Option<String>,
        from_cache: bool,
    },
    /// Paint from the cache at once, then fetch the MR, its diffs and its discussions.
    Open(MrKey),
    RefreshMr(MrKey),
    RefreshDiscussions(MrKey),
    SaveState {
        key: MrKey,
        fold: FoldState,
        viewed: BTreeSet<String>,
        split: bool,
    },
    OpenUrl(String),
    Yank(String),
    /// Post the draft at `index` of the open review; the answer carries the id GitLab gave it.
    SaveDraft {
        key: MrKey,
        index: usize,
        draft: Box<Draft>,
    },
    /// The changed draft in full, position included, so GitLab keeps it on its line.
    UpdateDraft {
        key: MrKey,
        id: u64,
        draft: Box<Draft>,
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
        position: Box<Position>,
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
    /// `cached` answers only paint while the fresh one is on its way.
    Queue {
        scope: Option<String>,
        /// Who the forge says I am: it names me when no login stored my name, as with a token variable.
        me: String,
        sections: Sections,
        opened: HashMap<MrKey, DateTime<Utc>>,
        cached: bool,
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
