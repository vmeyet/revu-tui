//! Diffs and drawing helpers the diff view's tests share.
use crate::diff;
use crate::forge::DiffFile;
use crate::review::{File, Review};
use ratatui::text::Span;

pub(super) fn spans_text(spans: &[Span<'_>]) -> String {
    spans.iter().map(|s| s.content.to_string()).collect()
}

/// One changed number, marked as a review loads it.
pub(super) fn total_pair() -> diff::Hunk {
    diff::words::mark(&diff::parse("@@ -1 +1 @@\n-const total = 1;\n+const total = 2;\n")[0])
}

pub(super) fn cart() -> File {
    File::from_diff(&DiffFile {
        diff: include_str!("../../review/fixtures/cart.ts.diff").to_owned(),
        old_path: "src/cart.ts".into(),
        new_path: "src/cart.ts".into(),
        ..DiffFile::default()
    })
}

pub(super) fn painted(spans: &[Span<'_>]) -> String {
    spans
        .iter()
        .filter(|s| !s.content.trim().is_empty())
        .map(|s| format!("{:?}/{:?} {:?}", s.style.fg, s.style.bg, s.content))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn side_by_side_of(diff: &str, path: &str) -> Review {
    let diff = DiffFile { diff: diff.into(), old_path: path.into(), new_path: path.into(), ..Default::default() };
    Review::new((*crate::review::tests::review().mr).clone(), &[diff], vec![], &[]).with_side_by_side(true)
}

pub(super) fn long_added_line() -> Review {
    let diff = DiffFile {
        diff: format!("@@ -1 +1,2 @@\n a\n+{}\n", "x".repeat(40)),
        old_path: "a.txt".into(),
        new_path: "a.txt".into(),
        ..Default::default()
    };
    Review::new((*crate::review::tests::review().mr).clone(), &[diff], vec![], &[])
}
