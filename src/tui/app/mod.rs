//! The pure state machine: keys in, actions out, incoming answers applied. No clock, no network.
mod feedback;
mod incoming;
mod keys;
mod queue;
mod review;
mod state;
#[cfg(test)]
mod tests;

pub use feedback::Toast;
pub use queue::{Badge, QueueRow};
pub use review::Open;
pub use state::{App, Settings};

use crate::api::{Discussion, Sections};
use crate::diff::fold::FoldState;
use crate::review::Review;
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Queue,
    Open,
    Poll,
    Local,
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
    Failed {
        what: Failure,
        message: String,
    },
}
