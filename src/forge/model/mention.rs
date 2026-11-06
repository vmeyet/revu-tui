//! Where someone mentioned me on an MR and I have not answered yet: GitLab's to-do items of that kind.
use super::MrState;
use crate::forge::QueueMr;

/// The forge's id for one pending mention, which marking it done names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct MentionId(pub(crate) u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Mention {
    pub(crate) id: MentionId,
    /// The MR as the mention carries it: enough for a queue row when no section lists it.
    pub(crate) mr: QueueMr,
    pub(crate) state: MrState,
    /// The note that mentions me; `None` when it points at the MR itself.
    pub(crate) note: Option<u64>,
}

impl Mention {
    /// Asked for every row on every draw: compared field by field, so no key is built.
    pub(crate) fn is_on(&self, mr: &QueueMr) -> bool {
        self.mr.number == mr.number && self.mr.project == mr.project && self.mr.host == mr.host
    }
}
