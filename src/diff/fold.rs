//! What is folded in a review, cached per MR so reopening restores it.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Fold {
    #[default]
    Open,
    Closed,
}

impl Fold {
    fn flipped(self) -> Self {
        match self {
            Fold::Open => Fold::Closed,
            Fold::Closed => Fold::Open,
        }
    }
}

/// What `initial` needs to know about a file; kept apart from the API types on purpose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileMeta {
    pub path: String,
    pub too_large: bool,
    pub binary: bool,
}

/// A missing key means open, so the maps only hold what differs from a fresh view.
/// Hunks nest under their file path because JSON has no tuple keys.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoldState {
    #[serde(default)]
    pub files: BTreeMap<String, Fold>,
    #[serde(default)]
    pub hunks: BTreeMap<String, BTreeMap<usize, Fold>>,
}

impl FoldState {
    /// Closes what nobody wants to read first: huge files, binaries and the configured globs.
    pub fn initial(files: &[FileMeta], fold_globs: &[String]) -> Self {
        let patterns: Vec<glob::Pattern> = fold_globs.iter().filter_map(|g| glob::Pattern::new(g).ok()).collect();
        let closed = |file: &FileMeta| file.too_large || file.binary || patterns.iter().any(|p| p.matches(&file.path));
        let files = files.iter().filter(|f| closed(f)).map(|f| (f.path.clone(), Fold::Closed)).collect();
        Self { files, hunks: BTreeMap::new() }
    }

    pub fn file_is_open(&self, path: &str) -> bool {
        self.files.get(path).copied().unwrap_or_default() == Fold::Open
    }

    pub fn hunk_is_open(&self, path: &str, index: usize) -> bool {
        self.hunks.get(path).and_then(|h| h.get(&index)).copied().unwrap_or_default() == Fold::Open
    }

    pub fn toggle_file(&self, path: &str) -> Self {
        let mut files = self.files.clone();
        let next = files.get(path).copied().unwrap_or_default().flipped();
        set_or_forget(&mut files, path.to_owned(), next);
        Self { files, hunks: self.hunks.clone() }
    }

    pub fn toggle_hunk(&self, path: &str, index: usize) -> Self {
        let mut hunks = self.hunks.clone();
        let mut in_file = hunks.remove(path).unwrap_or_default();
        let next = in_file.get(&index).copied().unwrap_or_default().flipped();
        set_or_forget(&mut in_file, index, next);
        if !in_file.is_empty() {
            hunks.insert(path.to_owned(), in_file);
        }
        Self { files: self.files.clone(), hunks }
    }

    pub fn fold_all(&self, paths: &[String]) -> Self {
        let files = paths.iter().map(|p| (p.clone(), Fold::Closed)).collect();
        Self { files, hunks: self.hunks.clone() }
    }
}

fn set_or_forget<K: Ord>(map: &mut BTreeMap<K, Fold>, key: K, fold: Fold) {
    match fold {
        Fold::Open => {
            map.remove(&key);
        }
        Fold::Closed => {
            map.insert(key, fold);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn file(path: &str) -> FileMeta {
        FileMeta { path: path.into(), too_large: false, binary: false }
    }

    #[test]
    fn everything_starts_open() {
        let state = FoldState::default();
        assert!(state.file_is_open("src/a.rs"));
        assert!(state.hunk_is_open("src/a.rs", 0));
    }

    #[test]
    fn initial_closes_large_binary_and_glob_matches() {
        let files = [
            file("src/a.rs"),
            FileMeta { too_large: true, ..file("data.json") },
            FileMeta { binary: true, ..file("logo.png") },
            file("Cargo.lock"),
            file("src/snapshots/x.snap"),
        ];
        let globs = ["*.lock".to_owned(), "**/*.snap".to_owned(), "[".to_owned()];
        let state = FoldState::initial(&files, &globs);
        assert!(state.file_is_open("src/a.rs"));
        assert!(!state.file_is_open("data.json"));
        assert!(!state.file_is_open("logo.png"));
        assert!(!state.file_is_open("Cargo.lock"));
        assert!(!state.file_is_open("src/snapshots/x.snap"));
        assert_eq!(state.files.len(), 4, "an invalid glob is ignored");
    }

    #[test]
    fn toggling_twice_returns_to_the_same_value_and_leaves_no_key() {
        let state = FoldState::default();
        let closed = state.toggle_file("a");
        assert!(!closed.file_is_open("a"));
        assert!(state.file_is_open("a"), "the source is untouched");
        let reopened = closed.toggle_file("a");
        assert_eq!(reopened, state);
        let hunk_closed = state.toggle_hunk("a", 2);
        assert!(!hunk_closed.hunk_is_open("a", 2));
        assert!(hunk_closed.hunk_is_open("a", 1));
        assert_eq!(hunk_closed.toggle_hunk("a", 2), state);
    }

    #[test]
    fn fold_all_and_unfold_all() {
        let paths = ["a".to_owned(), "b".to_owned()];
        let folded = FoldState::default().toggle_hunk("a", 0).fold_all(&paths);
        assert!(!folded.file_is_open("a") && !folded.file_is_open("b"));
        assert!(!folded.hunk_is_open("a", 0), "hunk folds survive fold_all");
        assert_ne!(folded, FoldState::default());
    }

    #[test]
    fn round_trips_through_json() {
        let state = FoldState::default().toggle_file("a").toggle_hunk("b", 3);
        let text = serde_json::to_string(&state).unwrap();
        assert_eq!(serde_json::from_str::<FoldState>(&text).unwrap(), state);
    }
}
