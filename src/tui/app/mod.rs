//! The pure state machine: keys in, actions out, incoming answers applied. No clock, no network.
mod apply;
mod ask;
mod brief;
mod commands;
mod compact;
mod feedback;
mod inbox;
mod incoming;
mod input;
mod kept;
mod keys;
mod merge;
mod mouse;
mod notify;
mod order;
mod outline;
mod pane;
mod pin;
mod pins;
mod pipeline;
pub(crate) mod prefetch;
mod prose;
mod queue;
pub(crate) mod quit;
mod react;
mod ready;
mod repeat;
mod review;
mod search;
mod session;
mod share;
mod stack;
mod state;
mod taps;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
mod threads;
#[cfg(test)]
mod timing;
mod tree;
mod triage;
mod usage;
mod view;
mod whats_new;
mod write;
mod writes;
mod zen;

pub(crate) use apply::Confirm;
pub(crate) use ask::{Answer, AnswerSource, AnswerState, Part, PastAnswer};
pub(crate) use brief::{Brief, ThreadRow};
pub(crate) use feedback::Toast;
pub(crate) use inbox::{Spot, progress_bar};
pub(crate) use kept::{Kept, ReviewInputs};
pub(crate) use mouse::{Areas, List, ListRow};
pub(crate) use order::{QueueView, ViewChange, with_member};
pub(crate) use outline::{Entry as OutlineEntry, Outline, PaneLine, Symbols};
pub(crate) use pane::{ConversationList, Pane};
pub(crate) use pin::PinChange;
pub(crate) use pins::{MIN_HEIGHT as PIN_MIN_HEIGHT, Pins, pins, settle as settle_with_pins};
pub(crate) use pipeline::{Pipeline, Run};
pub(crate) use prefetch::Ahead;
pub(crate) use prose::{Prose, Texts, Versions};
pub(crate) use queue::{Badge, QueueRow};
pub(crate) use react::{Pick, Prompt, failure as react_failure};
pub(crate) use review::Open;
pub(crate) use search::{Search, mark as mark_search};
pub(crate) use session::SessionPick;
pub(crate) use share::{Sharing, Stage as ShareStage};
pub(crate) use state::{App, Settings};
pub(crate) use taps::{Tap, TapSpot};
pub(crate) use tree::Tree;
pub(crate) use triage::Mark;
pub(crate) use whats_new::WhatsNew;
pub(crate) use write::Publish;

use crate::diff::fold::FoldState;
pub(crate) use crate::forge::MrKey;
use crate::forge::{
    ApprovalChange, Discussion, Position, ProjectPath, QueueMr, ReactionChange, Readiness, ReviewKind, Sections, Sha, ThreadId,
};
use crate::review::{Draft, Review};
pub(crate) use crate::review::{Entry, EntryKind};
use chrono::{DateTime, Utc};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Focus {
    #[default]
    Queue,
    Review,
    Side,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Action {
    /// The queue for `scope` (a project path, or `None` for every project); `from_cache` paints
    /// the last answer for that scope first, so a switch never shows the other list.
    LoadQueue {
        scope: Option<ProjectPath>,
        from_cache: bool,
    },
    /// Remember one change to how the queue of `scope` is sorted and grouped; `order` counts the changes made.
    ChangeQueueView {
        scope: Option<ProjectPath>,
        change: ViewChange,
        order: u64,
    },
    /// Remember `key` pinned, or unpinned, in the queue of `scope`; `order` counts the changes made.
    ChangePin {
        scope: Option<ProjectPath>,
        key: MrKey,
        change: PinChange,
        order: u64,
    },
    /// Paint from the cache at once, then fetch the MR, its diffs and its discussions.
    Open(MrKey),
    /// Load these MRs into the cache ahead of time, quietly: nothing answers.
    Prefetch(Vec<Ahead>),
    RefreshMr(MrKey),
    /// The minute's poll: the MR alone while its head is still `head`, the whole review once it moved.
    PollMr {
        key: MrKey,
        head: Sha,
    },
    RefreshDiscussions(MrKey),
    SaveState {
        key: MrKey,
        /// Grows with every save: a save finishing after a newer one changes nothing.
        order: u64,
        fold: FoldState,
        /// Viewed files with the fingerprint of the change seen.
        viewed: BTreeMap<String, String>,
        /// Kept so the queue counts the MR's progress without its diff.
        auto_folded: Arc<BTreeSet<String>>,
        side_by_side: bool,
        /// Where the cursor rests, so the MR opens there next time; `None` keeps what is saved.
        spot: Option<Spot>,
    },
    OpenUrl(String),
    /// Put `text` on the clipboard; `done` is the toast when it worked.
    Copy {
        text: String,
        done: String,
    },
    /// Pipe `message` to the share target's command; `done` is the toast when it worked.
    Share {
        target: Box<crate::share::Target>,
        message: String,
        done: String,
    },
    /// Keep `target` as the one last shared to on `host`, so `Y` there offers it first.
    RememberShare {
        host: String,
        target: Option<String>,
    },
    /// Post a draft of the open review; the answer carries it back with the id the forge gave it.
    SaveDraft {
        key: MrKey,
        draft: Box<Draft>,
    },
    /// The changed draft in full, position included, so GitLab keeps it on its line.
    /// `held` is the draft as the forge holds it now: it goes back on screen if the forge refuses.
    UpdateDraft {
        key: MrKey,
        id: u64,
        draft: Box<Draft>,
        held: Box<Draft>,
    },
    DeleteDraft {
        key: MrKey,
        id: u64,
        held: Box<Draft>,
    },
    /// `⌘enter`: the text goes public at once, no draft.
    Post {
        key: MrKey,
        to: Post,
        body: String,
    },
    /// Every draft at once, then the approval when asked.
    Publish {
        key: MrKey,
        kind: ReviewKind,
        count: usize,
    },
    Resolve {
        key: MrKey,
        thread: ThreadId,
        resolved: bool,
    },
    Approve {
        key: MrKey,
        change: ApprovalChange,
    },
    /// The whole file at the head commit, for the lines around a hunk (`+`).
    LoadFile {
        key: MrKey,
        path: String,
        sha: Sha,
    },
    /// `v`: the file `path` at commit `sha`, for the reader's program at `line`; `note` says whose file it is.
    View {
        key: MrKey,
        path: String,
        sha: Sha,
        line: u32,
        note: Option<String>,
    },
    /// A macOS notification: an MR landed in To review.
    Notify {
        title: String,
        body: String,
    },
    /// My reaction `emoji` on `note` (index `index` of `thread`), added or taken off.
    React {
        key: MrKey,
        thread: ThreadId,
        index: usize,
        note: Box<crate::forge::Note>,
        emoji: crate::forge::Emoji,
        change: ReactionChange,
    },
    /// Count one more `emoji` given on `host`, so the picker there offers it sooner.
    CountReaction {
        host: String,
        emoji: crate::forge::Emoji,
    },
    /// Merge the MR as `plan` says while its head is `head`; asked only after the reader said yes.
    Merge {
        key: MrKey,
        head: Sha,
        plan: crate::forge::MergePlan,
    },
    /// Mark my MR a draft, or ready for review.
    SetReadiness {
        key: MrKey,
        readiness: Readiness,
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
        head: Sha,
    },
    /// `O`: the symbols changed in `files`, read at commits `base` and `head`.
    LoadOutline {
        key: MrKey,
        base: Sha,
        head: Sha,
        files: Vec<crate::outline::Sides>,
    },
    /// `v`: the Markdown file of `sides`, read at commits `base` and `head`.
    LoadProse {
        key: MrKey,
        base: Sha,
        head: Sha,
        sides: crate::outline::Sides,
    },
    /// The review apps of the MR of `branch`, `head` telling which run it as it is now.
    LoadDeployments {
        key: MrKey,
        branch: String,
        head: Sha,
    },
    /// `:set theme=…`: write the theme to the config so the next start keeps it.
    SaveTheme(String),
    /// Ask Claude; `id` names the answer the stream belongs to.
    Ask {
        key: MrKey,
        id: u64,
        request: Box<crate::ai::anthropic::Ask>,
        source: AnswerSource,
        /// What the answer is kept under, so `a h` lists it: its title and the commit asked about.
        label: String,
        head: Sha,
    },
    /// `a h`: the answers kept for the MR.
    LoadAnswers(MrKey),
    /// Ask Jev how urgent and how big a queue MR is.
    Triage(Box<QueueMr>),
    /// Ask Jev whether the open MR waits on me and how risky each file is, at commit `head`.
    Read {
        key: MrKey,
        head: Sha,
        waits: Option<serde_json::Value>,
        files: Vec<(String, serde_json::Value)>,
    },
    /// `a g`: find or start the agent session of `mr`, then show it.
    Session {
        mr: Box<crate::agent::Mr>,
        choice: crate::agent::Choice,
    },
    /// Run by the event loop itself, never a background task: the editor takes the terminal.
    Compose {
        input: Input,
        draft: String,
    },
}

impl Action {
    /// A link on its way to the clipboard; the toast spells it out, for a reader whose browser is on another machine.
    pub(super) fn copy_link(url: String) -> Self {
        Self::Copy { done: format!("copied {url}"), text: url }
    }
}

/// Where a comment posted at once goes: a new thread on a line (or range), or the end of a thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Post {
    Thread(Box<Position>),
    Reply(ThreadId),
}

/// What the input row is for while it is open.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Input {
    /// A new note on the line (or range) `position` names.
    Comment {
        position: Box<Position>,
    },
    Reply {
        thread: ThreadId,
    },
    EditDraft {
        draft: crate::review::DraftId,
    },
    /// A question to Claude about `scope`.
    Ask {
        scope: Box<crate::ai::context::Scope>,
        target: Box<ask::Target>,
    },
    /// The next question on the conversation of the open answer.
    FollowUp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    Queue,
    /// Another host did not answer the queue; its rows are the ones it last answered.
    Host,
    /// Opening `key` failed; it matters only while `key` is the MR being opened.
    Open {
        key: MrKey,
    },
    /// Reading the open MR `key` again failed; it matters only while `key` is still open.
    Poll {
        key: MrKey,
    },
    Local,
    /// The draft is not saved; `r` posts it again.
    Draft {
        key: MrKey,
    },
    /// The forge kept the draft as `held`; `wanted` is the text it was to take.
    DraftUpdate {
        key: MrKey,
        held: Box<Draft>,
        wanted: String,
    },
    /// The forge kept the draft `held`.
    DraftDelete {
        key: MrKey,
        held: Box<Draft>,
    },
    Publish,
    /// The comment stayed off the forge; its text waits in the box.
    Post {
        key: MrKey,
        to: Post,
    },
    /// Jev could not answer; the views go on without its marks.
    Triage,
    Resolve {
        thread: ThreadId,
        resolved: bool,
    },
    Approve,
    /// The CI run could not be read; the pipeline pane says why.
    Checks,
    /// The files could not be read; the outline pane says why.
    Outline,
    /// The Markdown file could not be read; the prose view says why.
    Prose,
    /// The suggestion was not committed.
    Apply,
    /// The forge did not merge the MR.
    Merge,
    /// The MR kept its draft state.
    SetReadiness,
    /// The reaction did not reach the forge; its count goes back.
    React {
        thread: ThreadId,
        index: usize,
        emoji: crate::forge::Emoji,
        change: ReactionChange,
    },
    /// The ready command failed; Ready keeps its last answer.
    Ready,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Incoming {
    /// `cached` answers only paint while the fresh one is on its way.
    Queue {
        scope: Option<ProjectPath>,
        /// Who the forge says I am: it names me when no login stored my name, as with a token variable.
        me: String,
        sections: Sections,
        opened: HashMap<MrKey, DateTime<Utc>>,
        cached: bool,
    },
    /// How the queue of `scope` was last sorted and grouped.
    QueueView {
        scope: Option<ProjectPath>,
        view: QueueView,
    },
    /// The MRs pinned in the queue of `scope`, as the cache kept them.
    Pins {
        scope: Option<ProjectPath>,
        pins: BTreeSet<MrKey>,
    },
    /// The forge says these pins of `scope` are merged or closed.
    PinsGone {
        scope: Option<ProjectPath>,
        gone: Vec<MrKey>,
    },
    /// `cached` is how old the cache entry was; `None` means it just came from GitLab.
    Review {
        key: MrKey,
        review: Box<Review>,
        cached: Option<Duration>,
    },
    /// Opening found the painted head still current: the MR, its threads and drafts are fresh, the diff stays.
    Threads {
        key: MrKey,
        mr: Box<crate::forge::Mr>,
        discussions: Vec<Discussion>,
        drafts: Vec<Draft>,
    },
    /// The MR read again at the head already shown: title, state, approvals and the like, the diff unchanged.
    Mr {
        key: MrKey,
        mr: Box<crate::forge::Mr>,
    },
    /// Where the reader left this MR last time, sent once it is painted.
    Resume {
        key: MrKey,
        spot: Spot,
    },
    /// How far each started MR went, for the queue.
    Progress(HashMap<MrKey, crate::review::Progress>),
    Discussions {
        key: MrKey,
        discussions: Vec<Discussion>,
    },
    Done(String),
    /// A file read whole at commit `sha`, split into lines, for the context around its hunks.
    File {
        key: MrKey,
        path: String,
        sha: Sha,
        text: String,
    },
    /// `draft` is the one sent, as it was sent. The forge may fold it into a note it already holds:
    /// `body` is the text it now holds under `id`.
    DraftSaved {
        key: MrKey,
        draft: Box<Draft>,
        id: u64,
        body: String,
    },
    /// The forge took a draft edit or delete.
    DraftChanged {
        key: MrKey,
    },
    Published {
        key: MrKey,
        approval: crate::forge::Approval,
        count: usize,
    },
    Posted {
        key: MrKey,
        to: Post,
    },
    Resolved {
        key: MrKey,
        thread: ThreadId,
        resolved: bool,
    },
    Approved {
        key: MrKey,
        change: ApprovalChange,
    },
    /// The forge merged the MR.
    Merged {
        key: MrKey,
    },
    /// The MR is a draft now, or ready for review.
    ReadinessSet {
        key: MrKey,
        readiness: Readiness,
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
    /// The symbols the open MR changes and the calls between them.
    Outline {
        key: MrKey,
        reading: crate::outline::Reading,
    },
    /// Both sides of the Markdown file the prose view shows.
    Prose {
        key: MrKey,
        sides: crate::outline::Sides,
        versions: Versions,
    },
    /// The answers kept for the MR, newest first.
    PastAnswers {
        key: MrKey,
        answers: Vec<PastAnswer>,
    },
    /// The open MR's review apps, newest deployment of each environment.
    Deployments {
        key: MrKey,
        deployments: Vec<crate::forge::Deployment>,
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
    /// Several agent sessions could be the MR's: the reader picks one, or a new one.
    SessionPick {
        mr: Box<crate::agent::Mr>,
        sessions: Vec<crate::agent::Session>,
    },
    /// The session's program, ready for the terminal; the loop lends it.
    SessionReady(crate::agent::Handoff),
    /// Back from the session `said` names, with what went wrong if anything did.
    SessionLeft {
        said: String,
        outcome: Result<(), String>,
    },
    /// What came back from the editor.
    Composed {
        input: Input,
        edited: crate::tui::compose::Edited,
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
        head: Sha,
        reading: crate::ai::triage::Reading,
    },
    Failed {
        what: Failure,
        message: String,
    },
}
