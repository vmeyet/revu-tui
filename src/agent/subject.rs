//! What a session belongs to: an MR on a forge, or on `revu local` the branch of a worktree.
use crate::cache::keys;
use crate::forge::MrKey;
use crate::share::Fields;
use std::path::PathBuf;

/// A session name longer than this is cut: it is a tmux window's name too.
const NAME_CHARS: usize = 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Subject {
    Mr(Mr),
    Branch(Branch),
}

/// What a session is told about its MR.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Mr {
    pub(crate) key: MrKey,
    pub(crate) fields: Fields,
    pub(crate) target: String,
    /// I wrote it: the session may change it, where someone else's only gets a plan.
    pub(crate) mine: bool,
}

/// A branch checked out in a worktree: its sessions are the ones started in that folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Branch {
    /// The worktree's key, which the lanes and the cache folder go by.
    pub(crate) key: MrKey,
    /// The branch, or the short sha of a detached `HEAD`.
    pub(crate) name: String,
    pub(crate) dir: PathBuf,
    /// What its diff compares against.
    pub(crate) target: String,
}

impl Subject {
    pub(crate) fn key(&self) -> &MrKey {
        match self {
            Self::Mr(mr) => &mr.key,
            Self::Branch(branch) => &branch.key,
        }
    }

    /// How toasts name it: `!42`, or the branch.
    pub(crate) fn reference(&self) -> &str {
        match self {
            Self::Mr(mr) => &mr.fields.reference,
            Self::Branch(branch) => &branch.name,
        }
    }

    /// The session may change it: my own MR, or any branch of mine on this machine.
    pub(super) fn is_mine(&self) -> bool {
        match self {
            Self::Mr(mr) => mr.mine,
            Self::Branch(_) => true,
        }
    }

    /// Where revu remembers its session: per MR, or per branch of a worktree, which may check out another one later.
    pub(super) fn cache_key(&self) -> String {
        match self {
            Self::Mr(mr) => keys::session(&mr.key),
            Self::Branch(branch) => keys::branch_session(&branch.key, &branch.name),
        }
    }

    /// `revu: !42 charge cards at checkout` or `revu: feat/checkout`, as the reader names sessions: a scope, then a title.
    pub(super) fn session_name(&self) -> String {
        let name = match self {
            Self::Mr(mr) => format!("revu: {} {}", mr.fields.reference, mr.fields.title),
            Self::Branch(branch) => format!("revu: {}", branch.name),
        };
        shortened(&name, NAME_CHARS)
    }
}

/// `text` as it is when it has at most `chars` characters, else cut to end in `…` at that length.
pub(super) fn shortened(text: &str, chars: usize) -> String {
    if text.chars().count() <= chars {
        return text.to_owned();
    }
    let kept: String = text.chars().take(chars - 1).collect();
    format!("{}…", kept.trim_end())
}

#[cfg(test)]
mod tests {
    use super::super::tests::{branch, mr};
    use super::*;

    #[test]
    fn a_session_is_named_after_the_mr_and_a_long_title_is_cut() {
        assert_eq!(Subject::Mr(mr()).session_name(), "revu: !42 charge cards at checkout");
        let long = Mr { fields: Fields { title: "é".repeat(80), ..mr().fields }, ..mr() };
        let name = Subject::Mr(long).session_name();
        assert_eq!(name.chars().count(), NAME_CHARS);
        assert!(name.starts_with("revu: !42 é") && name.ends_with('…'), "{name}");
    }

    #[test]
    fn a_branch_is_named_after_itself_and_remembered_apart_from_the_next_branch_of_its_worktree() {
        let subject = Subject::Branch(branch());
        assert_eq!((subject.session_name().as_str(), subject.reference()), ("revu: feat/checkout", "feat/checkout"));
        let other = Subject::Branch(Branch { name: "fix/cache".into(), ..branch() });
        assert_ne!(subject.cache_key(), other.cache_key());
        assert!(subject.is_mine() && !Subject::Mr(mr()).is_mine(), "a branch on this machine is mine");
    }
}
