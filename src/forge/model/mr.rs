//! One MR as the header shows it: its state, pipeline, approvals and how it merges.
use super::{ProjectPath, Sha};
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
    /// I wrote it, as the host that holds it knows me: a queue can mix hosts, and names differ on each.
    #[serde(default)]
    pub(crate) mine: bool,
}

impl Mr {
    /// Why I may not merge this MR from revu, or `None` when I may: it is mine, open, not a draft,
    /// green, without conflicts, and approved by someone with no approval left to give.
    pub(crate) fn merge_refusal(&self) -> Option<String> {
        let approvals = &self.approvals;
        let reason = match () {
            () if !self.mine => "only your own MRs merge from revu".to_owned(),
            () if self.state != MrState::Open => format!("this MR is {}", self.state.word()),
            () if self.draft => "a draft cannot merge: mark it ready first".to_owned(),
            () if self.conflicts => "it has conflicts with its target branch".to_owned(),
            () if self.pipeline.as_ref().is_some_and(|p| p.status == PipelineStatus::Failed) => "its pipeline failed".to_owned(),
            () if approvals.approved_by.is_empty() => "nobody approved it yet".to_owned(),
            () if approvals.approvals_left > 0 => {
                format!("it needs {} more approval{}", approvals.approvals_left, if approvals.approvals_left == 1 { "" } else { "s" })
            }
            () => return None,
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
    pub(crate) fn with_mine(&self, change: ApprovalChange, me: &str) -> Self {
        let approve = change == ApprovalChange::Approve;
        let had = self.approved_by.iter().any(|u| u.username == me);
        let mut approved_by: Vec<User> = self.approved_by.iter().filter(|u| u.username != me).cloned().collect();
        let approvals_left = match (change, had) {
            (ApprovalChange::Approve, false) => self.approvals_left.saturating_sub(1),
            (ApprovalChange::Unapprove, true) => self.approvals_left + 1,
            _ => self.approvals_left,
        };
        if approve {
            approved_by.push(User { id: 0, username: me.to_owned(), name: me.to_owned() });
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
        let mine = before.with_mine(ApprovalChange::Approve, "nina");
        assert_eq!((mine.approved, mine.approvals_left, mine.user_has_approved, mine.approved_by.len()), (true, 0, true, 2));
        assert_eq!(mine.with_mine(ApprovalChange::Approve, "nina"), mine, "approving twice counts once");
        assert_eq!(mine.with_mine(ApprovalChange::Unapprove, "nina"), Approvals { user_has_approved: false, ..before });
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
            mine: true,
        }
    }

    #[test]
    fn only_my_open_green_approved_mrs_may_merge_and_each_refusal_says_why() {
        assert_eq!(mergeable().merge_refusal(), None);
        let cases: [(Mr, &str); 7] = [
            (Mr { mine: false, ..mergeable() }, "only your own MRs merge from revu"),
            (Mr { state: MrState::Merged, ..mergeable() }, "this MR is merged"),
            (Mr { draft: true, ..mergeable() }, "a draft cannot merge: mark it ready first"),
            (Mr { conflicts: true, ..mergeable() }, "it has conflicts with its target branch"),
            (Mr { pipeline: Some(Pipeline { status: PipelineStatus::Failed, web_url: None }), ..mergeable() }, "its pipeline failed"),
            (Mr { approvals: Approvals { approved: true, ..Approvals::default() }, ..mergeable() }, "nobody approved it yet"),
            (Mr { approvals: Approvals { approvals_left: 2, ..mergeable().approvals }, ..mergeable() }, "it needs 2 more approvals"),
        ];
        for (mr, reason) in cases {
            assert_eq!(mr.merge_refusal().as_deref(), Some(reason));
        }
        let running = Mr { pipeline: Some(Pipeline { status: PipelineStatus::Running, web_url: None }), ..mergeable() };
        assert_eq!(running.merge_refusal(), None, "a running pipeline is the forge's call: it may merge when it passes");
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
