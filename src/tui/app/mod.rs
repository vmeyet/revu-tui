//! The pure state machine: keys in, actions out, incoming answers applied. No clock, no network.
mod apply;
mod ask;
mod brief;
mod commands;
mod feedback;
mod incoming;
mod input;
mod keys;
mod notify;
mod order;
mod pane;
mod pins;
mod pipeline;
pub mod prefetch;
mod queue;
mod review;
mod share;
mod stack;
mod state;
#[cfg(test)]
mod tests;
mod tree;
mod triage;
mod view;
mod write;
mod zen;

pub use apply::Confirm;
pub use ask::{Answer, AnswerState, Part};
pub use brief::{Brief, FileRow, ThreadRow};
pub use feedback::Toast;
pub use order::QueueView;
pub use pane::{Entry, EntryKind};
pub use pins::{MIN_HEIGHT as PIN_MIN_HEIGHT, Pins, pins, settle as settle_with_pins};
pub use pipeline::{Pipeline, Run};
pub use prefetch::Ahead;
pub use queue::{Badge, QueueRow};
pub use review::Open;
pub use share::{Sharing, Stage as ShareStage};
pub use state::{App, Settings};
pub use tree::Tree;
pub use triage::Mark;
pub use write::Publish;

use crate::diff::fold::FoldState;
pub use crate::forge::MrKey;
use crate::forge::{Discussion, Position, QueueMr, Sections};
use crate::review::{Draft, Review};
use chrono::{DateTime, Utc};
use std::collections::{BTreeMap, HashMap};
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
    /// Remember how the queue of `scope` is sorted and grouped.
    SaveQueueView {
        scope: Option<String>,
        view: QueueView,
    },
    /// Paint from the cache at once, then fetch the MR, its diffs and its discussions.
    Open(MrKey),
    /// Load these MRs into the cache ahead of time, quietly: nothing answers.
    Prefetch(Vec<Ahead>),
    RefreshMr(MrKey),
    RefreshDiscussions(MrKey),
    SaveState {
        key: MrKey,
        fold: FoldState,
        /// Viewed files with the fingerprint of the change seen.
        viewed: BTreeMap<String, String>,
        split: bool,
    },
    OpenUrl(String),
    Yank(String),
    /// Pipe `message` to the share target's command; `done` is the toast when it worked.
    Share {
        target: Box<crate::share::Target>,
        message: String,
        done: String,
    },
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
    /// The whole file at the head commit, for the lines around a hunk (`+`).
    LoadFile {
        key: MrKey,
        path: String,
        sha: String,
    },
    /// `v`: the file `path` at commit `sha`, for the reader's program at `line`; `note` says whose file it is.
    View {
        key: MrKey,
        path: String,
        sha: String,
        line: u32,
        note: Option<String>,
    },
    /// A macOS notification: an MR landed in To review.
    Notify {
        title: String,
        body: String,
    },
    /// Commit `suggestion` on the MR's branch `branch`; asked only after the reader said yes.
    Apply {
        key: MrKey,
        branch: String,
        suggestion: Box<crate::forge::Suggestion>,
    },
    /// A picture a comment of `key` points at, `url` as the note wrote it.
    LoadImage {
        key: MrKey,
        url: String,
    },
    /// `p`: the jobs of the CI run on the head commit `head`.
    LoadChecks {
        key: MrKey,
        head: String,
    },
    /// `:set theme=…`: write the theme to the config so the next start keeps it.
    SaveTheme(String),
    /// Ask Claude; `id` names the answer the stream belongs to, `fresh` skips the cached answer.
    Ask {
        key: MrKey,
        id: u64,
        request: Box<crate::ai::anthropic::Ask>,
        fresh: bool,
    },
    /// Ask Jev how urgent and how big a queue MR is.
    Triage(Box<QueueMr>),
    /// Ask Jev whether the open MR waits on me and how risky each file is, at commit `head`.
    Read {
        key: MrKey,
        head: String,
        waits: Option<serde_json::Value>,
        files: Vec<(String, serde_json::Value)>,
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
    /// A question to Claude about `scope`; `concern` makes it the subject of a drafted comment.
    Ask {
        scope: Box<crate::ai::context::Scope>,
        concern: bool,
        target: Box<ask::Target>,
    },
    /// The next question on the conversation of the open answer.
    FollowUp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    Queue,
    Open,
    Poll,
    Local,
    Draft {
        index: usize,
    },
    Publish,
    /// Jev could not answer; the views go on without its marks.
    Triage,
    Resolve {
        thread: String,
        resolved: bool,
    },
    Approve,
    /// The CI run could not be read; the pipeline pane says why.
    Checks,
    /// The suggestion was not committed.
    Apply,
    /// The ready command failed; Ready keeps its last answer.
    Ready,
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
    /// How the queue of `scope` was last sorted and grouped.
    QueueView {
        scope: Option<String>,
        view: QueueView,
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
    /// A file read whole, split into lines, for the context around its hunks.
    File {
        key: MrKey,
        path: String,
        text: String,
    },
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
    /// The suggestion is a commit on `branch` now.
    Applied {
        key: MrKey,
        branch: String,
    },
    /// A picture decoded and ready, or `None` when it could not be fetched or read.
    Image {
        url: String,
        image: Option<image::DynamicImage>,
    },
    /// The CI run of the open MR's head; `None` when nothing ran on it.
    Checks {
        key: MrKey,
        checks: Option<crate::forge::checks::Checks>,
    },
    /// A file ready for the reader's program; the loop hands it the terminal.
    ViewReady {
        key: MrKey,
        view: crate::open::View,
    },
    /// Back from the program, with what went wrong if anything did.
    Viewed {
        view: crate::open::View,
        outcome: Result<(), String>,
    },
    /// What came back from the editor; `None` when the user backed out.
    Composed {
        input: Input,
        text: Option<String>,
    },
    /// A piece of Claude's answer `id` for the MR `key`.
    Answer {
        key: MrKey,
        id: u64,
        part: Part,
    },
    Triaged {
        key: MrKey,
        verdict: crate::ai::triage::Verdict,
    },
    Read {
        key: MrKey,
        head: String,
        reading: crate::ai::triage::Reading,
    },
    Failed {
        what: Failure,
        message: String,
    },
}
