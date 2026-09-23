//! The files of a review as a folder tree: folders deeper than two levels start folded.
use super::File;
use std::collections::BTreeSet;

/// Folders at this depth or deeper start folded, so a large MR opens as an outline.
const OPEN_DEPTH: usize = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeRow {
    Folder { path: String, name: String, depth: usize, open: bool, files: usize },
    File { index: usize, name: String, depth: usize },
}

/// Which folders the reader flipped away from their default, by path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreeFolds {
    flipped: BTreeSet<String>,
}

impl TreeFolds {
    pub fn is_open(&self, folder: &str) -> bool {
        (depth_of(folder) < OPEN_DEPTH) != self.flipped.contains(folder)
    }

    pub fn toggled(&self, folder: &str) -> Self {
        let mut flipped = self.flipped.clone();
        if !flipped.remove(folder) {
            flipped.insert(folder.to_owned());
        }
        Self { flipped }
    }

    /// Opens every folder above `path`, so a file jumped to is visible in the tree.
    pub fn revealing(&self, path: &str) -> Self {
        let mut next = self.clone();
        for folder in folders_above(path) {
            if !next.is_open(&folder) {
                next = next.toggled(&folder);
            }
        }
        next
    }
}

/// The rows a tree shows: every folder whose parents are open, and the files inside open folders.
pub fn rows(files: &[File], folds: &TreeFolds) -> Vec<TreeRow> {
    let mut order: Vec<(usize, &str)> = files.iter().enumerate().map(|(i, f)| (i, f.new_path.as_str())).collect();
    order.sort_by_cached_key(|(_, path)| sort_key(path));
    let mut rows = vec![];
    let mut shown: BTreeSet<String> = BTreeSet::new();
    for (index, path) in order {
        let folders = folders_above(path);
        let mut visible = true;
        for folder in &folders {
            if !visible {
                break;
            }
            if shown.insert(folder.clone()) {
                let files_inside = files.iter().filter(|f| f.new_path.starts_with(&format!("{folder}/"))).count();
                let open = folds.is_open(folder);
                rows.push(TreeRow::Folder {
                    path: folder.clone(),
                    name: last(folder).to_owned(),
                    depth: depth_of(folder),
                    open,
                    files: files_inside,
                });
            }
            visible = folds.is_open(folder);
        }
        if visible {
            rows.push(TreeRow::File { index, name: last(path).to_owned(), depth: folders.len() });
        }
    }
    rows
}

/// Folders before files at every level, then by name, as file browsers show them.
fn sort_key(path: &str) -> Vec<(bool, String)> {
    let parts: Vec<&str> = path.split('/').collect();
    parts.iter().enumerate().map(|(i, part)| (i + 1 == parts.len(), (*part).to_owned())).collect()
}

fn folders_above(path: &str) -> Vec<String> {
    let parts: Vec<&str> = path.split('/').collect();
    (1..parts.len()).map(|n| parts[..n].join("/")).collect()
}

fn depth_of(folder: &str) -> usize {
    folder.matches('/').count()
}

fn last(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn files(paths: &[&str]) -> Vec<File> {
        paths
            .iter()
            .map(|p| File::from_diff(&crate::forge::DiffFile { new_path: (*p).into(), old_path: (*p).into(), ..Default::default() }))
            .collect()
    }

    fn names(rows: &[TreeRow]) -> Vec<String> {
        rows.iter()
            .map(|r| match r {
                TreeRow::Folder { name, depth, open, .. } => format!("{}{}{name}/", "  ".repeat(*depth), if *open { "▾" } else { "▸" }),
                TreeRow::File { name, depth, .. } => format!("{}{name}", "  ".repeat(*depth)),
            })
            .collect()
    }

    #[test]
    fn folders_open_to_depth_two_and_come_before_files() {
        let files = files(&["src/pay/charge.rs", "Cargo.lock", "src/pay/deep/a.rs", "src/main.rs"]);
        assert_eq!(names(&rows(&files, &TreeFolds::default())), ["▾src/", "  ▾pay/", "    ▸deep/", "    charge.rs", "  main.rs", "Cargo.lock"]);
    }

    #[test]
    fn a_folded_folder_hides_what_is_inside_and_counts_it() {
        let files = files(&["src/pay/charge.rs", "src/main.rs"]);
        let folds = TreeFolds::default().toggled("src");
        let rows = rows(&files, &folds);
        assert_eq!(names(&rows), ["▸src/"]);
        assert!(matches!(&rows[0], TreeRow::Folder { files: 2, .. }));
    }

    #[test]
    fn revealing_opens_every_folder_above_a_file() {
        let files = files(&["src/pay/deep/a.rs"]);
        let folds = TreeFolds::default().toggled("src").revealing("src/pay/deep/a.rs");
        assert_eq!(names(&rows(&files, &folds)), ["▾src/", "  ▾pay/", "    ▾deep/", "      a.rs"]);
    }
}
