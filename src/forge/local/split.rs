//! `git diff --raw -p -z` cut into one `DiffFile` per path: the raw records name the paths and kinds, the patch that
//! follows holds the bodies in the same order.
use super::super::{DiffFile, FileKind};

/// So the user's config cannot change what `git diff` prints.
pub(super) const DIFF_FLAGS: [&str; 7] =
    ["--no-ext-diff", "--no-color", "--no-textconv", "-M", "--src-prefix=a/", "--dst-prefix=b/", "--submodule=short"];
/// Past this many lines a file's body is withheld, as a forge withholds a huge one.
const MAX_LINES: usize = 20_000;

/// One raw record: `:old_mode new_mode old_sha new_sha STATUS`, then one path, or two after a rename.
struct Record<'a> {
    modes_differ: bool,
    status: char,
    old_path: &'a str,
    new_path: &'a str,
}

impl Record<'_> {
    /// How many `diff --git` sections the patch holds for it: a type change is a deletion then a creation, and an
    /// unmerged path has none.
    fn sections(&self) -> usize {
        match self.status {
            'T' => 2,
            'U' => 0,
            _ => 1,
        }
    }

    fn kind(&self, body: &str) -> FileKind {
        match self.status {
            'A' => FileKind::Added,
            'D' => FileKind::Deleted,
            'R' => FileKind::Renamed,
            'M' if self.modes_differ && body.is_empty() => FileKind::Mode,
            _ => FileKind::Modified,
        }
    }
}

/// The files of one `git diff --raw -p -z`.
pub(super) fn files(output: &str) -> Vec<DiffFile> {
    let (records, patch) = records(output);
    let mut sections = patch.split("\ndiff --git ").filter(|section| !section.is_empty());
    let mut files = Vec::with_capacity(records.len());
    for record in records {
        let body: String = sections.by_ref().take(record.sections()).map(hunks).collect();
        files.push(file(record.old_path, record.new_path, record.kind(&body), body));
    }
    files
}

/// The file of one `git diff --no-index -- /dev/null <path>`.
pub(super) fn new_file(path: &str, patch: &str) -> DiffFile {
    file(path, path, FileKind::Added, hunks(patch).to_owned())
}

fn file(old_path: &str, new_path: &str, change: FileKind, body: String) -> DiffFile {
    let too_large = body.lines().count() > MAX_LINES;
    let diff = if too_large { String::new() } else { body };
    DiffFile { diff, old_path: old_path.to_owned(), new_path: new_path.to_owned(), change, too_large }
}

/// The raw records at the head of `output`, and the patch after them.
fn records(output: &str) -> (Vec<Record<'_>>, &str) {
    let mut records = vec![];
    let mut rest = output;
    while rest.starts_with(':') {
        let Some((record, after)) = next_record(rest) else { break };
        records.push(record);
        rest = after;
    }
    (records, rest.trim_start_matches('\0'))
}

fn next_record(text: &str) -> Option<(Record<'_>, &str)> {
    let (meta, rest) = text.split_once('\0')?;
    let words: Vec<&str> = meta.split(' ').collect();
    let [old_mode, new_mode, _, _, status] = words.as_slice() else { return None };
    let status = status.chars().next()?;
    let (old_path, rest) = rest.split_once('\0')?;
    let (new_path, rest) = if matches!(status, 'R' | 'C') { rest.split_once('\0')? } else { (old_path, rest) };
    Some((Record { modes_differ: old_mode != new_mode, status, old_path, new_path }, rest))
}

/// A section from its first `@@`; nothing for a binary, mode-only or pure rename section. An unmerged path's notice,
/// the one line git writes between sections, is dropped.
fn hunks(section: &str) -> &str {
    let start = section.find("\n@@ ").map(|at| at + 1).or_else(|| section.starts_with("@@ ").then_some(0));
    let body = start.map_or("", |start| &section[start..]);
    body.find("\n* Unmerged path ").map_or(body, |at| &body[..=at])
}
