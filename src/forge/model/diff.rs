//! A changed file, and where a note hangs in it.
use super::Refs;
use serde::{Deserialize, Serialize};

/// One changed file: its unified diff body (hunks from the first `@@`) and how it changed.
/// `change` has no default, so diffs cached before it existed fail to read and are fetched again.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DiffFile {
    pub(crate) diff: String,
    pub(crate) old_path: String,
    pub(crate) new_path: String,
    pub(crate) change: FileKind,
    /// The forge withheld the body: too many lines for it to send.
    #[serde(default)]
    pub(crate) too_large: bool,
}

/// How a file changed; a renamed file's old name is its `old_path`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum FileKind {
    Added,
    Deleted,
    Renamed,
    #[default]
    Modified,
    /// Only its mode changed, so its body is empty without being binary.
    Mode,
}

/// Which side of the diff a line number counts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub(crate) enum Side {
    Old,
    New,
}

/// One diff line by its numbers: a removed line has only `old`, an added one only `new`, a
/// context line both.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LineRef {
    pub(crate) old: Option<u32>,
    pub(crate) new: Option<u32>,
}

impl LineRef {
    /// The side a note on this line hangs on: new whenever the line exists there.
    pub(crate) fn side(self) -> Side {
        if self.new.is_some() { Side::New } else { Side::Old }
    }

    /// The number on `side()`.
    pub(crate) fn number(self) -> Option<u32> {
        self.new.or(self.old)
    }
}

/// Where a note hangs in the diff: one line, or the lines from `start` to `line`, in one file.
/// Each backend builds its own wire shape from this (GitLab's SHAs and `line_range`, GitHub's
/// `side`/`line`/`commit_id`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Position {
    pub(crate) refs: Refs,
    pub(crate) old_path: String,
    pub(crate) new_path: String,
    /// The line the note sits on: the last line of a range.
    pub(crate) line: LineRef,
    /// The first line of a range; `None` for a note on one line.
    pub(crate) start: Option<LineRef>,
}

impl Position {
    /// The path of the file on the side the note hangs on.
    pub(crate) fn path(&self) -> &str {
        match self.line.side() {
            Side::New => &self.new_path,
            Side::Old => &self.old_path,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_hangs_on_the_new_side_when_it_exists_there() {
        let context = LineRef { old: Some(12), new: Some(12) };
        let removed = LineRef { old: Some(13), new: None };
        let added = LineRef { old: None, new: Some(14) };
        assert_eq!((context.side(), context.number()), (Side::New, Some(12)));
        assert_eq!((removed.side(), removed.number()), (Side::Old, Some(13)));
        assert_eq!((added.side(), added.number()), (Side::New, Some(14)));
    }

    #[test]
    fn a_diff_cached_before_files_had_a_kind_is_not_read() {
        let old = r#"{"diff": "", "old_path": "a.rs", "new_path": "a.rs", "new_file": true}"#;
        assert!(serde_json::from_str::<DiffFile>(old).is_err());
    }
}
