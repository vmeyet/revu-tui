//! How often I gave each reaction, counted per host in the shared cache, so the picker can offer the ones I use first.
use crate::cache::{Cache, keys};
use crate::forge::Emoji;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::BTreeMap;

/// The reactions I gave, by host.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct History(BTreeMap<String, Vec<Given>>);

/// One reaction I gave on a host: how many times, and `last`, which rises each time any reaction is given there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Given {
    pub emoji: Emoji,
    pub count: u32,
    pub last: u64,
}

impl History {
    /// The same history with `emoji` given once more on `host`.
    #[must_use]
    pub fn with(&self, host: &str, emoji: Emoji) -> Self {
        let given = self.0.get(host).cloned().unwrap_or_default();
        let last = given.iter().map(|g| g.last).max().unwrap_or_default() + 1;
        let count = given.iter().find(|g| g.emoji == emoji).map_or(0, |g| g.count) + 1;
        let others = given.into_iter().filter(|g| g.emoji != emoji);
        let mut hosts = self.0.clone();
        hosts.insert(host.to_owned(), others.chain([Given { emoji, count, last }]).collect());
        Self(hosts)
    }

    /// Every reaction given on `host`, the most given first, then the latest given.
    pub fn most_used(&self, host: &str) -> Vec<Emoji> {
        let mut given = self.0.get(host).cloned().unwrap_or_default();
        given.sort_by_key(|g| Reverse((g.count, g.last)));
        given.into_iter().map(|g| g.emoji).collect()
    }
}

/// The history kept in the cache; none when the file is missing or broken.
pub fn read(cache: &Cache) -> History {
    cache.read(&keys::reactions()).unwrap_or_default()
}

/// Counts `emoji` given once more on `host` in the cache, whatever other revu processes wrote meanwhile.
pub fn record(cache: &Cache, host: &str, emoji: Emoji) -> Result<History> {
    cache.update(&keys::reactions(), |history: History| history.with(host, emoji))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hundred() -> Emoji {
        Emoji::from_gitlab("100").unwrap()
    }

    #[test]
    fn the_most_given_come_first_and_ties_go_to_the_latest() {
        let history = History::default()
            .with("gitlab.com", Emoji::Eyes)
            .with("gitlab.com", Emoji::Rocket)
            .with("gitlab.com", Emoji::Rocket)
            .with("gitlab.com", Emoji::Heart)
            .with("gitlab.com", hundred());
        assert_eq!(history.most_used("gitlab.com"), [Emoji::Rocket, hundred(), Emoji::Heart, Emoji::Eyes]);
    }

    #[test]
    fn each_host_keeps_its_own_counts() {
        let history = History::default().with("gitlab.com", hundred()).with("github.com", Emoji::Eyes);
        assert_eq!(history.most_used("gitlab.com"), [hundred()]);
        assert_eq!(history.most_used("github.com"), [Emoji::Eyes]);
        assert_eq!(history.most_used("gitlab.example.org"), []);
    }

    #[test]
    fn the_cache_keeps_the_counts_and_a_missing_or_broken_file_is_no_history() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::in_dir(dir.path());
        assert_eq!(read(&cache), History::default());
        record(&cache, "gitlab.com", Emoji::Eyes).unwrap();
        let history = record(&cache, "gitlab.com", hundred()).unwrap();
        assert_eq!(read(&cache), history);
        assert_eq!(history.most_used("gitlab.com"), [hundred(), Emoji::Eyes]);
        std::fs::write(dir.path().join(keys::reactions()), b"[not json").unwrap();
        assert_eq!(read(&cache), History::default());
        assert_eq!(record(&cache, "gitlab.com", Emoji::Eyes).unwrap().most_used("gitlab.com"), [Emoji::Eyes]);
    }
}
