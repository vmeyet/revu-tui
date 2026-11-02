//! What a lockfile change means package by package: pure, the two texts in and a [`Summary`] out.
mod cargo;
mod npm;

use anyhow::{Context as _, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// One entry per format: adding a lockfile is a parser and a row here.
const FORMATS: [Format; 2] =
    [Format { file_name: "Cargo.lock", parse: cargo::parse }, Format { file_name: "package-lock.json", parse: npm::parse }];

struct Format {
    file_name: &'static str,
    parse: fn(&str) -> Result<Vec<Package>>,
}

fn format_of(path: &str) -> Option<&'static Format> {
    let file_name = path.rsplit('/').next()?;
    FORMATS.iter().find(|format| format.file_name == file_name)
}

#[expect(dead_code, reason = "the diff view does not call it yet")]
pub(crate) fn is_lockfile(path: &str) -> bool {
    format_of(path).is_some()
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct PackageName(String);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Version(String);

impl fmt::Display for PackageName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Version {
    /// Major, minor and patch; `None` when the version is not semver.
    fn numbers(&self) -> Option<[u64; 3]> {
        let release = self.0.split(['-', '+']).next()?;
        let parts: Vec<u64> = release.split('.').map(str::parse).collect::<Result<_, _>>().ok()?;
        parts.try_into().ok()
    }

    /// Whether both are semver and this one comes before `other`.
    #[cfg_attr(not(test), expect(dead_code, reason = "the diff view does not call it yet"))]
    pub(crate) fn is_older_than(&self, other: &Self) -> bool {
        self.numbers().zip(other.numbers()).is_some_and(|(mine, theirs)| mine < theirs)
    }
}

/// What a lockfile pins a version to; the same version with another origin is other code.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Origin {
    checksum: Option<String>,
    source: Option<String>,
}

/// One installed copy, as a parser reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Package {
    name: PackageName,
    version: Version,
    origin: Origin,
}

/// How far an update goes, the biggest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Bump {
    Breaking,
    Minor,
    Patch,
    /// Not semver, or only a pre-release or build part moved.
    Other,
}

impl Bump {
    fn between(from: &Version, to: &Version) -> Self {
        let (Some([major, minor, patch]), Some([new_major, new_minor, new_patch])) = (from.numbers(), to.numbers()) else {
            return Self::Other;
        };
        if major != new_major || (major == 0 && minor != new_minor) || (major == 0 && minor == 0 && patch != new_patch) {
            Self::Breaking
        } else if minor != new_minor {
            Self::Minor
        } else if patch != new_patch {
            Self::Patch
        } else {
            Self::Other
        }
    }
}

/// What differs under one version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Drift {
    Checksum,
    Source,
    ChecksumAndSource,
}

impl Drift {
    fn between(base: &BTreeSet<&Origin>, head: &BTreeSet<&Origin>) -> Self {
        fn checksums<'a>(set: &BTreeSet<&'a Origin>) -> BTreeSet<Option<&'a str>> {
            set.iter().map(|o| o.checksum.as_deref()).collect()
        }
        fn sources<'a>(set: &BTreeSet<&'a Origin>) -> BTreeSet<Option<&'a str>> {
            set.iter().map(|o| o.source.as_deref()).collect()
        }
        match (checksums(base) != checksums(head), sources(base) != sources(head)) {
            (true, false) => Self::Checksum,
            (false, true) => Self::Source,
            _ => Self::ChecksumAndSource,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ChangeKind {
    /// The same version, other code: the row a reader must not miss.
    Warning {
        version: Version,
        drift: Drift,
    },
    Updated {
        from: Version,
        to: Version,
        bump: Bump,
    },
    Added {
        version: Version,
    },
    Removed {
        version: Version,
    },
}

impl ChangeKind {
    fn rank(&self) -> u8 {
        match self {
            Self::Warning { .. } => 0,
            Self::Updated { bump, .. } => 1 + *bump as u8,
            Self::Added { .. } => 5,
            Self::Removed { .. } => 6,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Change {
    pub(crate) name: PackageName,
    pub(crate) kind: ChangeKind,
}

/// Every package change of one lockfile, in reading order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Summary {
    pub(crate) changes: Vec<Change>,
}

/// The summary between the two sides of the lockfile at `path`; a side is `None` where the file does not exist.
#[cfg_attr(not(test), expect(dead_code, reason = "the diff view does not call it yet"))]
pub(crate) fn summarize(path: &str, base: Option<&str>, head: Option<&str>) -> Result<Summary> {
    let format = format_of(path).with_context(|| format!("{path} is not a lockfile"))?;
    let read = |text: Option<&str>| text.map_or_else(|| Ok(vec![]), format.parse);
    Ok(compare(&read(base)?, &read(head)?))
}

type Versions<'a> = BTreeMap<&'a Version, BTreeSet<&'a Origin>>;

/// Copies of a version collapse into one: moving a copy in the tree changes nothing a reader cares about.
fn by_name(packages: &[Package]) -> BTreeMap<&PackageName, Versions<'_>> {
    let mut names: BTreeMap<&PackageName, Versions<'_>> = BTreeMap::new();
    for package in packages {
        names.entry(&package.name).or_default().entry(&package.version).or_default().insert(&package.origin);
    }
    names
}

fn compare(base: &[Package], head: &[Package]) -> Summary {
    let (base, head) = (by_name(base), by_name(head));
    let none = Versions::new();
    let names: BTreeSet<_> = base.keys().chain(head.keys()).copied().collect();
    let mut changes: Vec<Change> = names
        .into_iter()
        .flat_map(|name| {
            let kinds = package_changes(base.get(name).unwrap_or(&none), head.get(name).unwrap_or(&none));
            kinds.into_iter().map(|kind| Change { name: name.clone(), kind })
        })
        .collect();
    changes.sort_by_key(|c| c.kind.rank());
    Summary { changes }
}

fn package_changes(base: &Versions<'_>, head: &Versions<'_>) -> Vec<ChangeKind> {
    let warnings = base.iter().filter_map(|(&version, origins)| {
        let now = head.get(version).filter(|now| *now != origins)?;
        Some(ChangeKind::Warning { version: version.clone(), drift: Drift::between(origins, now) })
    });
    warnings.chain(moved_versions(only_in(base, head), only_in(head, base))).collect()
}

fn only_in<'a>(versions: &Versions<'a>, other: &Versions<'a>) -> Vec<&'a Version> {
    let mut only: Vec<&Version> = versions.keys().filter(|v| !other.contains_key(*v)).copied().collect();
    only.sort_by_key(|v| (v.numbers(), v.0.as_str()));
    only
}

/// A gone version pairs with a new one it is compatible with; what is left pairs only when the counts match,
/// since an extra version is a package added or removed, not an update.
fn moved_versions(gone: Vec<&Version>, mut new: Vec<&Version>) -> Vec<ChangeKind> {
    let updated = |from: &Version, to: &Version| ChangeKind::Updated { from: from.clone(), to: to.clone(), bump: Bump::between(from, to) };
    let mut changes = vec![];
    let mut left = vec![];
    for from in gone {
        match new.iter().position(|to| Bump::between(from, to) != Bump::Breaking) {
            Some(at) => changes.push(updated(from, new.remove(at))),
            None => left.push(from),
        }
    }
    if left.len() == new.len() {
        changes.extend(left.into_iter().zip(new).map(|(from, to)| updated(from, to)));
        return changes;
    }
    changes.extend(new.into_iter().map(|v| ChangeKind::Added { version: v.clone() }));
    changes.extend(left.into_iter().map(|v| ChangeKind::Removed { version: v.clone() }));
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(name: &str, version: &str, checksum: &str) -> Package {
        let origin = Origin { checksum: Some(checksum.into()), source: Some("registry".into()) };
        Package { name: PackageName(name.into()), version: Version(version.into()), origin }
    }

    fn sourced(version: &str, source: &str) -> Package {
        let same = package("a", version, "same");
        Package { origin: Origin { source: Some(source.into()), ..same.origin }, ..same }
    }

    fn version(text: &str) -> Version {
        Version(text.into())
    }

    fn updated(from: &str, to: &str, bump: Bump) -> ChangeKind {
        ChangeKind::Updated { from: version(from), to: version(to), bump }
    }

    fn kinds(base: &[Package], head: &[Package]) -> Vec<ChangeKind> {
        compare(base, head).changes.into_iter().map(|c| c.kind).collect()
    }

    #[test]
    fn a_bump_is_breaking_past_the_part_semver_keeps_stable() {
        let bump = |from: &str, to: &str| Bump::between(&version(from), &version(to));
        assert_eq!(bump("1.2.3", "2.0.0"), Bump::Breaking);
        assert_eq!(bump("0.29.0", "0.30.0"), Bump::Breaking);
        assert_eq!(bump("0.0.3", "0.0.4"), Bump::Breaking);
        assert_eq!(bump("1.40.0", "1.41.0"), Bump::Minor);
        assert_eq!(bump("0.1.0", "0.1.1"), Bump::Patch);
        assert_eq!(bump("1.0.0-rc.1", "1.0.0"), Bump::Other);
        assert_eq!(bump("2024a", "2024b"), Bump::Other);
        assert!(version("1.9.0").is_older_than(&version("1.10.0")) && !version("x").is_older_than(&version("y")));
    }

    #[test]
    fn changes_read_warnings_then_biggest_updates_then_added_then_removed_alphabetical_inside() {
        let row = |(name, version, checksum)| package(name, version, checksum);
        let base = [
            ("zeta", "1.0.0", "a"),
            ("minor", "1.0.0", "a"),
            ("major-b", "1.0.0", "a"),
            ("major-a", "1.0.0", "a"),
            ("gone", "1.0.0", "a"),
            ("odd", "x", "a"),
        ];
        let head = [
            ("zeta", "1.0.0", "b"),
            ("minor", "1.1.0", "a"),
            ("major-b", "2.0.0", "a"),
            ("major-a", "2.0.0", "a"),
            ("new", "1.0.0", "a"),
            ("odd", "y", "a"),
        ];
        let names: Vec<String> = compare(&base.map(row), &head.map(row)).changes.iter().map(|c| c.name.to_string()).collect();
        assert_eq!(names, ["zeta", "major-a", "major-b", "minor", "odd", "new", "gone"]);
    }

    #[test]
    fn the_same_version_with_another_checksum_or_source_warns() {
        let warning = |drift| vec![ChangeKind::Warning { version: version("1.0.0"), drift }];
        assert_eq!(kinds(&[package("a", "1.0.0", "x")], &[package("a", "1.0.0", "y")]), warning(Drift::Checksum));
        assert_eq!(kinds(&[sourced("1.0.0", "x")], &[sourced("1.0.0", "y")]), warning(Drift::Source));
        let bare = Package { origin: Origin { checksum: None, source: None }, ..package("a", "1.0.0", "x") };
        assert_eq!(kinds(&[package("a", "1.0.0", "x")], &[bare]), warning(Drift::ChecksumAndSource));
    }

    #[test]
    fn a_warning_stays_when_the_same_package_also_updates_another_version() {
        let base = [package("a", "1.0.0", "x"), package("a", "2.0.0", "x")];
        let head = [package("a", "1.0.0", "y"), package("a", "2.1.0", "x")];
        let warning = ChangeKind::Warning { version: version("1.0.0"), drift: Drift::Checksum };
        assert_eq!(kinds(&base, &head), vec![warning, updated("2.0.0", "2.1.0", Bump::Minor)]);
    }

    #[test]
    fn a_second_version_of_a_name_is_an_addition_not_an_update() {
        let base = [package("rand", "0.8.5", "x")];
        let head = [package("rand", "0.8.5", "x"), package("rand", "0.9.0", "x")];
        assert_eq!(kinds(&base, &head), vec![ChangeKind::Added { version: version("0.9.0") }]);
    }

    #[test]
    fn of_several_versions_the_update_pairs_with_the_compatible_one() {
        let base = [package("a", "1.0.0", "x"), package("a", "2.0.0", "x")];
        let removed = ChangeKind::Removed { version: version("1.0.0") };
        assert_eq!(kinds(&base, &[package("a", "2.1.0", "x")]), vec![updated("2.0.0", "2.1.0", Bump::Minor), removed]);
    }

    #[test]
    fn a_missing_side_reads_as_no_packages_and_a_broken_side_fails() {
        let text = include_str!("fixtures/cargo_head.lock");
        let added = summarize("Cargo.lock", None, Some(text)).unwrap();
        assert!(!added.changes.is_empty() && added.changes.iter().all(|c| matches!(c.kind, ChangeKind::Added { .. })));
        assert!(summarize("sub/Cargo.lock", Some("not = [toml"), Some(text)).is_err());
    }
}
