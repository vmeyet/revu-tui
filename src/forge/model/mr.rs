//! One MR as the header shows it: its state, pipeline, approvals and how it merges.
use super::{ProjectPath, Sha};
use crate::forge::Me;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct User {
    pub(crate) id: u64,
    pub(crate) username: String,
    pub(crate) name: String,
}

/// One merge request, with what the header and the approvals need.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Mr {
    pub(crate) project: ProjectPath,
    pub(crate) number: u64,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) state: MrState,
    pub(crate) draft: bool,
    pub(crate) author: User,
    pub(crate) source_branch: String,
    pub(crate) target_branch: String,
    pub(crate) web_url: String,
    pub(crate) updated_at: DateTime<Utc>,
    pub(crate) refs: Refs,
    pub(crate) pipeline: Option<Pipeline>,
    /// As the forge counts it; GitLab caps it at `"1000+"`.
    pub(crate) changes_count: Option<String>,
    pub(crate) conflicts: bool,
    pub(crate) reviewers: Vec<User>,
    pub(crate) labels: Vec<String>,
    pub(crate) approvals: Approvals,
    /// How the forge merges it, from its project settings; the confirmation names the method.
    #[serde(default)]
    pub(crate) merge: MergePlan,
    /// Whether the forge would merge it now; an MR cached before revu read it holds `Unknown`.
    #[serde(default)]
    pub(crate) merge_verdict: MergeVerdict,
    /// I wrote it, as the host that holds it knows me: a queue can mix hosts, and names differ on each.
    #[serde(default)]
    pub(crate) mine: bool,
    /// How many uncommitted files a local diff leaves out; a forge has none, so it is never written.
    #[serde(default, skip_serializing)]
    pub(crate) uncommitted: u32,
    /// Why a local diff has nothing to compare: its worktree shares no history with the target. Never on a forge.
    #[serde(default, skip_serializing)]
    pub(crate) no_diff: Option<String>,
}

impl Mr {
    /// Why I may not merge this MR from revu, or `None` when I may: it is mine, open, and the forge says it is mergeable.
    /// Only "mine" is revu's own rule. The forge decides the rest, with its project's rules: what revu
    /// sees of the MR names a reason only when the forge gave no verdict, and never refuses a mergeable MR.
    pub(crate) fn merge_refusal(&self) -> Option<String> {
        if !self.mine {
            return Some("only your own MRs merge from revu".to_owned());
        }
        if self.state != MrState::Open {
            return Some(format!("this MR is {}", self.state.word()));
        }
        let verdict = match self.merge_verdict {
            MergeVerdict::Unknown => self.guessed_verdict().unwrap_or(MergeVerdict::Unknown),
            said => said,
        };
        self.refusal_for(verdict)
    }

    /// What stands in the way as far as revu's own fields show, for an MR the forge gave no verdict on.
    fn guessed_verdict(&self) -> Option<MergeVerdict> {
        let pipeline = self.pipeline.as_ref().map(|p| p.status);
        match () {
            () if self.draft => Some(MergeVerdict::Draft),
            () if self.conflicts => Some(MergeVerdict::Conflicts),
            () if pipeline == Some(PipelineStatus::Failed) => Some(MergeVerdict::PipelineNotPassed),
            () if pipeline.is_some_and(PipelineStatus::is_running) => Some(MergeVerdict::PipelineRunning),
            () if self.approvals.approvals_left > 0 => Some(MergeVerdict::NotApproved),
            () => None,
        }
    }

    /// `verdict` in words the reader can act on, the end of `cannot merge: …`; `None` when it is mergeable.
    fn refusal_for(&self, verdict: MergeVerdict) -> Option<String> {
        let left = self.approvals.approvals_left;
        let reason = match verdict {
            MergeVerdict::Mergeable => return None,
            MergeVerdict::Checking => "the forge is still checking whether it can merge: try again in a moment".to_owned(),
            MergeVerdict::Conflicts => "it has conflicts with its target branch".to_owned(),
            MergeVerdict::Behind => "its branch is behind its target branch: rebase it first".to_owned(),
            MergeVerdict::PipelineNotPassed => "its pipeline must pass first".to_owned(),
            MergeVerdict::PipelineRunning => "its pipeline is still running".to_owned(),
            MergeVerdict::NotApproved if left > 0 => format!("it needs {left} more approval{}", if left == 1 { "" } else { "s" }),
            MergeVerdict::NotApproved => "it is not approved yet".to_owned(),
            MergeVerdict::ChangesRequested => "a reviewer requested changes".to_owned(),
            MergeVerdict::UnresolvedThreads => "its threads must be resolved first".to_owned(),
            MergeVerdict::Draft => "a draft cannot merge: mark it ready first".to_owned(),
            MergeVerdict::NotOpen => "this MR is not open".to_owned(),
            MergeVerdict::NotAllowed => "you are not allowed to merge into this branch".to_owned(),
            MergeVerdict::Blocked => "a rule of its project blocks it: its page on the forge says which".to_owned(),
            MergeVerdict::Unknown => "the forge did not say whether it can merge: merge it from its page".to_owned(),
        };
        Some(reason)
    }

    /// Why I may not mark this MR a draft or ready from revu, or `None` when I may: it is mine and open.
    pub(crate) fn draft_refusal(&self) -> Option<String> {
        match () {
            () if !self.mine => Some("it is not yours".to_owned()),
            () if self.state != MrState::Open => Some(format!("this MR is {}", self.state.word())),
            () => None,
        }
    }
}

/// Where an MR stands; stored as GitLab named it before it had a type, so old caches and `--json` readers keep working.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum MrState {
    #[serde(rename = "opened")]
    Open,
    Merged,
    Closed,
}

impl MrState {
    pub(crate) fn word(self) -> &'static str {
        match self {
            MrState::Open => "open",
            MrState::Merged => "merged",
            MrState::Closed => "closed",
        }
    }
}

/// How a forge merges an MR, as its project allows and prefers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum MergeMethod {
    #[default]
    Merge,
    Squash,
    Rebase,
}

impl MergeMethod {
    pub(crate) fn word(self) -> &'static str {
        match self {
            MergeMethod::Merge => "merge commit",
            MergeMethod::Squash => "squash",
            MergeMethod::Rebase => "rebase",
        }
    }
}

/// What the forge answers to "would you merge this MR now, for me", its project's rules applied.
/// Stored by its word: a word this revu does not know, like none at all, reads as `Unknown`, which is never mergeable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MergeVerdict {
    Mergeable,
    /// The forge has not finished working it out; asked again a moment later, it answers.
    Checking,
    Conflicts,
    /// The source branch misses commits of its target, and the project wants them in first.
    Behind,
    /// A pipeline the project requires has not succeeded: it failed, never ran, or still runs.
    PipelineNotPassed,
    PipelineRunning,
    NotApproved,
    ChangesRequested,
    UnresolvedThreads,
    Draft,
    NotOpen,
    /// I have no right to merge into the target branch.
    NotAllowed,
    /// A rule of the project blocks it: one revu has no word for, or one the forge does not name.
    Blocked,
    #[default]
    #[serde(other)]
    Unknown,
}

/// The method, and whether the forge deletes the source branch afterwards.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MergePlan {
    pub(crate) method: MergeMethod,
    pub(crate) remove_branch: bool,
}

/// The diff a review reads: its base, where the branch started (GitLab tells it apart from the
/// base; elsewhere it is the base), and its head. Notes on lines are made against these.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Refs {
    pub(crate) base: Sha,
    pub(crate) start: Sha,
    pub(crate) head: Sha,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Pipeline {
    pub(crate) status: PipelineStatus,
    pub(crate) web_url: Option<String>,
}

/// Where a pipeline stands. GitLab has all of these; GitHub's check rollup is only ever success,
/// failed or running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PipelineStatus {
    Created,
    WaitingForResource,
    WaitingForCallback,
    Preparing,
    Pending,
    Running,
    Success,
    Failed,
    Canceling,
    Canceled,
    Skipped,
    Manual,
    Scheduled,
}

impl PipelineStatus {
    const ALL: [PipelineStatus; 13] = [
        PipelineStatus::Created,
        PipelineStatus::WaitingForResource,
        PipelineStatus::WaitingForCallback,
        PipelineStatus::Preparing,
        PipelineStatus::Pending,
        PipelineStatus::Running,
        PipelineStatus::Success,
        PipelineStatus::Failed,
        PipelineStatus::Canceling,
        PipelineStatus::Canceled,
        PipelineStatus::Skipped,
        PipelineStatus::Manual,
        PipelineStatus::Scheduled,
    ];

    pub(crate) fn word(self) -> &'static str {
        match self {
            PipelineStatus::Created => "created",
            PipelineStatus::WaitingForResource => "waiting_for_resource",
            PipelineStatus::WaitingForCallback => "waiting_for_callback",
            PipelineStatus::Preparing => "preparing",
            PipelineStatus::Pending => "pending",
            PipelineStatus::Running => "running",
            PipelineStatus::Success => "success",
            PipelineStatus::Failed => "failed",
            PipelineStatus::Canceling => "canceling",
            PipelineStatus::Canceled => "canceled",
            PipelineStatus::Skipped => "skipped",
            PipelineStatus::Manual => "manual",
            PipelineStatus::Scheduled => "scheduled",
        }
    }

    /// The status a word names in either case, as GitLab's REST (`failed`) and GraphQL (`FAILED`)
    /// spell it; a word this list does not know yet is taken for a pipeline still on its way.
    fn from_word(word: &str) -> Self {
        Self::ALL.into_iter().find(|status| status.word().eq_ignore_ascii_case(word)).unwrap_or(PipelineStatus::Pending)
    }

    /// Not done yet: the badge waits on it.
    pub(crate) fn is_running(self) -> bool {
        matches!(
            self,
            PipelineStatus::Created
                | PipelineStatus::WaitingForResource
                | PipelineStatus::WaitingForCallback
                | PipelineStatus::Preparing
                | PipelineStatus::Pending
                | PipelineStatus::Running
        )
    }
}

impl Serialize for PipelineStatus {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.word())
    }
}

impl<'de> Deserialize<'de> for PipelineStatus {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::from_word(&String::deserialize(deserializer)?))
    }
}

/// Where the MR's branch was deployed and can be tried: a review app, a preview, a storybook.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Deployment {
    pub(crate) environment: String,
    pub(crate) url: String,
    /// Whether it runs the MR as it is now, not an older push.
    pub(crate) current: bool,
}

impl Deployment {
    /// The one to try first: a `review/` app, else the first listed.
    pub(crate) fn to_try(all: &[Deployment]) -> Option<&Deployment> {
        all.iter().find(|d| d.environment.starts_with("review/")).or_else(|| all.first())
    }
}

/// What became of the approval asked with a publish; the drafts went public whatever it says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Approval {
    NotAsked,
    Given,
    /// The forge refused it, in words the reader can act on.
    Refused(String),
}

/// What a publish says with the drafts: only them, or an approval too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReviewKind {
    Comment,
    Approve,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ApprovalChange {
    Approve,
    Unapprove,
}

/// Whether an MR is a draft or ready for review.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Readiness {
    Draft,
    Ready,
}

impl Readiness {
    /// The end of `!42 is …`.
    pub(crate) fn described(self) -> &'static str {
        match self {
            Self::Draft => "a draft",
            Self::Ready => "ready for review",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Approvals {
    pub(crate) approved: bool,
    pub(crate) approvals_left: u32,
    pub(crate) user_has_approved: bool,
    pub(crate) user_can_approve: bool,
    pub(crate) approved_by: Vec<User>,
}

impl Approvals {
    /// The approvals once mine, `me`'s, is added or taken back, counted the way the forge will
    /// until its own answer comes; a removal gives back the approval it had taken.
    pub(crate) fn with_mine(&self, change: ApprovalChange, me: &Me) -> Self {
        let approve = change == ApprovalChange::Approve;
        let had = self.approved_by.iter().any(|u| me.is(&u.username));
        let mut approved_by: Vec<User> = self.approved_by.iter().filter(|u| !me.is(&u.username)).cloned().collect();
        let approvals_left = match (change, had) {
            (ApprovalChange::Approve, false) => self.approvals_left.saturating_sub(1),
            (ApprovalChange::Unapprove, true) => self.approvals_left + 1,
            _ => self.approvals_left,
        };
        if approve {
            approved_by.push(User { id: 0, username: me.to_string(), name: me.to_string() });
        }
        Self { approved: approvals_left == 0, approvals_left, user_has_approved: approve, approved_by, ..self.clone() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn my_approval_counts_at_once_and_taking_it_back_undoes_it() {
        let lea = User { id: 1, username: "lea".into(), name: "Lea".into() };
        let before = Approvals { approved: false, approvals_left: 1, approved_by: vec![lea], ..Approvals::default() };
        let mine = before.with_mine(ApprovalChange::Approve, &"nina".into());
        assert_eq!((mine.approved, mine.approvals_left, mine.user_has_approved, mine.approved_by.len()), (true, 0, true, 2));
        assert_eq!(mine.with_mine(ApprovalChange::Approve, &"nina".into()), mine, "approving twice counts once");
        assert_eq!(mine.with_mine(ApprovalChange::Unapprove, &"nina".into()), Approvals { user_has_approved: false, ..before });
    }

    fn mergeable() -> Mr {
        let lea = User { id: 3, username: "lea".into(), name: "Léa".into() };
        Mr {
            project: "acme/widgets".into(),
            number: 42,
            title: "feat: charge cards".into(),
            description: String::new(),
            state: MrState::Open,
            draft: false,
            author: User { id: 5, username: "nina".into(), name: "Nina".into() },
            source_branch: "feat/checkout".into(),
            target_branch: "main".into(),
            web_url: String::new(),
            updated_at: DateTime::default(),
            refs: Refs { base: "a".into(), start: "a".into(), head: "b".into() },
            pipeline: Some(Pipeline { status: PipelineStatus::Success, web_url: None }),
            changes_count: None,
            conflicts: false,
            reviewers: vec![],
            labels: vec![],
            approvals: Approvals { approved: true, approved_by: vec![lea], ..Approvals::default() },
            merge: MergePlan::default(),
            merge_verdict: MergeVerdict::Mergeable,
            mine: true,
            uncommitted: 0,
            no_diff: None,
        }
    }

    fn told(merge_verdict: MergeVerdict) -> Mr {
        Mr { merge_verdict, ..mergeable() }
    }

    fn pipeline(status: PipelineStatus) -> Pipeline {
        Pipeline { status, web_url: None }
    }

    #[test]
    fn each_verdict_of_the_forge_refuses_in_its_own_words() {
        let cases: [(MergeVerdict, &str); 11] = [
            (MergeVerdict::Conflicts, "it has conflicts with its target branch"),
            (MergeVerdict::Behind, "its branch is behind its target branch: rebase it first"),
            (MergeVerdict::PipelineNotPassed, "its pipeline must pass first"),
            (MergeVerdict::PipelineRunning, "its pipeline is still running"),
            (MergeVerdict::NotApproved, "it is not approved yet"),
            (MergeVerdict::ChangesRequested, "a reviewer requested changes"),
            (MergeVerdict::UnresolvedThreads, "its threads must be resolved first"),
            (MergeVerdict::Draft, "a draft cannot merge: mark it ready first"),
            (MergeVerdict::NotOpen, "this MR is not open"),
            (MergeVerdict::NotAllowed, "you are not allowed to merge into this branch"),
            (MergeVerdict::Blocked, "a rule of its project blocks it: its page on the forge says which"),
        ];
        for (verdict, reason) in cases {
            assert_eq!(told(verdict).merge_refusal().as_deref(), Some(reason));
        }
    }

    #[test]
    fn an_mr_the_forge_calls_mergeable_merges_whatever_revu_would_have_guessed() {
        let unapproved =
            Mr { approvals: Approvals::default(), pipeline: Some(pipeline(PipelineStatus::Failed)), ..told(MergeVerdict::Mergeable) };
        assert_eq!(unapproved.merge_refusal(), None, "the project asks for no approval and no green pipeline");
    }

    #[test]
    fn an_mr_that_is_not_mine_is_refused_even_when_the_forge_calls_it_mergeable() {
        let theirs = Mr { mine: false, ..told(MergeVerdict::Mergeable) };
        assert_eq!(theirs.merge_refusal().as_deref(), Some("only your own MRs merge from revu"));
    }

    #[test]
    fn a_forge_still_checking_says_to_try_again_in_a_moment() {
        let reason = told(MergeVerdict::Checking).merge_refusal();
        assert_eq!(reason.as_deref(), Some("the forge is still checking whether it can merge: try again in a moment"));
    }

    #[test]
    fn a_missing_approval_says_how_many_are_left() {
        let two_left = Mr { approvals: Approvals { approvals_left: 2, ..Approvals::default() }, ..told(MergeVerdict::NotApproved) };
        assert_eq!(two_left.merge_refusal().as_deref(), Some("it needs 2 more approvals"));
    }

    #[test]
    fn an_mr_no_longer_open_says_what_became_of_it_whatever_the_verdict() {
        let merged = Mr { state: MrState::Merged, ..told(MergeVerdict::Mergeable) };
        assert_eq!(merged.merge_refusal().as_deref(), Some("this MR is merged"));
    }

    #[test]
    fn an_unknown_verdict_is_never_ready() {
        let reason = told(MergeVerdict::Unknown).merge_refusal();
        assert_eq!(reason.as_deref(), Some("the forge did not say whether it can merge: merge it from its page"));
    }

    #[test]
    fn an_unknown_verdict_takes_its_reason_from_what_revu_sees() {
        let unknown = || told(MergeVerdict::Unknown);
        let cases: [(Mr, &str); 5] = [
            (Mr { draft: true, ..unknown() }, "a draft cannot merge: mark it ready first"),
            (Mr { conflicts: true, ..unknown() }, "it has conflicts with its target branch"),
            (Mr { pipeline: Some(pipeline(PipelineStatus::Failed)), ..unknown() }, "its pipeline must pass first"),
            (Mr { pipeline: Some(pipeline(PipelineStatus::Running)), ..unknown() }, "its pipeline is still running"),
            (Mr { approvals: Approvals { approvals_left: 1, ..Approvals::default() }, ..unknown() }, "it needs 1 more approval"),
        ];
        for (mr, reason) in cases {
            assert_eq!(mr.merge_refusal().as_deref(), Some(reason));
        }
    }

    #[test]
    fn a_blocked_verdict_keeps_the_forges_word_over_what_revu_sees() {
        let blocked = Mr { pipeline: Some(pipeline(PipelineStatus::Failed)), ..told(MergeVerdict::Blocked) };
        assert_eq!(blocked.merge_refusal().as_deref(), Some("a rule of its project blocks it: its page on the forge says which"));
    }

    #[test]
    fn a_verdict_is_stored_by_its_word_and_a_word_not_known_reads_as_unknown() {
        let stored = serde_json::to_string(&[MergeVerdict::Mergeable, MergeVerdict::UnresolvedThreads, MergeVerdict::Unknown]).unwrap();
        assert_eq!(stored, r#"["mergeable","unresolved_threads","unknown"]"#);
        let read: Vec<MergeVerdict> = serde_json::from_str(r#"["behind","in_a_merge_train"]"#).unwrap();
        assert_eq!(read, [MergeVerdict::Behind, MergeVerdict::Unknown]);
    }

    #[test]
    fn only_my_open_mrs_turn_draft_or_ready() {
        assert_eq!(mergeable().draft_refusal(), None);
        assert_eq!(Mr { draft: true, ..mergeable() }.draft_refusal(), None);
        assert_eq!(Mr { mine: false, ..mergeable() }.draft_refusal().as_deref(), Some("it is not yours"));
        assert_eq!(Mr { state: MrState::Closed, ..mergeable() }.draft_refusal().as_deref(), Some("this MR is closed"));
    }

    #[test]
    fn states_are_stored_in_the_words_old_caches_hold() {
        assert_eq!(serde_json::to_string(&[MrState::Open, MrState::Merged, MrState::Closed]).unwrap(), r#"["opened","merged","closed"]"#);
        let read: Vec<PipelineStatus> = serde_json::from_str(r#"["success","FAILED","waiting_for_resource","blocked"]"#).unwrap();
        assert_eq!(read, [PipelineStatus::Success, PipelineStatus::Failed, PipelineStatus::WaitingForResource, PipelineStatus::Pending]);
        assert_eq!(serde_json::to_string(&read).unwrap(), r#"["success","failed","waiting_for_resource","pending"]"#);
        assert!(read[2].is_running() && !read[1].is_running());
    }
}
