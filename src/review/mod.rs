//! The value the TUI edits: one MR, its files as parsed hunks, its threads hung on lines, and what is folded.
pub mod draft;
pub mod image;
pub mod place;
pub mod position;
pub mod suggestion;
pub mod thread;
pub mod tree;

pub use draft::{Draft, DraftId};
pub use place::{Conversation, Entry, EntryKind, Mark, Marker, Markers, Place, Spot};
pub use thread::{Anchor, Side, Thread, anchor_of};

use crate::diff::fold::{FileMeta, FoldState};
use crate::diff::words::{self, InlineRule};
use crate::diff::{self, Hunk, LineKind};
pub use crate::forge::FileKind;
use crate::forge::{DiffFile, Discussion, Mr, ThreadId};
use crate::syntax::{self, Spans};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock};

/// Above this many lines a file starts folded, whatever the forge says.
const TOO_LARGE_LINES: usize = 2000;

/// How far a review went: a file that opens folded counts only once marked viewed, the rest wait apart as `folded`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub viewed: usize,
    pub files: usize,
    pub folded: usize,
}

impl Progress {
    pub fn new(files: usize, viewed: &BTreeSet<String>, auto_folded: &BTreeSet<String>) -> Self {
        let folded = auto_folded.difference(viewed).count();
        Self { viewed: viewed.len(), files: files.saturating_sub(folded), folded }
    }

    pub fn done(self) -> bool {
        self.viewed == self.files
    }
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
    colours: Colours,
}

/// The syntax colours of a file, per hunk and per line, each hunk highlighted the first time one of its lines is
/// read: a folded file costs nothing until it is opened and drawn.
#[derive(Clone, Default)]
struct Colours {
    language: Option<&'static syntax::Language>,
    hunks: Vec<OnceLock<Vec<Spans>>>,
}

impl Colours {
    fn coloured(&self) -> usize {
        self.hunks.iter().filter(|hunk| hunk.get().is_some()).count()
    }
}

/// Colours follow from the hunks and the path, so files equal in those have equal colours, highlighted yet or not.
impl PartialEq for Colours {
    fn eq(&self, other: &Self) -> bool {
        self.language.map(|l| l.name) == other.language.map(|l| l.name)
    }
}

impl Eq for Colours {}

impl std::fmt::Debug for Colours {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Colours")
            .field("language", &self.language.map(|l| l.name))
            .field("coloured", &self.coloured())
            .finish_non_exhaustive()
    }
}

impl File {
    pub fn from_diff(diff: &DiffFile) -> Self {
        let hunks: Vec<Hunk> = diff::parse(&diff.diff).iter().map(words::mark).collect();
        let lines = hunks.iter().map(|h| h.lines.len()).sum::<usize>();
        let count = |kind: LineKind| hunks.iter().flat_map(|h| &h.lines).filter(|l| l.kind == kind).count();
        let kind = diff.change;
        let too_large = diff.too_large || lines > TOO_LARGE_LINES;
        let path = if kind == FileKind::Deleted { &diff.old_path } else { &diff.new_path };
        let language = syntax::language_for(path, &syntax::LANGUAGES).filter(|_| !too_large);
        let colours = Colours { language, hunks: hunks.iter().map(|_| OnceLock::new()).collect() };
        Self {
            old_path: diff.old_path.clone(),
            new_path: diff.new_path.clone(),
            kind,
            binary: diff.diff.is_empty() && !matches!(kind, FileKind::Mode | FileKind::Renamed),
            too_large,
            colours,
            additions: count(LineKind::Added),
            deletions: count(LineKind::Removed),
            hunks,
        }
    }

    /// The syntax colours of one diff line, its hunk highlighted on the first ask; empty when the file is not coloured.
    pub fn spans(&self, hunk: usize, line: usize) -> &[(std::ops::Range<usize>, syntax::Token)] {
        let (Some(language), Some(cell), Some(lines)) = (self.colours.language, self.colours.hunks.get(hunk), self.hunks.get(hunk)) else {
            return &[];
        };
        cell.get_or_init(|| colour(language, lines)).get(line).map_or(&[], Vec::as_slice)
    }

    /// How many hunks were highlighted so far.
    #[cfg(test)]
    pub fn coloured_hunks(&self) -> usize {
        self.colours.coloured()
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

    fn line_at(&self, anchor: &Anchor) -> Option<&diff::Line> {
        let path = match anchor.side {
            Side::New => &self.new_path,
            Side::Old => &self.old_path,
        };
        if path != &anchor.path {
            return None;
        }
        self.hunks.iter().flat_map(|h| &h.lines).find(|l| match anchor.side {
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
    /// An unchanged line outside the hunk, read from the whole file: `new` is its line on the new side.
    Context {
        file: usize,
        hunk: usize,
        old: u32,
        new: u32,
    },
    /// A removed line and its added twin read as one row, the changed words side by side.
    Pair {
        file: usize,
        hunk: usize,
        removed: usize,
        added: usize,
    },
    /// A conversation under its line, where threads read in the diff: `new` and `old` number the line, `stop` counts
    /// the cursor stops of its conversations.
    Thread {
        file: usize,
        hunk: usize,
        new: Option<u32>,
        old: Option<u32>,
        stop: usize,
    },
    Gap,
}

impl Row {
    /// The file a row belongs to; the header and gaps belong to none.
    pub fn file(&self) -> Option<usize> {
        match self {
            Row::File { index, .. } => Some(*index),
            Row::Hunk { file, .. }
            | Row::Line { file, .. }
            | Row::Pair { file, .. }
            | Row::Context { file, .. }
            | Row::Thread { file, .. } => Some(*file),
            Row::Header | Row::Gap => None,
        }
    }

    /// The file and hunk a row sits in, a hunk row its own.
    pub fn hunk(&self) -> Option<(usize, usize)> {
        match self {
            Row::Hunk { file, index: hunk, .. }
            | Row::Line { file, hunk, .. }
            | Row::Pair { file, hunk, .. }
            | Row::Context { file, hunk, .. }
            | Row::Thread { file, hunk, .. } => Some((*file, *hunk)),
            Row::Header | Row::File { .. } | Row::Gap => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Review {
    /// What a fetch brought: never changed afterwards, so copies of the review share it.
    pub mr: Arc<Mr>,
    pub files: Arc<[File]>,
    pub threads: Arc<[Thread]>,
    /// The files that open folded (configured globs, too large, binary): they only count once viewed.
    pub auto_folded: Arc<BTreeSet<String>>,
    pub drafts: Vec<Draft>,
    pub viewed: BTreeSet<String>,
    pub fold: FoldState,
    /// Which changed pairs read as one row.
    pub inline: InlineRule,
    /// The old file beside the new one, `D` in the TUI; the reader's choice, kept even while the diff is too narrow.
    pub side_by_side: bool,
    /// The diff area holds two sides of code; below, side by side falls back to inline.
    pub wide: bool,
    /// One side of the diff alone, as plain code: before on the old side, after on the new, `<` `>` in the TUI.
    pub peek: Option<Side>,
    /// Lines that changed only in whitespace read as one quiet row, `W` in the TUI.
    pub quiet_whitespace: bool,
    /// Unchanged lines shown around hunks, `+` in the TUI.
    pub context: Context,
    /// Threads read under their lines rather than in the right pane: in zen and in a narrow window.
    pub threads_in_diff: bool,
    /// The threads whose every note shows in the diff, by id; the others read as one row.
    pub unfolded: BTreeSet<ThreadId>,
}

/// The files read whole, and how many lines each hunk shows beyond its own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Context {
    /// The new side of a file at the head commit, one entry per line, shared between copies of the review.
    pub texts: BTreeMap<String, Arc<Vec<String>>>,
    /// Extra lines above and below a hunk, by (file, hunk).
    pub around: BTreeMap<(usize, usize), u32>,
}

impl Review {
    pub fn new(mr: Mr, diffs: &[DiffFile], discussions: Vec<Discussion>, fold_globs: &[String]) -> Self {
        let files: Vec<File> = diffs.iter().map(File::from_diff).collect();
        let threads = threads_of(discussions, &files);
        let metas: Vec<FileMeta> = files.iter().map(File::meta).collect();
        let fold = FoldState::initial(&metas, fold_globs);
        Self {
            mr: Arc::new(mr),
            files: files.into(),
            threads: threads.into(),
            auto_folded: Arc::new(fold.files.keys().cloned().collect()),
            drafts: vec![],
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

    pub fn with_mr(self, mr: Mr) -> Self {
        Self { mr: Arc::new(mr), ..self }
    }

    pub fn with_inline(self, inline: InlineRule) -> Self {
        Self { inline, ..self }
    }

    pub fn with_side_by_side(self, side_by_side: bool) -> Self {
        Self { side_by_side, ..self }
    }

    pub fn with_wide(self, wide: bool) -> Self {
        Self { wide, ..self }
    }

    pub fn with_peek(self, peek: Option<Side>) -> Self {
        Self { peek, ..self }
    }

    /// Side by side was chosen, the diff is wide enough for it and no peek reads one side alone.
    pub fn shows_side_by_side(&self) -> bool {
        self.side_by_side && self.wide && self.peek.is_none()
    }

    pub fn with_threads_in_diff(self, threads_in_diff: bool) -> Self {
        Self { threads_in_diff, ..self }
    }

    pub fn with_unfolded(self, unfolded: BTreeSet<ThreadId>) -> Self {
        Self { unfolded, ..self }
    }

    /// Threads hang under their lines: asked for, and the diff is not side by side, where the pane keeps them.
    pub fn reads_threads_in_diff(&self) -> bool {
        self.threads_in_diff && !self.shows_side_by_side()
    }

    fn shows_line(&self, line: &diff::Line) -> bool {
        match self.peek {
            None => true,
            Some(Side::Old) => line.kind != LineKind::Added,
            Some(Side::New) => line.kind != LineKind::Removed,
        }
    }

    /// The removed and added lines of `hunk` read as one row; none while a peek shows one side.
    fn pairs(&self, hunk: &Hunk) -> Vec<(usize, usize)> {
        if self.peek.is_some() {
            return vec![];
        }
        let mut pairs = if self.shows_side_by_side() { words::side_by_side_pairs(hunk) } else { words::inline_pairs(hunk, self.inline) };
        if self.quiet_whitespace {
            let quiet: Vec<(usize, usize)> = words::whitespace_pairs(hunk).into_iter().filter(|pair| !pairs.contains(pair)).collect();
            pairs.extend(quiet);
        }
        pairs
    }

    pub fn with_context(self, context: Context) -> Self {
        Self { context, ..self }
    }

    pub fn with_quiet_whitespace(self, quiet_whitespace: bool) -> Self {
        Self { quiet_whitespace, ..self }
    }

    pub fn with_fold(self, fold: FoldState) -> Self {
        Self { fold, ..self }
    }

    pub fn with_viewed(self, viewed: BTreeSet<String>) -> Self {
        Self { viewed, ..self }
    }

    /// Viewed files with the fingerprint of the change the reader saw, as the cache keeps them.
    pub fn viewed_fingerprints(&self) -> BTreeMap<String, String> {
        self.files.iter().filter(|f| self.viewed.contains(&f.new_path)).map(|f| (f.new_path.clone(), f.fingerprint())).collect()
    }

    /// The saved viewed files that still show the change the reader saw: one pushed to since is to read again.
    pub fn still_viewed(&self, saved: &BTreeMap<String, String>) -> BTreeSet<String> {
        self.files.iter().filter(|f| saved.get(&f.new_path) == Some(&f.fingerprint())).map(|f| f.new_path.clone()).collect()
    }

    pub fn progress(&self) -> Progress {
        Progress::new(self.files.len(), &self.viewed, &self.auto_folded)
    }

    pub fn with_drafts(self, drafts: Vec<Draft>) -> Self {
        Self { drafts, ..self }
    }

    /// The colours already highlighted in `old` for every file that did not change, so a refresh highlights only what moved.
    pub fn with_colours_kept(self, old: &Arc<[File]>) -> Self {
        if Arc::ptr_eq(&self.files, old) {
            return self;
        }
        let by_path: BTreeMap<&str, &File> = old.iter().map(|file| (file.new_path.as_str(), file)).collect();
        let kept = |file: &File| by_path.get(file.new_path.as_str()).copied().filter(|old| *old == file).unwrap_or(file).clone();
        Self { files: self.files.iter().map(kept).collect(), ..self }
    }

    /// The same diff with fresh threads, for the cheap discussions poll; threads equal to the current ones stay the same
    /// shared slice, so nothing built on them is built again.
    pub fn with_discussions(self, discussions: Vec<Discussion>) -> Self {
        let threads = threads_of(discussions, &self.files);
        if *self.threads == *threads {
            return self;
        }
        Self { threads: threads.into(), ..self }
    }

    /// Every note in every thread, to notice new ones between two polls.
    pub fn note_count(&self) -> usize {
        self.threads.iter().map(|t| t.notes.len()).sum()
    }

    /// The thread flipped locally, ahead of the forge's answer.
    pub fn with_resolved(self, id: &ThreadId, resolved: bool) -> Self {
        let threads = self.threads.iter().map(|t| if t.id == *id { Thread { resolved, ..t.clone() } } else { t.clone() }).collect();
        Self { threads, ..self }
    }

    /// My reaction `emoji` on note `index` of thread `id` added or taken off, ahead of the forge's answer.
    pub fn with_reaction(self, id: &ThreadId, index: usize, emoji: crate::forge::Emoji, on: bool) -> Self {
        let react =
            |note: &crate::forge::Note| crate::forge::Note { reactions: crate::forge::toggled(&note.reactions, emoji, on), ..note.clone() };
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
    pub fn with_approved(self, approved: bool, me: &str) -> Self {
        let approvals = self.mr.approvals.with_mine(approved, me);
        Self { mr: Arc::new(Mr { approvals, ..(*self.mr).clone() }), ..self }
    }

    pub fn unresolved(&self) -> usize {
        self.threads.iter().filter(|t| t.unresolved()).count()
    }

    pub fn thread(&self, id: &ThreadId) -> Option<&Thread> {
        self.threads.iter().find(|t| t.id == *id)
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
            .filter(|(_, d)| d.anchor.as_ref().is_some_and(|a| !self.files.iter().any(|f| f.line_at(a).is_some())))
            .map(|(i, _)| i)
            .collect()
    }

    pub fn outdated(&self, path: &str) -> Vec<&Thread> {
        self.threads.iter().filter(|t| t.outdated && t.anchor.as_ref().is_some_and(|a| a.path == path)).collect()
    }

    pub fn rows(&self) -> Vec<Row> {
        let rows = self.code_rows();
        if !self.reads_threads_in_diff() {
            return rows;
        }
        let markers = self.markers();
        let with_threads = |row: Row| {
            let threads = self.thread_rows(&markers, &row);
            std::iter::once(row).chain(threads)
        };
        rows.into_iter().flat_map(with_threads).collect()
    }

    /// The conversations of a marked line, a row for each of their cursor stops.
    fn thread_rows(&self, markers: &Markers, row: &Row) -> Vec<Row> {
        let (Some((file, hunk)), Some(_)) = (row.hunk(), self.marker_of(markers, row)) else { return vec![] };
        let Some(place @ Place::Line { new, old, .. }) = self.place_of(row) else { return vec![] };
        let stops = self.stops(&self.conversations(&place)).len();
        (0..stops).map(|stop| Row::Thread { file, hunk, new, old, stop }).collect()
    }

    fn code_rows(&self) -> Vec<Row> {
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
        let text = self.context.texts.get(&file.new_path);
        let mut shown_until = 0;
        for (hunk_index, hunk) in file.hunks.iter().enumerate() {
            let open = self.fold.hunk_is_open(&file.new_path, hunk_index);
            rows.push(Row::Hunk { file: index, index: hunk_index, open });
            if !open {
                continue;
            }
            let extra = self.context.around.get(&(index, hunk_index)).copied().unwrap_or(0);
            if text.is_some() {
                let from = hunk.new_start.saturating_sub(extra).max(shown_until + 1).max(1);
                let offset = i64::from(hunk.old_start) - i64::from(hunk.new_start);
                rows.extend((from..hunk.new_start).map(|new| context_row(index, hunk_index, offset, new)));
            }
            let pairs = self.pairs(hunk);
            for (line_index, line) in hunk.lines.iter().enumerate() {
                if !self.shows_line(line) || pairs.iter().any(|&(_, added)| added == line_index) {
                    continue;
                }
                if let Some(&(removed, added)) = pairs.iter().find(|&&(removed, _)| removed == line_index) {
                    rows.push(Row::Pair { file: index, hunk: hunk_index, removed, added });
                } else {
                    rows.push(Row::Line { file: index, hunk: hunk_index, index: line_index });
                }
            }
            let last_new = hunk.lines.iter().filter_map(|l| l.new).max().unwrap_or(hunk.new_start.saturating_sub(1));
            let last_old = hunk.lines.iter().filter_map(|l| l.old).max().unwrap_or(hunk.old_start.saturating_sub(1));
            shown_until = last_new;
            if let Some(text) = text {
                let next_start = file.hunks.get(hunk_index + 1).map_or(u32::MAX, |h| h.new_start);
                let until = (last_new + extra).min(text.len() as u32).min(next_start.saturating_sub(1));
                let offset = i64::from(last_old) - i64::from(last_new);
                rows.extend((last_new + 1..=until).map(|new| context_row(index, hunk_index, offset, new)));
                shown_until = shown_until.max(until);
            }
        }
    }
}

/// Line `new` of the file; `offset` is how far the old side's numbering sits from the new one's there.
fn context_row(file: usize, hunk: usize, offset: i64, new: u32) -> Row {
    let old = (i64::from(new) + offset).max(0) as u32;
    Row::Context { file, hunk, old, new }
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

    fn charge() -> DiffFile {
        DiffFile {
            diff: include_str!("fixtures/charge.diff").to_owned(),
            old_path: "src/pay/charge.rs".into(),
            new_path: "src/pay/charge.rs".into(),
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

    fn has_span(file: &File, line: usize, text: &str, token: syntax::Token) -> bool {
        let written = &file.hunks[0].lines[line].text;
        file.spans(0, line).iter().any(|(r, t)| &written[r.clone()] == text && *t == token)
    }

    #[test]
    fn typescript_files_carry_syntax_per_line_and_other_files_none() {
        let file = File::from_diff(&cart());
        assert!(has_span(&file, 1, "const", syntax::Token::Keyword));
        assert!(has_span(&file, 4, "\"total\"", syntax::Token::String));
        assert!(File::from_diff(&charge()).spans(0, 1).is_empty(), "no grammar for .rs yet");
        assert!(File::from_diff(&DiffFile { too_large: true, ..cart() }).spans(0, 1).is_empty(), "too large files stay plain");
    }

    #[test]
    fn a_hunk_is_highlighted_the_first_time_one_of_its_lines_is_read() {
        let file = File::from_diff(&cart());
        assert_eq!(file.coloured_hunks(), 0, "parsing a diff highlights nothing");
        assert!(has_span(&file, 4, "\"total\"", syntax::Token::String));
        assert_eq!(file.coloured_hunks(), 1);
    }

    #[test]
    fn a_refresh_keeps_the_colours_of_unchanged_files_and_colours_changed_ones_afresh() {
        let edited = DiffFile { diff: cart().diff.replace("\"total\"", "total + 1"), ..cart() };
        let other = DiffFile { old_path: "src/other.ts".into(), new_path: "src/other.ts".into(), ..cart() };
        let old = Review::new(mr(), &[cart(), other.clone()], vec![], &[]);
        old.files.iter().for_each(|file| assert_ne!(file.spans(0, 1), []));
        let fresh = Review::new(mr(), &[edited.clone(), other], vec![], &[]).with_colours_kept(&old.files);
        assert_eq!(fresh.files[1].coloured_hunks(), 1, "the unchanged file keeps its colours");
        assert_eq!(fresh.files[0].coloured_hunks(), 0, "the changed file is highlighted again when read");
        assert!(!has_span(&fresh.files[0], 4, "\"total\"", syntax::Token::String), "no colour of the old text survives");
        assert_eq!(fresh.files[0].spans(0, 4), File::from_diff(&edited).spans(0, 4));
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
    fn counts_additions_and_deletions() {
        let file = File::from_diff(&charge());
        assert_eq!((file.additions, file.deletions, file.hunks.len()), (4, 2, 2));
    }

    #[test]
    fn an_empty_diff_is_binary_unless_it_is_a_mode_change_or_a_rename() {
        let empty = |diff: DiffFile| File::from_diff(&DiffFile { diff: String::new(), ..diff }).binary;
        assert!(empty(charge()));
        assert!(empty(DiffFile { change: FileKind::Added, ..charge() }));
        assert!(!empty(DiffFile { change: FileKind::Mode, ..charge() }));
        assert!(!empty(DiffFile { change: FileKind::Renamed, old_path: "src/pay/old.rs".into(), ..charge() }));
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
        let far = review.thread(&"9f2c0aa1d4e5b6c7".into()).unwrap();
        assert!(far.outdated, "line 57 is not in the diff");
        assert!(!review.thread(&"c0ffee00c0ffee00".into()).unwrap().outdated);
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
        let lines = review.files[0].hunks.iter().map(|h| h.lines.len()).sum::<usize>();
        assert_eq!(rows.len(), 1 + 2 + 2 + 2 + lines, "header, gaps, files, hunks and lines only: conversations live in the pane");
        assert_eq!(rows.last(), Some(&Row::File { index: 1, open: false }), "the lock file starts folded");
        assert_eq!(rows.iter().filter(|r| matches!(r, Row::Gap)).count(), 2);
    }

    fn thread_stops(review: &Review) -> Vec<(Option<u32>, Option<u32>, usize)> {
        let stop = |row: &Row| if let Row::Thread { new, old, stop, .. } = row { Some((*new, *old, *stop)) } else { None };
        review.rows().iter().filter_map(stop).collect()
    }

    #[test]
    fn where_threads_read_in_the_diff_a_marked_line_is_followed_by_a_row_per_cursor_stop() {
        let anchor = Anchor { path: "src/pay/charge.rs".into(), side: Side::Old, line: 13 };
        let drafts = vec![Draft::reply("c0ffee00c0ffee00".into(), "agreed"), Draft::new(Some(anchor), "nit")];
        let review = review().with_drafts(drafts).with_threads_in_diff(true);
        let rows = review.rows();
        let line = rows.iter().position(|row| *row == Row::Line { file: 0, hunk: 0, index: 1 }).unwrap();
        let under = |stop| Row::Thread { file: 0, hunk: 0, new: None, old: Some(13), stop };
        assert_eq!(rows[line + 1..line + 4], [under(0), under(1), Row::Line { file: 0, hunk: 0, index: 2 }], "my draft, then the thread");
        assert_eq!(review.place_of(&under(1)), review.place_of(&rows[line]), "a thread row stands for its line");
        let unfolded = review.clone().with_unfolded(BTreeSet::from(["c0ffee00c0ffee00".into()]));
        assert_eq!(thread_stops(&unfolded), [(None, Some(13), 0), (None, Some(13), 1), (None, Some(13), 2)], "its note and my reply");
        let folded_hunk = review.clone().with_fold(review.fold.toggle_hunk("src/pay/charge.rs", 0));
        assert_eq!(thread_stops(&folded_hunk), [], "a folded hunk hides its threads with its lines");
    }

    #[test]
    fn threads_stay_out_of_the_rows_in_the_pane_layouts_and_side_by_side() {
        assert_eq!(thread_stops(&review()), [], "the pane holds them");
        let side_by_side = review().with_threads_in_diff(true).with_side_by_side(true);
        assert!(side_by_side.shows_side_by_side() && thread_stops(&side_by_side).is_empty());
        assert_eq!(thread_stops(&side_by_side.with_wide(false)).len(), 1, "too narrow for two sides: inline, threads under their lines");
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
    fn a_closed_hunk_keeps_its_row_and_hides_its_lines() {
        let review = review();
        let folded = review.clone().with_fold(review.fold.toggle_hunk("src/pay/charge.rs", 0));
        let rows = folded.rows();
        assert_eq!(rows[3], Row::Hunk { file: 0, index: 0, open: false });
        assert_eq!(rows[4], Row::Hunk { file: 0, index: 1, open: true });
        assert!(rows.contains(&Row::Line { file: 0, hunk: 1, index: 0 }));
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

    fn review_of_one(diff: &str) -> Review {
        let diff = DiffFile { diff: diff.into(), new_path: "a.rs".into(), old_path: "a.rs".into(), ..DiffFile::default() };
        Review::new(mr(), &[diff], vec![], &[])
    }

    /// The diff rows of the first hunk as `(removed, added)` for pairs and `(index, index)` for lines.
    fn line_rows(review: &Review) -> Vec<(usize, usize)> {
        let pick = |r: &Row| match r {
            Row::Line { index, .. } => Some((*index, *index)),
            Row::Pair { removed, added, .. } => Some((*removed, *added)),
            _ => None,
        };
        review.rows().iter().filter_map(pick).collect()
    }

    #[test]
    fn quiet_whitespace_pairs_a_reindented_line_the_inline_rule_leaves_apart() {
        let review = review_of_one("@@ -1 +1 @@\n-\t\t\tone();\n+            one();\n");
        assert_eq!(line_rows(&review), [(0, 0), (1, 1)], "too much changed to read inline");
        assert_eq!(line_rows(&review.with_quiet_whitespace(true)), [(0, 1)], "W reads them as one row");
    }

    #[test]
    fn side_by_side_sets_each_removed_line_beside_an_added_one_and_the_longer_run_alone() {
        let review = review_of_one("@@ -1,7 +1,6 @@\n a\n-b\n-c\n-d\n+B\n e\n-f\n+F\n+G\n+H\n").with_side_by_side(true);
        assert_eq!(line_rows(&review), [(0, 0), (1, 4), (2, 2), (3, 3), (5, 5), (6, 7), (8, 8), (9, 9)]);
        let folded = review.clone().with_fold(review.fold.toggle_hunk("a.rs", 0));
        assert!(line_rows(&folded).is_empty(), "a folded hunk hides its rows side by side too");
    }

    #[test]
    fn a_peek_keeps_one_side_of_every_hunk_as_plain_lines() {
        let review = review_of_one("@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n").with_side_by_side(true);
        assert_eq!(line_rows(&review.clone().with_peek(Some(Side::Old))), [(0, 0), (1, 1), (3, 3)], "before: no added line, no pair");
        assert_eq!(line_rows(&review.clone().with_peek(Some(Side::New))), [(0, 0), (2, 2), (3, 3)], "after: no removed line");
        assert!(!review.with_peek(Some(Side::New)).shows_side_by_side(), "a peek reads one side, never two");
    }

    #[test]
    fn side_by_side_falls_back_to_the_inline_rows_while_the_diff_is_narrow() {
        let review = review_of_one("@@ -1,2 +1,1 @@\n-let b = 2;\n-gone\n+let b = 20;\n");
        let narrow = review.clone().with_side_by_side(true).with_wide(false);
        assert!(narrow.side_by_side && !narrow.shows_side_by_side(), "the choice stays");
        assert_eq!(line_rows(&narrow), line_rows(&review));
        assert_eq!(line_rows(&narrow.with_wide(true)), [(0, 2), (1, 1)]);
    }

    fn contexts(review: &Review) -> Vec<(usize, u32, u32)> {
        review
            .rows()
            .iter()
            .filter_map(|r| if let Row::Context { hunk, old, new, .. } = r { Some((*hunk, *old, *new)) } else { None })
            .collect()
    }

    #[test]
    fn context_lines_come_from_the_whole_file_numbered_on_both_sides_and_never_twice() {
        let review = review();
        let path = review.files[0].new_path.clone();
        let text: Vec<String> = (1..=60).map(|n| format!("line {n}")).collect();
        let context = Context { texts: BTreeMap::from([(path, Arc::new(text))]), around: BTreeMap::from([((0, 0), 10), ((0, 1), 20)]) };
        let rows = contexts(&review.clone().with_context(context.clone()));
        let above_first: Vec<u32> = rows.iter().filter(|(h, _, n)| *h == 0 && *n < 12).map(|(_, _, n)| *n).collect();
        assert_eq!(above_first, (2..12).collect::<Vec<_>>(), "ten lines above the first hunk");
        assert!(rows.iter().any(|&(h, o, n)| h == 0 && n == 17 && o == 16), "below the first hunk, old numbers follow its offset");
        let second_above: Vec<u32> = rows.iter().filter(|(h, _, n)| *h == 1 && *n < 41).map(|(_, _, n)| *n).collect();
        assert_eq!(second_above.first(), Some(&27), "the second hunk starts after what the first one showed");
        assert!(rows.iter().any(|&(h, o, n)| h == 1 && n == 40 && o == 39), "the second hunk's offset: old is one less");
        assert!(rows.iter().all(|(_, _, n)| *n <= 60), "never past the end of the file");
        let side_by_side = review.with_side_by_side(true).with_context(context);
        assert_eq!(contexts(&side_by_side), rows, "side by side shows the same lines around its hunks");
    }

    #[test]
    fn no_context_is_shown_before_the_file_arrives() {
        let context = Context { texts: BTreeMap::new(), around: BTreeMap::from([((0, 0), 10)]) };
        assert_eq!(contexts(&review().with_context(context)), [] as [(usize, u32, u32); 0]);
    }
}
