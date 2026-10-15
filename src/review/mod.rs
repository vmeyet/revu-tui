//! The value the TUI edits: one MR, its files as parsed hunks, its threads hung on lines, and what is folded.
pub(crate) mod draft;
mod file;
pub(crate) mod image;
pub(crate) mod place;
pub(crate) mod position;
mod rows;
pub(crate) mod suggestion;
pub(crate) mod thread;
pub(crate) mod tree;

pub(crate) use draft::{Draft, DraftId};
pub(crate) use file::File;
pub(crate) use place::{Conversation, Entry, EntryKind, Mark, Marker, Markers, Place, Spot};
pub(crate) use rows::Row;
pub(crate) use thread::{Anchor, Side, Thread, anchor_of};

use crate::diff::fold::{FileMeta, FoldState};
use crate::diff::words::InlineRule;
pub(crate) use crate::forge::FileKind;
use crate::forge::{DiffFile, Discussion, Mr, ThreadId};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// How far a review went: a file that opens folded counts only once marked viewed, the rest wait apart as `folded`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Progress {
    pub(crate) viewed: usize,
    pub(crate) files: usize,
    pub(crate) folded: usize,
}

impl Progress {
    pub(crate) fn new(files: usize, viewed: &BTreeSet<String>, auto_folded: &BTreeSet<String>) -> Self {
        let folded = auto_folded.difference(viewed).count();
        Self { viewed: viewed.len(), files: files.saturating_sub(folded), folded }
    }

    pub(crate) fn done(self) -> bool {
        self.viewed == self.files
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Review {
    /// What a fetch brought: never changed afterwards, so copies of the review share it.
    pub(crate) mr: Arc<Mr>,
    pub(crate) files: Arc<[File]>,
    pub(crate) threads: Arc<[Thread]>,
    /// The files that open folded (configured globs, too large, binary): they only count once viewed.
    pub(crate) auto_folded: Arc<BTreeSet<String>>,
    /// Replaced whole on every change, so a copy compared by identity tells whether they changed.
    pub(crate) drafts: Arc<[Draft]>,
    pub(crate) viewed: BTreeSet<String>,
    pub(crate) fold: FoldState,
    /// Which changed pairs read as one row.
    pub(crate) inline: InlineRule,
    /// The old file beside the new one, `D` in the TUI; the reader's choice, kept even while the diff is too narrow.
    pub(crate) side_by_side: bool,
    /// The diff area holds two sides of code; below, side by side falls back to inline.
    pub(crate) wide: bool,
    /// One side of the diff alone, as plain code: before on the old side, after on the new, `<` `>` in the TUI.
    pub(crate) peek: Option<Side>,
    /// Lines that changed only in whitespace read as one quiet row, `W` in the TUI.
    pub(crate) quiet_whitespace: bool,
    /// Unchanged lines shown around hunks, `+` in the TUI.
    pub(crate) context: Context,
    /// Threads read under their lines rather than in the right pane: in zen and in a narrow window.
    pub(crate) threads_in_diff: bool,
    /// The threads whose every note shows in the diff, by id; the others read as one row.
    pub(crate) unfolded: BTreeSet<ThreadId>,
}

/// The files read whole, and how many lines each hunk shows beyond its own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Context {
    /// The new side of a file at the head commit, one entry per line, shared between copies of the review.
    pub(crate) texts: BTreeMap<String, Arc<Vec<String>>>,
    /// Extra lines above and below a hunk, by (file, hunk).
    pub(crate) around: BTreeMap<(usize, usize), u32>,
}

impl Review {
    pub(crate) fn new(mr: Mr, diffs: &[DiffFile], discussions: Vec<Discussion>, fold_globs: &[String]) -> Self {
        let files: Vec<File> = diffs.iter().map(File::from_diff).collect();
        let threads = threads_of(discussions, &files);
        let metas: Vec<FileMeta> = files.iter().map(File::meta).collect();
        let fold = FoldState::initial(&metas, fold_globs);
        Self {
            mr: Arc::new(mr),
            files: files.into(),
            threads: threads.into(),
            auto_folded: Arc::new(fold.files.keys().cloned().collect()),
            drafts: Arc::from([]),
            viewed: BTreeSet::new(),
            fold,
            inline: InlineRule::default(),
            side_by_side: false,
            wide: true,
            peek: None,
            quiet_whitespace: false,
            context: Context::default(),
            threads_in_diff: false,
            unfolded: BTreeSet::new(),
        }
    }

    pub(crate) fn with_mr(self, mr: Mr) -> Self {
        Self { mr: Arc::new(mr), ..self }
    }

    pub(crate) fn with_inline(self, inline: InlineRule) -> Self {
        Self { inline, ..self }
    }

    pub(crate) fn with_side_by_side(self, side_by_side: bool) -> Self {
        Self { side_by_side, ..self }
    }

    pub(crate) fn with_wide(self, wide: bool) -> Self {
        Self { wide, ..self }
    }

    pub(crate) fn with_peek(self, peek: Option<Side>) -> Self {
        Self { peek, ..self }
    }

    /// Side by side was chosen, the diff is wide enough for it and no peek reads one side alone.
    pub(crate) fn shows_side_by_side(&self) -> bool {
        self.side_by_side && self.wide && self.peek.is_none()
    }

    pub(crate) fn with_threads_in_diff(self, threads_in_diff: bool) -> Self {
        Self { threads_in_diff, ..self }
    }

    pub(crate) fn with_unfolded(self, unfolded: BTreeSet<ThreadId>) -> Self {
        Self { unfolded, ..self }
    }

    /// Threads hang under their lines: asked for, and the diff is not side by side, where the pane keeps them.
    pub(crate) fn reads_threads_in_diff(&self) -> bool {
        self.threads_in_diff && !self.shows_side_by_side()
    }

    pub(crate) fn with_context(self, context: Context) -> Self {
        Self { context, ..self }
    }

    pub(crate) fn with_quiet_whitespace(self, quiet_whitespace: bool) -> Self {
        Self { quiet_whitespace, ..self }
    }

    pub(crate) fn with_fold(self, fold: FoldState) -> Self {
        Self { fold, ..self }
    }

    pub(crate) fn with_viewed(self, viewed: BTreeSet<String>) -> Self {
        Self { viewed, ..self }
    }

    /// Viewed files with the fingerprint of the change the reader saw, as the cache keeps them.
    pub(crate) fn viewed_fingerprints(&self) -> BTreeMap<String, String> {
        self.files.iter().filter(|f| self.viewed.contains(&f.new_path)).map(|f| (f.new_path.clone(), f.fingerprint())).collect()
    }

    /// The saved viewed files that still show the change the reader saw: one pushed to since is to read again.
    pub(crate) fn still_viewed(&self, saved: &BTreeMap<String, String>) -> BTreeSet<String> {
        self.files.iter().filter(|f| saved.get(&f.new_path) == Some(&f.fingerprint())).map(|f| f.new_path.clone()).collect()
    }

    pub(crate) fn progress(&self) -> Progress {
        Progress::new(self.files.len(), &self.viewed, &self.auto_folded)
    }

    pub(crate) fn with_drafts(self, drafts: Vec<Draft>) -> Self {
        Self { drafts: drafts.into(), ..self }
    }

    /// The colours already highlighted in `old` for every file that did not change, so a refresh highlights only what moved.
    pub(crate) fn with_colours_kept(self, old: &Arc<[File]>) -> Self {
        if Arc::ptr_eq(&self.files, old) {
            return self;
        }
        let by_path: BTreeMap<&str, &File> = old.iter().map(|file| (file.new_path.as_str(), file)).collect();
        let kept = |file: &File| by_path.get(file.new_path.as_str()).copied().filter(|old| *old == file).unwrap_or(file).clone();
        Self { files: self.files.iter().map(kept).collect(), ..self }
    }

    /// The same diff with fresh threads, for the cheap discussions poll; threads equal to the current ones stay the same
    /// shared slice, so nothing built on them is built again.
    pub(crate) fn with_discussions(self, discussions: Vec<Discussion>) -> Self {
        let threads = threads_of(discussions, &self.files);
        if *self.threads == *threads {
            return self;
        }
        Self { threads: threads.into(), ..self }
    }

    /// Every note in every thread, to notice new ones between two polls.
    pub(crate) fn note_count(&self) -> usize {
        self.threads.iter().map(|t| t.notes.len()).sum()
    }

    /// The thread flipped locally, ahead of the forge's answer.
    pub(crate) fn with_resolved(self, id: &ThreadId, resolved: bool) -> Self {
        let threads = self.threads.iter().map(|t| if t.id == *id { Thread { resolved, ..t.clone() } } else { t.clone() }).collect();
        Self { threads, ..self }
    }

    /// My reaction `emoji` on note `index` of thread `id` added or taken off, ahead of the forge's answer.
    pub(crate) fn with_reaction(
        self,
        id: &ThreadId,
        index: usize,
        emoji: crate::forge::Emoji,
        change: crate::forge::ReactionChange,
    ) -> Self {
        let react = |note: &crate::forge::Note| crate::forge::Note {
            reactions: crate::forge::toggled(&note.reactions, emoji, change),
            ..note.clone()
        };
        let threads = self
            .threads
            .iter()
            .map(|t| {
                if t.id != *id {
                    return t.clone();
                }
                let notes = t.notes.iter().enumerate().map(|(i, n)| if i == index { react(n) } else { n.clone() }).collect();
                Thread { notes, ..t.clone() }
            })
            .collect();
        Self { threads, ..self }
    }

    /// The MR marked approved by me or not, ahead of the forge's answer.
    /// My approval, `me`'s, added or taken back ahead of the forge's answer, so the header and `M` agree at once.
    pub(crate) fn with_approved(self, change: crate::forge::ApprovalChange, me: &str) -> Self {
        let approvals = self.mr.approvals.with_mine(change, me);
        Self { mr: Arc::new(Mr { approvals, ..(*self.mr).clone() }), ..self }
    }

    pub(crate) fn unresolved(&self) -> usize {
        self.threads.iter().filter(|t| t.unresolved()).count()
    }

    pub(crate) fn thread(&self, id: &ThreadId) -> Option<&Thread> {
        self.threads.iter().find(|t| t.id == *id)
    }

    pub(crate) fn threads_at(&self, path: &str, side: Side, line: u32) -> Vec<&Thread> {
        self.threads
            .iter()
            .filter(|t| !t.outdated)
            .filter(|t| t.anchor.as_ref().is_some_and(|a| a.path == path && a.side == side && a.line == line))
            .collect()
    }

    /// Drafts hung on a line, with their index in `drafts`; replies never hang anywhere.
    pub(crate) fn drafts_at(&self, path: &str, side: Side, line: u32) -> Vec<(usize, &Draft)> {
        self.drafts.iter().enumerate().filter(|(_, d)| d.is_at(path, side, line)).collect()
    }

    /// Drafts hung on a line the current diff no longer has: the forge refuses to publish them.
    pub(crate) fn stranded(&self) -> Vec<usize> {
        self.drafts
            .iter()
            .enumerate()
            .filter(|(_, d)| d.anchor.as_ref().is_some_and(|a| !self.files.iter().any(|f| f.line_at(a).is_some())))
            .map(|(i, _)| i)
            .collect()
    }

    pub(crate) fn outdated(&self, path: &str) -> Vec<&Thread> {
        self.threads.iter().filter(|t| t.outdated && t.anchor.as_ref().is_some_and(|a| a.path == path)).collect()
    }
}
fn threads_of(discussions: Vec<Discussion>, files: &[File]) -> Vec<Thread> {
    let mut threads: Vec<Thread> = discussions
        .into_iter()
        .filter_map(Thread::from_discussion)
        .map(|t| {
            let outdated = t.anchor.as_ref().is_some_and(|a| !files.iter().any(|f| f.line_at(a).is_some()));
            t.with_outdated(outdated)
        })
        .collect();
    threads.sort_by_key(|t| t.first().created_at);
    threads
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::forge::gitlab::fixture;
    use serde_json::json;

    pub(crate) fn mr() -> Mr {
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

    /// A review of one generated file, renamed so tests can tell its two paths apart, plus a
    /// second file so ranges can try to cross files.
    pub(crate) fn review_of(diff: &str) -> Review {
        let renamed = DiffFile {
            diff: diff.to_owned(),
            old_path: "src/old.rs".into(),
            new_path: "src/new.rs".into(),
            change: FileKind::Renamed,
            ..DiffFile::default()
        };
        let other =
            DiffFile { diff: "@@ -1 +1 @@\n-a\n+b\n".into(), old_path: "b.rs".into(), new_path: "b.rs".into(), ..DiffFile::default() };
        Review::new(mr(), &[renamed, other], vec![], &[])
    }

    /// Every `(hunk, line)` of the review's first file, in diff order.
    pub(crate) fn lines_of(review: &Review) -> Vec<(usize, usize)> {
        review.files[0].hunks.iter().enumerate().flat_map(|(h, hunk)| (0..hunk.lines.len()).map(move |l| (h, l))).collect()
    }

    pub(super) fn charge() -> DiffFile {
        DiffFile {
            diff: include_str!("fixtures/charge.diff").to_owned(),
            old_path: "src/pay/charge.rs".into(),
            new_path: "src/pay/charge.rs".into(),
            ..DiffFile::default()
        }
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

    /// A review of `files` files of 100 changed lines each, as big as the spec's performance budget.
    fn big_review(files: usize) -> Review {
        let body: String = (0..50).map(|i| format!("-    let value_{i} = old({i});\n+    let value_{i} = new({i});\n")).collect();
        let diffs: Vec<DiffFile> = (0..files)
            .map(|n| DiffFile {
                diff: format!("@@ -1,50 +1,50 @@\n{body}"),
                old_path: format!("src/f{n}.rs"),
                new_path: format!("src/f{n}.rs"),
                ..charge()
            })
            .collect();
        Review::new(mr(), &diffs, discussions(), &[])
    }

    /// Prints what one fold change costs on a 5 000-line review; run with `--ignored --nocapture`.
    #[test]
    #[ignore = "a measurement, not a check"]
    fn measure_a_fold_on_a_big_review() {
        let review = big_review(50);
        let rounds = 1_000;
        let started = std::time::Instant::now();
        let mut last = review;
        for _ in 0..rounds {
            let fold = last.fold.clone();
            last = last.with_fold(fold);
        }
        println!("with_fold on {} lines: {:?} each", 50 * 100, started.elapsed() / rounds);
        let started = std::time::Instant::now();
        for _ in 0..rounds {
            let fold = last.fold.clone();
            last = last.with_fold(fold);
            assert_ne!(last.rows(), [] as [Row; 0]);
        }
        println!("with_fold then rows, what za does: {:?} each", started.elapsed() / rounds);
        assert_eq!(last.files.len(), 50);
    }

    pub(crate) fn review() -> Review {
        Review::new(mr(), &[charge(), lock()], discussions(), &["*.lock".into()])
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
        let far = review.thread(&"9f2c0aa1d4e5b6c7".into()).unwrap();
        assert!(far.outdated, "line 57 is not in the diff");
        assert!(!review.thread(&"c0ffee00c0ffee00".into()).unwrap().outdated);
        assert_eq!(review.outdated("src/pay/charge.rs").len(), 1);
        assert!(review.threads_at("src/pay/charge.rs", Side::New, 57).is_empty(), "outdated threads never hang on a line");
    }

    #[test]
    fn fresh_discussions_keep_the_files_and_folds() {
        let review = review();
        let refreshed = review.clone().with_discussions(vec![fixture::discussion(include_str!("../forge/gitlab/fixtures/diff_note.json"))]);
        assert_eq!(refreshed.threads.len(), 1);
        assert_eq!(refreshed.files, review.files);
        assert_eq!(refreshed.fold, review.fold);
    }

    #[test]
    fn a_viewed_file_stays_viewed_until_its_change_moves() {
        let review = review();
        let path = review.files[0].new_path.clone();
        let saved = review.clone().with_viewed(BTreeSet::from([path.clone()])).viewed_fingerprints();
        assert_eq!(review.still_viewed(&saved), BTreeSet::from([path.clone()]));
        let moved = BTreeMap::from([(path, "an older change".to_owned())]);
        assert!(review.still_viewed(&moved).is_empty(), "a file pushed to since is to read again");
    }

    #[test]
    fn progress_leaves_auto_folded_files_apart_until_viewed() {
        let review = review();
        let (charge, lock) = (BTreeSet::from(["src/pay/charge.rs".to_owned()]), BTreeSet::from(["Cargo.lock".to_owned()]));
        assert_eq!(review.progress(), Progress { viewed: 0, files: 1, folded: 1 }, "the lock file waits apart");
        let read = review.clone().with_viewed(charge).progress();
        assert_eq!(read, Progress { viewed: 1, files: 1, folded: 1 });
        assert!(read.done(), "every file to read is viewed");
        assert_eq!(
            review.clone().with_viewed(lock).progress(),
            Progress { viewed: 1, files: 2, folded: 0 },
            "a viewed lock file joins the count"
        );
        let by_hand = review.clone().with_fold(review.fold.toggle_file("src/pay/charge.rs"));
        assert_eq!(by_hand.progress().files, 1, "a file folded by hand still counts");
        let unfolded = review.clone().with_fold(review.fold.toggle_file("Cargo.lock"));
        assert_eq!(unfolded.progress().folded, 1, "unfolding the lock file does not make it count");
    }
}
