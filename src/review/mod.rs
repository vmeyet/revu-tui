//! The value the TUI edits: one MR, its files as parsed hunks, its threads hung on lines, and what is folded.
pub mod draft;
pub mod position;
pub mod suggestion;
pub mod thread;
pub mod tree;

pub use draft::Draft;
pub use thread::{Anchor, Side, Thread};

use crate::diff::fold::{FileMeta, FoldState};
use crate::diff::words::{self, InlineRule};
use crate::diff::{self, Hunk, LineKind};
use crate::forge::{DiffFile, Discussion, Mr};
use crate::syntax::{self, Spans};
use std::collections::BTreeSet;

/// Above this many lines a file starts folded, whatever the forge says.
const TOO_LARGE_LINES: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Added,
    Deleted,
    Renamed,
    Modified,
    Mode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    pub old_path: String,
    pub new_path: String,
    pub kind: FileKind,
    pub binary: bool,
    pub too_large: bool,
    pub hunks: Vec<Hunk>,
    pub additions: usize,
    pub deletions: usize,
    /// Syntax colours per hunk, per line, parallel to `hunks`; empty for a language revu does not colour.
    pub syntax: Vec<Vec<Spans>>,
}

impl File {
    pub fn from_diff(diff: &DiffFile) -> Self {
        let hunks: Vec<Hunk> = diff::parse(&diff.diff).iter().map(diff::words::mark).collect();
        let lines = hunks.iter().map(|h| h.lines.len()).sum::<usize>();
        let count = |kind: LineKind| hunks.iter().flat_map(|h| &h.lines).filter(|l| l.kind == kind).count();
        let kind = kind_of(diff);
        let too_large = diff.too_large || lines > TOO_LARGE_LINES;
        let path = if kind == FileKind::Deleted { &diff.old_path } else { &diff.new_path };
        let syntax = match syntax::language_for(path, &syntax::LANGUAGES) {
            Some(language) if !too_large => hunks.iter().map(|hunk| colour(language, hunk)).collect(),
            _ => Vec::new(),
        };
        Self {
            old_path: diff.old_path.clone(),
            new_path: diff.new_path.clone(),
            kind,
            binary: diff.diff.is_empty() && !matches!(kind, FileKind::Mode | FileKind::Renamed),
            too_large,
            syntax,
            additions: count(LineKind::Added),
            deletions: count(LineKind::Removed),
            hunks,
        }
    }

    /// The syntax colours of one diff line; empty when the file is not coloured.
    pub fn spans(&self, hunk: usize, line: usize) -> &[(std::ops::Range<usize>, syntax::Token)] {
        self.syntax.get(hunk).and_then(|lines| lines.get(line)).map_or(&[], Vec::as_slice)
    }

    /// Names this file's change: equal fingerprints mean nothing was pushed to it since.
    pub fn fingerprint(&self) -> String {
        let mut hash = sha1_smol::Sha1::new();
        for hunk in &self.hunks {
            hash.update(hunk.header.as_bytes());
            for line in &hunk.lines {
                hash.update(&[line.kind as u8]);
                hash.update(line.text.as_bytes());
                hash.update(b"\n");
            }
        }
        hash.digest().to_string()
    }

    pub fn meta(&self) -> FileMeta {
        FileMeta { path: self.new_path.clone(), too_large: self.too_large, binary: self.binary }
    }

    fn has_line(&self, anchor: &Anchor) -> bool {
        let path = match anchor.side {
            Side::New => &self.new_path,
            Side::Old => &self.old_path,
        };
        if path != &anchor.path {
            return false;
        }
        self.hunks.iter().flat_map(|h| &h.lines).any(|l| match anchor.side {
            Side::New => l.new == Some(anchor.line),
            Side::Old => l.old == Some(anchor.line),
        })
    }
}

/// Each side of a hunk is highlighted as one text, so a string or comment spanning lines keeps its
/// colour; each line then takes the spans of the side it belongs to (context lines, the new side).
fn colour(language: &syntax::Language, hunk: &Hunk) -> Vec<Spans> {
    let side = |skip: LineKind| -> (Vec<usize>, Vec<Spans>) {
        let indexes: Vec<usize> = hunk.lines.iter().enumerate().filter(|(_, l)| l.kind != skip).map(|(i, _)| i).collect();
        let text: Vec<&str> = indexes.iter().map(|&i| hunk.lines[i].text.as_str()).collect();
        (indexes, syntax::highlight(language, &text.join("\n")))
    };
    let (old_lines, old_spans) = side(LineKind::Added);
    let (new_lines, new_spans) = side(LineKind::Removed);
    let mut lines = vec![Spans::new(); hunk.lines.len()];
    for (index, spans) in old_lines.into_iter().zip(old_spans) {
        lines[index] = spans;
    }
    for (index, spans) in new_lines.into_iter().zip(new_spans) {
        lines[index] = spans;
    }
    lines
}

fn kind_of(diff: &DiffFile) -> FileKind {
    if diff.new_file {
        FileKind::Added
    } else if diff.deleted_file {
        FileKind::Deleted
    } else if diff.renamed_file {
        FileKind::Renamed
    } else if diff.diff.is_empty() && diff.a_mode != diff.b_mode {
        FileKind::Mode
    } else {
        FileKind::Modified
    }
}

/// One row of the review pane, in reading order, after folds. Indexes point into `Review`, so a row is tiny.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Header,
    File {
        index: usize,
        open: bool,
    },
    Hunk {
        file: usize,
        index: usize,
        open: bool,
    },
    Line {
        file: usize,
        hunk: usize,
        index: usize,
    },
    /// A removed line and its added twin read as one row, the changed words side by side.
    Pair {
        file: usize,
        hunk: usize,
        removed: usize,
        added: usize,
    },
    Thread {
        id: String,
    },
    /// An unpublished note on a line; the index points into `Review::drafts`.
    Draft {
        index: usize,
    },
    Outdated {
        file: usize,
    },
    Gap,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Review {
    pub mr: Mr,
    pub files: Vec<File>,
    pub threads: Vec<Thread>,
    pub drafts: Vec<Draft>,
    pub viewed: BTreeSet<String>,
    pub fold: FoldState,
    /// Which changed pairs read as one row.
    pub inline: InlineRule,
    /// Every changed line on its own row, `D` in the TUI.
    pub split: bool,
}

impl Review {
    pub fn new(mr: Mr, diffs: &[DiffFile], discussions: Vec<Discussion>, fold_globs: &[String]) -> Self {
        let files: Vec<File> = diffs.iter().map(File::from_diff).collect();
        let threads = threads_of(discussions, &files);
        let metas: Vec<FileMeta> = files.iter().map(File::meta).collect();
        let fold = FoldState::initial(&metas, fold_globs);
        Self { mr, files, threads, drafts: vec![], viewed: BTreeSet::new(), fold, inline: InlineRule::default(), split: false }
    }

    pub fn with_inline(&self, inline: InlineRule) -> Self {
        Self { inline, ..self.clone() }
    }

    pub fn with_split(&self, split: bool) -> Self {
        Self { split, ..self.clone() }
    }

    pub fn with_fold(&self, fold: FoldState) -> Self {
        Self { fold, ..self.clone() }
    }

    pub fn with_viewed(&self, viewed: BTreeSet<String>) -> Self {
        Self { viewed, ..self.clone() }
    }

    /// Viewed files with the fingerprint of the change the reader saw, as the cache keeps them.
    pub fn viewed_fingerprints(&self) -> std::collections::BTreeMap<String, String> {
        self.files.iter().filter(|f| self.viewed.contains(&f.new_path)).map(|f| (f.new_path.clone(), f.fingerprint())).collect()
    }

    /// The saved viewed files that still show the change the reader saw: one pushed to since is to read again.
    pub fn still_viewed(&self, saved: &std::collections::BTreeMap<String, String>) -> BTreeSet<String> {
        self.files.iter().filter(|f| saved.get(&f.new_path) == Some(&f.fingerprint())).map(|f| f.new_path.clone()).collect()
    }

    pub fn with_drafts(&self, drafts: Vec<Draft>) -> Self {
        Self { drafts, ..self.clone() }
    }

    /// The same diff with fresh threads, for the cheap discussions poll.
    pub fn with_discussions(&self, discussions: Vec<Discussion>) -> Self {
        Self { threads: threads_of(discussions, &self.files), ..self.clone() }
    }

    /// The thread flipped locally, ahead of the forge's answer.
    pub fn with_resolved(&self, id: &str, resolved: bool) -> Self {
        let threads = self.threads.iter().map(|t| if t.id == id { Thread { resolved, ..t.clone() } } else { t.clone() }).collect();
        Self { threads, ..self.clone() }
    }

    pub fn unresolved(&self) -> usize {
        self.threads.iter().filter(|t| t.resolvable && !t.resolved).count()
    }

    pub fn thread(&self, id: &str) -> Option<&Thread> {
        self.threads.iter().find(|t| t.id == id)
    }

    pub fn threads_at(&self, path: &str, side: Side, line: u32) -> Vec<&Thread> {
        self.threads
            .iter()
            .filter(|t| !t.outdated)
            .filter(|t| t.anchor.as_ref().is_some_and(|a| a.path == path && a.side == side && a.line == line))
            .collect()
    }

    /// Drafts hung on a line, with their index in `drafts`; replies never hang anywhere.
    pub fn drafts_at(&self, path: &str, side: Side, line: u32) -> Vec<(usize, &Draft)> {
        self.drafts.iter().enumerate().filter(|(_, d)| d.is_at(path, side, line)).collect()
    }

    /// Drafts hung on a line the current diff no longer has: the forge refuses to publish them.
    pub fn stranded(&self) -> Vec<usize> {
        self.drafts
            .iter()
            .enumerate()
            .filter(|(_, d)| d.anchor.as_ref().is_some_and(|a| !self.files.iter().any(|f| f.has_line(a))))
            .map(|(i, _)| i)
            .collect()
    }

    pub fn outdated(&self, path: &str) -> Vec<&Thread> {
        self.threads.iter().filter(|t| t.outdated && t.anchor.as_ref().is_some_and(|a| a.path == path)).collect()
    }

    pub fn rows(&self) -> Vec<Row> {
        let mut rows = vec![Row::Header];
        for (index, file) in self.files.iter().enumerate() {
            rows.push(Row::Gap);
            let open = self.fold.file_is_open(&file.new_path);
            rows.push(Row::File { index, open });
            if open {
                self.push_file_rows(&mut rows, index, file);
            }
        }
        rows
    }

    fn push_file_rows(&self, rows: &mut Vec<Row>, index: usize, file: &File) {
        for (hunk_index, hunk) in file.hunks.iter().enumerate() {
            let open = self.fold.hunk_is_open(&file.new_path, hunk_index);
            rows.push(Row::Hunk { file: index, index: hunk_index, open });
            if !open {
                continue;
            }
            let pairs = if self.split { vec![] } else { words::inline_pairs(hunk, self.inline) };
            for (line_index, line) in hunk.lines.iter().enumerate() {
                if pairs.iter().any(|&(_, added)| added == line_index) {
                    continue;
                }
                if let Some(&(removed, added)) = pairs.iter().find(|&&(removed, _)| removed == line_index) {
                    rows.push(Row::Pair { file: index, hunk: hunk_index, removed, added });
                    self.push_anchors(rows, file, &hunk.lines[added]);
                } else {
                    rows.push(Row::Line { file: index, hunk: hunk_index, index: line_index });
                }
                self.push_anchors(rows, file, line);
            }
        }
        if !self.outdated(&file.new_path).is_empty() {
            rows.push(Row::Outdated { file: index });
        }
    }

    /// Threads hung on a line, new side first: a context line carries both numbers.
    fn threads_on(&self, file: &File, line: &diff::Line) -> Vec<&Thread> {
        let new = line.new.map(|n| self.threads_at(&file.new_path, Side::New, n)).unwrap_or_default();
        let old = line.old.map(|n| self.threads_at(&file.old_path, Side::Old, n)).unwrap_or_default();
        new.into_iter().chain(old).collect()
    }
}

impl Review {
    /// The threads, then the drafts, hung on one line.
    fn push_anchors(&self, rows: &mut Vec<Row>, file: &File, line: &diff::Line) {
        rows.extend(self.threads_on(file, line).into_iter().map(|t| Row::Thread { id: t.id.clone() }));
        rows.extend(self.drafts_on(file, line).into_iter().map(|index| Row::Draft { index }));
    }

    fn drafts_on(&self, file: &File, line: &diff::Line) -> Vec<usize> {
        let new = line.new.map(|n| self.drafts_at(&file.new_path, Side::New, n)).unwrap_or_default();
        let old = line.old.map(|n| self.drafts_at(&file.old_path, Side::Old, n)).unwrap_or_default();
        new.into_iter().chain(old).map(|(index, _)| index).collect()
    }
}

fn threads_of(discussions: Vec<Discussion>, files: &[File]) -> Vec<Thread> {
    let mut threads: Vec<Thread> = discussions
        .into_iter()
        .filter_map(Thread::from_discussion)
        .map(|t| {
            let outdated = t.anchor.as_ref().is_some_and(|a| !files.iter().any(|f| f.has_line(a)));
            t.with_outdated(outdated)
        })
        .collect();
    threads.sort_by_key(|t| t.first().created_at);
    threads
}

#[cfg(test)]
pub(super) mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::forge::gitlab::fixture;
    use serde_json::json;

    fn mr() -> Mr {
        fixture::mr(
            &json!({
                "id": 1042, "iid": 42, "project_id": 7, "title": "feat: charge cards at checkout",
                "state": "opened", "draft": false,
                "author": {"id": 5, "username": "omar", "name": "Omar"},
                "source_branch": "feat/checkout", "target_branch": "main",
                "web_url": "https://gitlab.com/acme/widgets/-/merge_requests/42",
                "updated_at": "2026-09-22T09:12:00Z", "sha": "bbbb",
                "diff_refs": {"base_sha": "aaaa", "head_sha": "bbbb", "start_sha": "aaaa"}
            })
            .to_string(),
        )
    }

    fn charge() -> DiffFile {
        DiffFile {
            diff: include_str!("fixtures/charge.diff").to_owned(),
            old_path: "src/pay/charge.rs".into(),
            new_path: "src/pay/charge.rs".into(),
            a_mode: "100644".into(),
            b_mode: "100644".into(),
            ..DiffFile::default()
        }
    }

    fn cart() -> DiffFile {
        DiffFile {
            diff: include_str!("fixtures/cart.ts.diff").to_owned(),
            old_path: "src/cart.ts".into(),
            new_path: "src/cart.ts".into(),
            ..DiffFile::default()
        }
    }

    #[test]
    fn typescript_files_carry_syntax_per_line_and_other_files_none() {
        let file = File::from_diff(&cart());
        assert_eq!(file.syntax.len(), file.hunks.len());
        assert_eq!(file.syntax[0].len(), file.hunks[0].lines.len());
        let removed = &file.hunks[0].lines[1];
        assert!(file.spans(0, 1).iter().any(|(r, t)| &removed.text[r.clone()] == "const" && *t == crate::syntax::Token::Keyword));
        let added = &file.hunks[0].lines[4];
        assert!(file.spans(0, 4).iter().any(|(r, t)| &added.text[r.clone()] == "\"total\"" && *t == crate::syntax::Token::String));
        assert!(File::from_diff(&charge()).syntax.is_empty(), "no grammar for .rs yet");
        assert!(File::from_diff(&DiffFile { too_large: true, ..cart() }).syntax.is_empty(), "too large files stay plain");
    }

    fn lock() -> DiffFile {
        DiffFile {
            diff: "@@ -1 +1 @@\n-a\n+b\n".into(),
            old_path: "Cargo.lock".into(),
            new_path: "Cargo.lock".into(),
            ..DiffFile::default()
        }
    }

    fn discussions() -> Vec<Discussion> {
        vec![
            fixture::discussion(include_str!("../forge/gitlab/fixtures/discussions.json")),
            fixture::discussion(include_str!("../forge/gitlab/fixtures/diff_note.json")),
            fixture::discussion(include_str!("fixtures/old_side_note.json")),
        ]
    }

    pub(super) fn review() -> Review {
        Review::new(mr(), &[charge(), lock()], discussions(), &["*.lock".into()])
    }

    #[test]
    fn file_kinds_follow_gitlabs_flags() {
        let kind = |diff: DiffFile| File::from_diff(&diff).kind;
        assert_eq!(kind(DiffFile { new_file: true, ..charge() }), FileKind::Added);
        assert_eq!(kind(DiffFile { deleted_file: true, ..charge() }), FileKind::Deleted);
        assert_eq!(kind(DiffFile { renamed_file: true, ..charge() }), FileKind::Renamed);
        assert_eq!(kind(DiffFile { diff: String::new(), b_mode: "100755".into(), ..charge() }), FileKind::Mode);
        assert_eq!(kind(charge()), FileKind::Modified);
    }

    #[test]
    fn counts_additions_and_deletions() {
        let file = File::from_diff(&charge());
        assert_eq!((file.additions, file.deletions, file.hunks.len()), (4, 2, 2));
    }

    #[test]
    fn an_empty_diff_is_binary_unless_it_is_a_mode_change_or_a_rename() {
        let empty = |diff: DiffFile| File::from_diff(&DiffFile { diff: String::new(), ..diff }).binary;
        assert!(empty(charge()));
        assert!(empty(DiffFile { new_file: true, a_mode: "0".into(), ..charge() }));
        assert!(!empty(DiffFile { b_mode: "100755".into(), ..charge() }));
        assert!(!empty(DiffFile { renamed_file: true, old_path: "src/pay/old.rs".into(), ..charge() }));
        assert!(!File::from_diff(&charge()).binary);
    }

    #[test]
    fn too_large_from_gitlab_or_from_the_line_count() {
        assert!(File::from_diff(&DiffFile { too_large: true, ..charge() }).too_large);
        let long = format!("@@ -1,0 +1,{n} @@\n{}", "+x\n".repeat(TOO_LARGE_LINES + 1), n = TOO_LARGE_LINES + 1);
        assert!(File::from_diff(&DiffFile { diff: long, ..charge() }).too_large);
        assert!(!File::from_diff(&charge()).too_large);
    }

    #[test]
    fn threads_anchor_on_both_sides_and_sort_by_time() {
        let review = review();
        let ids: Vec<&str> = review.threads.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["c0ffee00c0ffee00", "6a9c1750b2d6e4f0", "9f2c0aa1d4e5b6c7"]);
        assert_eq!(review.threads_at("src/pay/charge.rs", Side::Old, 13).len(), 1);
        assert_eq!(review.threads_at("src/pay/charge.rs", Side::New, 13).len(), 0);
        assert_eq!(review.unresolved(), 1);
    }

    #[test]
    fn a_thread_whose_line_left_the_diff_is_outdated() {
        let review = review();
        let far = review.thread("9f2c0aa1d4e5b6c7").unwrap();
        assert!(far.outdated, "line 57 is not in the diff");
        assert!(!review.thread("c0ffee00c0ffee00").unwrap().outdated);
        assert_eq!(review.outdated("src/pay/charge.rs").len(), 1);
        assert!(review.threads_at("src/pay/charge.rs", Side::New, 57).is_empty(), "outdated threads never hang on a line");
    }

    #[test]
    fn rows_follow_folds() {
        let review = review();
        let rows = review.rows();
        assert_eq!(rows[0], Row::Header);
        assert_eq!(rows[2], Row::File { index: 0, open: true });
        assert_eq!(rows[3], Row::Hunk { file: 0, index: 0, open: true });
        let thread_at = rows.iter().position(|r| *r == Row::Thread { id: "c0ffee00c0ffee00".into() }).unwrap();
        assert_eq!(rows[thread_at - 1], Row::Line { file: 0, hunk: 0, index: 1 }, "right after the removed line 13");
        assert!(rows.contains(&Row::Outdated { file: 0 }));
        assert_eq!(rows.last(), Some(&Row::File { index: 1, open: false }), "the lock file starts folded");
        assert_eq!(rows.iter().filter(|r| matches!(r, Row::Gap)).count(), 2);
    }

    #[test]
    fn fresh_discussions_keep_the_files_and_folds() {
        let review = review();
        let refreshed = review.with_discussions(vec![fixture::discussion(include_str!("../forge/gitlab/fixtures/diff_note.json"))]);
        assert_eq!(refreshed.threads.len(), 1);
        assert_eq!(refreshed.files, review.files);
        assert_eq!(refreshed.fold, review.fold);
    }

    #[test]
    fn a_closed_hunk_keeps_its_row_and_hides_its_lines() {
        let review = review();
        let folded = review.with_fold(review.fold.toggle_hunk("src/pay/charge.rs", 0));
        let rows = folded.rows();
        assert_eq!(rows[3], Row::Hunk { file: 0, index: 0, open: false });
        assert_eq!(rows[4], Row::Hunk { file: 0, index: 1, open: true });
        assert!(!rows.contains(&Row::Thread { id: "c0ffee00c0ffee00".into() }));
        assert!(rows.contains(&Row::Line { file: 0, hunk: 1, index: 0 }));
    }

    #[test]
    fn drafts_hang_after_the_threads_of_their_line_and_replies_hang_nowhere() {
        let on_line =
            Draft::new(Some(Anchor { path: "src/pay/charge.rs".into(), side: Side::Old, line: 13 }), "why drop the default client?");
        let reply = Draft::reply("c0ffee00c0ffee00", "agreed");
        let on_mr = Draft::new(None, "overall fine");
        let review = review().with_drafts(vec![reply, on_line, on_mr]);
        let rows = review.rows();
        let thread_at = rows.iter().position(|r| *r == Row::Thread { id: "c0ffee00c0ffee00".into() }).unwrap();
        assert_eq!(rows[thread_at + 1], Row::Draft { index: 1 });
        assert_eq!(rows.iter().filter(|r| matches!(r, Row::Draft { .. })).count(), 1);
        assert_eq!(review.drafts_at("src/pay/charge.rs", Side::Old, 13).len(), 1);
        assert!(review.drafts_at("src/pay/charge.rs", Side::New, 13).is_empty());
    }

    #[test]
    fn folding_every_file_leaves_one_row_each() {
        let review = review();
        let paths: Vec<String> = review.files.iter().map(|f| f.new_path.clone()).collect();
        let rows = review.with_fold(review.fold.fold_all(&paths)).rows();
        assert_eq!(rows, [Row::Header, Row::Gap, Row::File { index: 0, open: false }, Row::Gap, Row::File { index: 1, open: false }]);
    }

    #[test]
    fn a_viewed_file_stays_viewed_until_its_change_moves() {
        let review = review();
        let path = review.files[0].new_path.clone();
        let saved = review.with_viewed(BTreeSet::from([path.clone()])).viewed_fingerprints();
        assert_eq!(review.still_viewed(&saved), BTreeSet::from([path.clone()]));
        let moved = std::collections::BTreeMap::from([(path, "an older change".to_owned())]);
        assert!(review.still_viewed(&moved).is_empty(), "a file pushed to since is to read again");
    }
}
