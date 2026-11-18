//! Who I am on each host: the name its forge answered, or the one the config remembers until it does.
use super::{MrKey, Queue, QueueMr};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// My username on one host; empty until the config or the forge names me.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct Me(String);

impl Me {
    /// Whether `username` is mine: GitLab and GitHub tell logins apart without case.
    pub(crate) fn is(&self, username: &str) -> bool {
        self.is_known() && self.0.eq_ignore_ascii_case(username)
    }

    pub(crate) fn is_known(&self) -> bool {
        !self.0.is_empty()
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl<T: Into<String>> From<T> for Me {
    fn from(username: T) -> Self {
        Self(username.into())
    }
}

impl std::fmt::Display for Me {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// My username on every host the queue draws from: names differ from one host to the next.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Identities {
    /// Keyed by host as `MrKey::host` names it: `None` for the one `revu` started with.
    names: BTreeMap<Option<String>, Me>,
}

/// A host whose forge names me otherwise than the name held for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Renamed {
    pub(crate) host: Option<String>,
    pub(crate) me: Me,
}

static UNKNOWN: Me = Me(String::new());

impl Identities {
    pub(crate) fn on_main(main: Me) -> Self {
        Self { names: BTreeMap::from([(None, main)]) }
    }

    pub(crate) fn with_host(mut self, host: &str, me: Me) -> Self {
        self.names.insert(Some(host.to_owned()), me);
        self
    }

    /// Who each queue's forge said I am; `others` are the queues of the other hosts.
    pub(crate) fn answered(main: &Queue, others: &[Queue]) -> Self {
        let named = others.iter().filter_map(|queue| Some((queue.host.as_deref()?, &queue.me)));
        named.fold(Self::on_main(main.me.clone()), |all, (host, me)| all.with_host(host, me.clone()))
    }

    pub(crate) fn main(&self) -> &Me {
        self.on_host(None)
    }

    /// Who I am on the host `key` lives on.
    pub(crate) fn on(&self, key: &MrKey) -> &Me {
        self.on_host(key.host.as_deref())
    }

    /// Who I am on the host the queue row came from.
    pub(crate) fn on_row(&self, mr: &QueueMr) -> &Me {
        self.on_host(mr.host.as_deref())
    }

    /// A host the queue does not draw from, as a worktree's, reads as the main one.
    fn on_host(&self, host: Option<&str>) -> &Me {
        let held = |wanted: Option<&str>| self.names.iter().find(|(host, _)| host.as_deref() == wanted).map(|(_, me)| me);
        held(host).or_else(|| held(None)).unwrap_or(&UNKNOWN)
    }

    /// These names where one is known, the names of `rest` for every other host.
    pub(crate) fn over(&self, rest: &Self) -> Self {
        let (known, unknown): (Vec<_>, Vec<_>) = self.names.clone().into_iter().partition(|(_, me)| me.is_known());
        Self { names: unknown.into_iter().chain(rest.names.clone()).chain(known).collect() }
    }

    /// The hosts where `answered` gives another name than the one known here.
    pub(crate) fn renamed_by(&self, answered: &Self) -> Vec<Renamed> {
        let changes = |held: &Me, new: &Me| held.is_known() && new.is_known() && held != new;
        let renamed = answered.names.iter().filter(|(host, new)| self.names.get(*host).is_some_and(|held| changes(held, new)));
        renamed.map(|(host, new)| Renamed { host: host.clone(), me: new.clone() }).collect()
    }
}

#[cfg(test)]
impl From<&str> for Identities {
    fn from(main: &str) -> Self {
        Self::on_main(Me::from(main))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on_github(key: MrKey) -> MrKey {
        MrKey { host: Some("github.com".into()), ..key }
    }

    #[test]
    fn a_username_is_mine_whatever_its_case() {
        assert!(Me::from("Nina").is("nina") && Me::from("nina").is("NINA"));
        assert!(!Me::from("nina").is("loic"));
    }

    #[test]
    fn nobody_is_me_before_a_name_is_known() {
        assert!(!Me::default().is(""));
    }

    #[test]
    fn an_mr_is_judged_by_the_name_of_its_own_host() {
        let me = Identities::from("nina").with_host("github.com", Me::from("nina-gh"));
        let key = MrKey::new("acme/widgets", 42);
        assert_eq!((me.on(&key).as_str(), me.on(&on_github(key)).as_str()), ("nina", "nina-gh"));
    }

    #[test]
    fn a_known_name_wins_over_the_other_one_and_an_unknown_one_keeps_it() {
        let held = Identities::from("old").with_host("github.com", Me::from("nina-gh"));
        let answered = Identities::from("nina").with_host("github.com", Me::default());
        assert_eq!(answered.over(&held), Identities::from("nina").with_host("github.com", Me::from("nina-gh")));
    }

    #[test]
    fn only_a_host_whose_known_name_changes_is_renamed() {
        let held = Identities::from("old").with_host("github.com", Me::from("nina-gh")).with_host("gitlab.com", Me::default());
        let answered = Identities::from("nina").with_host("github.com", Me::from("nina-gh")).with_host("gitlab.com", Me::from("nina"));
        assert_eq!(held.renamed_by(&answered), [Renamed { host: None, me: Me::from("nina") }]);
    }
}
