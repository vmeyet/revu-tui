//! How many raw diff lines the rows do not account for, so that a hunk no row explains still shows up.
//!
//! A row accounts for the fields it draws, and for a whole entry when the row is about that entry appearing or going. A copy that
//! only moved in the tree has no row, so it accounts for what both its places say. Everything else counts, and a file not written
//! as its format writes it counts whole.
use super::{Change, ChangeKind, Package, PackageName, Version};
use similar::{ChangeTag, TextDiff};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

/// Past this the diff gets rougher, so the count can only grow: a hostile file must not keep the reader waiting.
/// The one thing here that reads a clock; no file a person or a tool wrote gets near it.
const DIFF_TIME_LIMIT: Duration = Duration::from_secs(5);

/// An entry's lines as its format writes them, each read through [`significant`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct EntryLines {
    /// The line naming the place it installs at.
    pub(super) key: String,
    /// The fields a row draws.
    pub(super) drawn: Vec<String>,
    /// Every other field.
    pub(super) rest: Vec<String>,
}

impl EntryLines {
    fn body(&self) -> impl Iterator<Item = &str> {
        self.drawn.iter().chain(&self.rest).map(String::as_str)
    }
}

/// A line without its indent or trailing comma; `None` when it holds only brackets, which say nothing by themselves.
pub(super) fn significant(line: &str) -> Option<&str> {
    let text = line.trim().trim_end_matches(',').trim_end();
    (!text.chars().all(|c| matches!(c, '{' | '}' | '[' | ']'))).then_some(text)
}

/// One side of the file: what the diff compares, and what its parser read from it.
pub(super) struct Side<'a> {
    pub(super) text: &'a str,
    pub(super) packages: &'a [Package],
}

/// How many times each line appears.
type Tally<'a> = BTreeMap<&'a str, usize>;

fn tally<'a>(lines: impl Iterator<Item = &'a str>) -> Tally<'a> {
    let mut counts = Tally::new();
    for line in lines {
        *counts.entry(line).or_default() += 1;
    }
    counts
}

/// The versions on one side a row is about.
type Rowed<'a> = BTreeSet<(&'a PackageName, &'a Version)>;

fn rowed_on_each_side(changes: &[Change]) -> [Rowed<'_>; 2] {
    let mut sides = [Rowed::new(), Rowed::new()];
    for Change { name, kind } in changes {
        let [base, head] = &mut sides;
        match kind {
            ChangeKind::SameVersion { version, .. } => {
                base.insert((name, version));
                head.insert((name, version));
            }
            ChangeKind::Updated { from, to, .. } => {
                base.insert((name, from));
                head.insert((name, to));
            }
            ChangeKind::Added { version, .. } => {
                head.insert((name, version));
            }
            ChangeKind::Removed { version } => {
                base.insert((name, version));
            }
        }
    }
    sides
}

fn places_of(packages: &[Package]) -> BTreeSet<&str> {
    packages.iter().map(|p| p.place.as_str()).collect()
}

/// What the rows account for on one side: the fields they draw of a copy that stays, a whole copy that appears or goes,
/// and only the key of a copy with no row.
fn row_lines<'a>(packages: &'a [Package], other_places: &BTreeSet<&str>, rowed: &Rowed<'_>) -> Tally<'a> {
    tally(packages.iter().flat_map(|p| {
        let (has_row, stays) = (rowed.contains(&(&p.name, &p.version)), other_places.contains(p.place.as_str()));
        let key = (has_row || !stays).then_some(p.lines.key.as_str());
        let fields: Vec<&str> = match (has_row, stays) {
            (true, true) => p.lines.drawn.iter().map(String::as_str).collect(),
            (true, false) => p.lines.body().collect(),
            (false, _) => vec![],
        };
        key.into_iter().chain(fields)
    }))
}

/// The bodies of the copies at a place the other side does not have and with no row: they only moved.
fn moved_lines<'a>(packages: &'a [Package], other_places: &BTreeSet<&str>, rowed: &Rowed<'_>) -> Tally<'a> {
    let moved = packages.iter().filter(|p| !other_places.contains(p.place.as_str()) && !rowed.contains(&(&p.name, &p.version)));
    tally(moved.flat_map(|p| p.lines.body()))
}

fn in_both<'a>(one: &Tally<'a>, other: &Tally<'a>) -> Tally<'a> {
    one.iter().filter_map(|(line, count)| other.get(line).map(|there| (*line, (*count).min(*there)))).collect()
}

/// The changed lines left once every accounted one is used up.
fn not_accounted(changed: &Tally<'_>, by_rows: &Tally<'_>, by_moves: &Tally<'_>) -> usize {
    let times = |tally: &Tally<'_>, line: &str| tally.get(line).copied().unwrap_or(0);
    changed.iter().map(|(line, count)| count.saturating_sub(times(by_rows, line) + times(by_moves, line))).sum()
}

/// The raw lines that changed between the two sides and that neither the rows nor a moved copy explain.
pub(super) fn unshown_lines(base: &Side<'_>, head: &Side<'_>, changes: &[Change]) -> usize {
    let diff = TextDiff::configure().deadline(Instant::now() + DIFF_TIME_LIMIT).diff_lines(base.text, head.text);
    let changed = |tag: ChangeTag| tally(diff.iter_all_changes().filter(|c| c.tag() == tag).filter_map(|c| significant(c.value())));
    let [base_rowed, head_rowed] = rowed_on_each_side(changes);
    let (base_places, head_places) = (places_of(base.packages), places_of(head.packages));
    let moves = in_both(&moved_lines(base.packages, &head_places, &base_rowed), &moved_lines(head.packages, &base_places, &head_rowed));
    not_accounted(&changed(ChangeTag::Delete), &row_lines(base.packages, &head_places, &base_rowed), &moves)
        + not_accounted(&changed(ChangeTag::Insert), &row_lines(head.packages, &base_places, &head_rowed), &moves)
}

#[cfg(test)]
mod tests {
    use super::super::summarize;

    const NPM: &str = "package-lock.json";
    const CARGO: &str = "Cargo.lock";

    fn unshown(path: &str, base: &str, head: &str) -> usize {
        summarize(path, Some(base), Some(head)).unwrap().unshown_lines
    }

    fn pretty(entries: &str) -> String {
        let text = format!(r#"{{"lockfileVersion":3,"packages":{{"":{{"name":"widgets"}},{entries}}}}}"#);
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        serde_json::to_string_pretty(&value).unwrap()
    }

    fn fields(name: &str, version: &str, more: &str) -> String {
        format!(
            r#""version":"{version}","resolved":"https://registry.npmjs.org/{name}/-/{name}-{version}.tgz","integrity":"sha512-{name}{version}"{more}"#
        )
    }

    fn top(name: &str, version: &str, more: &str) -> String {
        format!(r#""node_modules/{name}":{{{}}}"#, fields(name, version, more))
    }

    fn nested(under: &str, name: &str, version: &str, more: &str) -> String {
        format!(r#""node_modules/{under}/node_modules/{name}":{{{}}}"#, fields(name, version, more))
    }

    #[test]
    fn a_bump_drawn_by_its_row_leaves_nothing_unshown() {
        assert_eq!(unshown(NPM, &pretty(&top("a", "1.0.0", "")), &pretty(&top("a", "1.0.1", ""))), 0);
    }

    #[test]
    fn a_new_entry_with_a_row_is_accounted_for_whole() {
        let more = r#","license":"MIT","dependencies":{"b":"^1.0.0"}"#;
        let (base, head) = (top("z", "1.0.0", ""), format!("{},{}", top("n", "1.0.0", more), top("z", "1.0.0", "")));
        assert_eq!(unshown(NPM, &pretty(&base), &pretty(&head)), 0);
    }

    #[test]
    fn a_removed_entry_with_a_row_is_accounted_for_whole() {
        let more = r#","license":"MIT""#;
        let (base, head) = (format!("{},{}", top("n", "1.0.0", more), top("z", "1.0.0", "")), top("z", "1.0.0", ""));
        assert_eq!(unshown(NPM, &pretty(&base), &pretty(&head)), 0);
    }

    #[test]
    fn a_field_no_row_draws_counts_though_no_row_changed() {
        let head = pretty(&top("a", "1.0.0", r#","bin":{"node":"evil.js"}"#));
        let summary = summarize(NPM, Some(&pretty(&top("a", "1.0.0", ""))), Some(&head)).unwrap();
        assert_eq!(
            (summary.changes.len(), summary.unshown_lines),
            (0, 4),
            "bin, its value, and both sides of the integrity line's new comma"
        );
    }

    #[test]
    fn a_field_no_row_draws_counts_beside_a_bump() {
        let head = pretty(&top("a", "1.0.1", r#","dependencies":{"evil":"*"}"#));
        assert_eq!(unshown(NPM, &pretty(&top("a", "1.0.0", "")), &head), 2);
    }

    #[test]
    fn a_change_outside_any_entry_counts() {
        let base = pretty(&top("a", "1.0.0", ""));
        assert_eq!(unshown(NPM, &base, &base.replace("widgets", "gadgets")), 2);
    }

    #[test]
    fn a_copy_that_only_moved_leaves_nothing_unshown() {
        let (base, head) = (nested("b", "a", "1.0.0", ""), nested("c", "a", "1.0.0", ""));
        assert_eq!(unshown(NPM, &pretty(&base), &pretty(&head)), 0);
    }

    #[test]
    fn a_field_slipped_into_a_copy_that_moved_counts() {
        let (base, head) = (nested("b", "a", "1.0.0", ""), nested("c", "a", "1.0.0", r#","license":"evil""#));
        assert_eq!(unshown(NPM, &pretty(&base), &pretty(&head)), 1);
    }

    #[test]
    fn entries_trading_places_in_the_file_count() {
        let (a, b) = (top("a", "1.0.0", ""), top("b", "1.0.0", ""));
        assert_eq!(unshown(NPM, &pretty(&format!("{a},{b}")), &pretty(&format!("{b},{a}"))), 8, "one entry's four lines, on each side");
    }

    #[test]
    fn a_file_on_one_line_counts_its_one_line_on_each_side() {
        let compact = |version: &str| format!(r#"{{"lockfileVersion":3,"packages":{{{}}}}}"#, top("a", version, ""));
        assert_eq!(unshown(NPM, &compact("1.0.0"), &compact("1.0.1")), 2);
    }

    #[test]
    fn a_bump_in_a_version_2_file_is_accounted_for_in_both_sections() {
        let file = |version: &str| {
            let text = format!(
                r#"{{"lockfileVersion":2,"packages":{{{}}},"dependencies":{{"a":{{{}}}}}}}"#,
                top("a", version, ""),
                fields("a", version, "")
            );
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            serde_json::to_string_pretty(&value).unwrap()
        };
        assert_eq!(unshown(NPM, &file("1.0.0"), &file("1.0.1")), 0);
    }

    #[test]
    fn identical_texts_count_nothing() {
        let same = pretty(&top("a", "1.0.0", ""));
        assert_eq!(unshown(NPM, &same, &same), 0);
    }

    fn crate_entry(name: &str, version: &str, dependencies: &[&str]) -> String {
        let source = "registry+https://github.com/rust-lang/crates.io-index";
        let list = if dependencies.is_empty() {
            String::new()
        } else {
            format!("dependencies = [\n{}]\n", dependencies.iter().map(|d| format!(" \"{d}\",\n")).collect::<String>())
        };
        format!("[[package]]\nname = \"{name}\"\nversion = \"{version}\"\nsource = \"{source}\"\nchecksum = \"{name}{version}\"\n{list}\n")
    }

    #[test]
    fn a_cargo_bump_and_its_own_dependencies_leave_nothing_unshown() {
        let (base, head) = (crate_entry("a", "1.0.0", &["b"]), crate_entry("a", "1.0.1", &["b", "c"]));
        assert_eq!(unshown(CARGO, &format!("version = 4\n\n{base}"), &format!("version = 4\n\n{head}")), 0);
    }

    #[test]
    fn a_cargo_dependency_on_an_unchanged_entry_counts() {
        let (base, head) = (crate_entry("a", "1.0.0", &[]), crate_entry("a", "1.0.0", &["evil"]));
        assert_eq!(unshown(CARGO, &format!("version = 4\n\n{base}"), &format!("version = 4\n\n{head}")), 2);
    }
}
