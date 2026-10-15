//! The crate's own source files, read at test time so a check over them also covers the files added later.
use std::path::Path;

/// One `.rs` file: its path from the folder it was found under, and its text.
pub(crate) struct SourceFile {
    pub(crate) path: String,
    pub(crate) text: String,
}

/// Every `.rs` file under `src/<folder>`, at any depth, sorted by path.
pub(crate) fn rust_files(folder: &str) -> Vec<SourceFile> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(folder);
    let mut files: Vec<SourceFile> = walk(&root)
        .into_iter()
        .map(|file| SourceFile {
            path: file.strip_prefix(&root).unwrap().to_string_lossy().into_owned(),
            text: std::fs::read_to_string(&file).unwrap(),
        })
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

fn walk(folder: &Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(folder)
        .unwrap_or_else(|e| panic!("{}: {e}", folder.display()))
        .map(|entry| entry.unwrap().path())
        .flat_map(|path| {
            if path.is_dir() {
                walk(&path)
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                vec![path]
            } else {
                Vec::new()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_files_reach_nested_folders_and_skip_other_files() {
        let paths: Vec<String> = rust_files("tui").into_iter().map(|f| f.path).collect();
        assert!(paths.contains(&"mod.rs".to_owned()));
        assert!(paths.contains(&"diff_view/line.rs".to_owned()));
        assert!(!paths.iter().any(|p| p.contains("snapshots/")), "snapshot files are not sources");
    }
}
