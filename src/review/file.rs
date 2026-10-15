//! One changed file as the review reads it: its parsed hunks, its counts, and its syntax colours.
use super::{Anchor, Side};
use crate::diff::fold::FileMeta;
use crate::diff::words;
use crate::diff::{self, Hunk, LineKind};
use crate::forge::{DiffFile, FileKind};
use crate::syntax::{self, Spans};
use std::sync::OnceLock;

/// Above this many lines a file starts folded, whatever the forge says.
const TOO_LARGE_LINES: usize = 2000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct File {
    pub(crate) old_path: String,
    pub(crate) new_path: String,
    pub(crate) kind: FileKind,
    pub(crate) binary: bool,
    pub(crate) too_large: bool,
    pub(crate) hunks: Vec<Hunk>,
    pub(crate) additions: usize,
    pub(crate) deletions: usize,
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
    pub(crate) fn from_diff(diff: &DiffFile) -> Self {
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
    pub(crate) fn spans(&self, hunk: usize, line: usize) -> &[(std::ops::Range<usize>, syntax::Token)] {
        let (Some(language), Some(cell), Some(lines)) = (self.colours.language, self.colours.hunks.get(hunk), self.hunks.get(hunk)) else {
            return &[];
        };
        cell.get_or_init(|| colour(language, lines)).get(line).map_or(&[], Vec::as_slice)
    }

    /// How many hunks were highlighted so far.
    #[cfg(test)]
    pub(crate) fn coloured_hunks(&self) -> usize {
        self.colours.coloured()
    }

    /// Names this file's change: equal fingerprints mean nothing was pushed to it since.
    pub(crate) fn fingerprint(&self) -> String {
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

    pub(crate) fn meta(&self) -> FileMeta {
        FileMeta { path: self.new_path.clone(), too_large: self.too_large, binary: self.binary }
    }

    pub(super) fn line_at(&self, anchor: &Anchor) -> Option<&diff::Line> {
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

#[cfg(test)]
mod tests {
    use super::super::Review;
    use super::super::tests::{charge, mr};
    use super::*;

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
}
