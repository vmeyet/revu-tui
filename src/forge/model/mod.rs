//! What the app knows about a merge request, whichever forge holds it. Each backend converts its
//! own wire shapes to these at its edge; nothing above `forge` sees a GitLab or GitHub field.

mod diff;
mod discussion;
mod ids;
mod mr;
mod reaction;

pub(crate) use diff::{DiffFile, FileKind, LineRef, Position, Side};
pub(crate) use discussion::{Applicable, Discussion, Draft, NewDraft, Note, Suggestion};
pub(crate) use ids::{MrKey, ProjectPath, Sha, ThreadId};
pub(crate) use mr::{
    Approval, ApprovalChange, Approvals, Deployment, MergeMethod, MergePlan, Mr, MrState, Pipeline, PipelineStatus, Readiness, Refs,
    ReviewKind, User,
};
pub(crate) use reaction::{Emoji, Reaction, ReactionChange, tally, toggled};
