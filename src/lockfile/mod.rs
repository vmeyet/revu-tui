//! What a lockfile change means package by package: pure, the two texts in and a [`Summary`] out.
mod cargo;
mod npm;

use anyhow::{Context as _, Result};
use std::cmp::Ordering;
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

    /// Whether both are semver and this one comes before `other`; a pre-release comes before its release.
    pub(crate) fn is_older_than(&self, other: &Self) -> bool {
        let (Some(mine), Some(theirs)) = (self.numbers(), other.numbers()) else { return false };
        mine.cmp(&theirs).then_with(|| pre_release_order(self.pre_release(), other.pre_release())) == Ordering::Less
    }

    fn pre_release(&self) -> Option<&str> {
        self.0.split('+').next()?.split_once('-').map(|(_, identifiers)| identifiers)
    }

    /// The version without its build metadata or anything a reader cannot see: two with the same one read as the same.
    fn release(&self) -> String {
        self.0.split('+').next().unwrap_or_default().chars().filter(char::is_ascii_graphic).collect()
    }
}

/// Semver's order: none is the release and comes last; numbers come before words, and more identifiers after fewer.
fn pre_release_order(mine: Option<&str>, theirs: Option<&str>) -> Ordering {
    match (mine, theirs) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(mine), Some(theirs)) => mine.split('.').map(identifier_order).cmp(theirs.split('.').map(identifier_order)),
    }
}

fn identifier_order(identifier: &str) -> (bool, u64, &str) {
    let number = identifier.parse::<u64>();
    (number.is_err(), number.unwrap_or(0), identifier)
}

/// What kind of place a source is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum SourceKind {
    Registry,
    Git,
    Link,
    Other,
}

/// Where a source points: its kind and host. A move inside a site is routine, a move between sites is not.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Site {
    kind: SourceKind,
    host: String,
}

impl Site {
    /// Cargo writes `registry+`, `sparse+` and `git+`; npm a tarball URL, whose path holds `/-/` on a registry.
    fn of(source: &str) -> Self {
        let (kind, place) = if let Some(target) = source.strip_prefix(LINK) {
            (SourceKind::Link, target)
        } else if let Some(url) = source.strip_prefix("registry+").or_else(|| source.strip_prefix("sparse+")) {
            (SourceKind::Registry, url)
        } else if let Some(url) = source.strip_prefix("git+") {
            (SourceKind::Git, url)
        } else if source.starts_with("http") && source.contains("/-/") {
            (SourceKind::Registry, source)
        } else {
            (SourceKind::Other, source)
        };
        let host = url::Url::parse(place).ok().and_then(|url| url.host_str().map(str::to_owned));
        Self { kind, host: host.unwrap_or_else(|| place.to_owned()) }
    }

    fn describe(&self) -> String {
        match self.kind {
            SourceKind::Git => format!("git on {}", self.host),
            SourceKind::Link => format!("a link to {}", self.host),
            SourceKind::Registry | SourceKind::Other => self.host.clone(),
        }
    }
}

/// How a parser spells a source that is a link to a folder.
const LINK: &str = "link:";

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
    /// Runs code when it installs.
    install_script: bool,
    /// The copy a project's own `require` finds, at the top of the tree.
    at_top: bool,
    /// Shipped inside another package, so it has no origin of its own.
    bundled: bool,
}

impl Package {
    fn new(name: String, version: String, origin: Origin) -> Self {
        Self { name: PackageName(name), version: Version(version), origin, install_script: false, at_top: false, bundled: false }
    }
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

/// Why a row warns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Alert {
    ChecksumChanged,
    SourceChanged,
    InstallScript,
    /// The new version comes from another kind of place or host than the old one.
    SourceMoved {
        to: String,
    },
    ChecksumLost,
    /// A new package from somewhere that is no registry the lockfile already used.
    OffRegistry {
        from: String,
    },
}

impl Alert {
    /// The alerts between the copies of one release, whatever its version text says.
    fn of_same_release(base: &Copies<'_>, head: &Copies<'_>) -> Vec<Self> {
        let (checksums, sources) = (base.checksums() != head.checksums(), base.sources() != head.sources());
        let moved = base.origins != head.origins;
        let mut alerts = vec![];
        if checksums || (moved && !sources) {
            alerts.push(Self::ChecksumChanged);
        }
        if sources || (moved && !checksums) {
            alerts.push(Self::SourceChanged);
        }
        alerts.extend(Self::of_install_script(base, head));
        alerts
    }

    fn of_install_script(base: &Copies<'_>, head: &Copies<'_>) -> Option<Self> {
        (head.install_script && !base.install_script).then_some(Self::InstallScript)
    }

    /// The alerts of a move from `from` to `to`.
    fn of_update(from: &Version, to: &Version, base: &Copies<'_>, head: &Copies<'_>) -> Vec<Self> {
        if from.release() == to.release() {
            return Self::of_same_release(base, head);
        }
        let moved = (base.sites() != head.sites()).then(|| Self::SourceMoved { to: head.describe_sites() });
        let lost =
            (base.checksums().iter().any(Option::is_some) && !head.checksums().iter().all(Option::is_some)).then_some(Self::ChecksumLost);
        moved.into_iter().chain(lost).chain(Self::of_install_script(base, head)).collect()
    }

    /// A new copy warns off a registry, or on a host the old side never used; a source of no kind is the project's own.
    fn of_added(head: &Copies<'_>, known_hosts: &BTreeSet<String>) -> Vec<Self> {
        let unknown = |site: &Site| !known_hosts.is_empty() && !known_hosts.contains(&site.host);
        let odd = head.sites().iter().any(|site| site.kind != SourceKind::Registry || unknown(site));
        odd.then(|| Self::OffRegistry { from: head.describe_sites() }).into_iter().collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ChangeKind {
    /// The same version, other code: the row a reader must not miss.
    SameVersion {
        version: Version,
        alerts: Vec<Alert>,
    },
    Updated {
        from: Version,
        to: Version,
        bump: Bump,
        alerts: Vec<Alert>,
    },
    Added {
        version: Version,
        alerts: Vec<Alert>,
    },
    Removed {
        version: Version,
    },
}

impl ChangeKind {
    /// Why the row warns; a row with no alert is routine.
    pub(crate) fn alerts(&self) -> &[Alert] {
        match self {
            Self::SameVersion { alerts, .. } | Self::Updated { alerts, .. } | Self::Added { alerts, .. } => alerts,
            Self::Removed { .. } => &[],
        }
    }

    pub(crate) fn warns(&self) -> bool {
        !self.alerts().is_empty()
    }

    fn rank(&self) -> u8 {
        match self {
            Self::SameVersion { .. } => 0,
            _ if self.warns() => 0,
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
pub(crate) fn summarize(path: &str, base: Option<&str>, head: Option<&str>) -> Result<Summary> {
    let format = format_of(path).with_context(|| format!("{path} is not a lockfile"))?;
    let read = |text: Option<&str>| text.map_or_else(|| Ok(vec![]), format.parse);
    Ok(compare(&read(base)?, &read(head)?))
}

/// The copies of one version of a package; a bundled copy has no origin of its own, so it adds none.
#[derive(Default)]
struct Copies<'a> {
    origins: BTreeSet<&'a Origin>,
    install_script: bool,
}

impl Copies<'_> {
    fn checksums(&self) -> BTreeSet<Option<&str>> {
        self.origins.iter().map(|o| o.checksum.as_deref()).collect()
    }

    fn sources(&self) -> BTreeSet<Option<&str>> {
        self.origins.iter().map(|o| o.source.as_deref()).collect()
    }

    fn sites(&self) -> BTreeSet<Site> {
        self.origins.iter().filter_map(|o| o.source.as_deref()).map(Site::of).collect()
    }

    fn describe_sites(&self) -> String {
        let sites: Vec<String> = self.sites().iter().map(Site::describe).collect();
        if sites.is_empty() { "no source".to_owned() } else { sites.join(", ") }
    }
}

/// Every copy of one package name on one side.
#[derive(Default)]
struct Installed<'a> {
    versions: BTreeMap<&'a Version, Copies<'a>>,
    at_top: Option<&'a Version>,
}

/// Copies of a version collapse into one: moving a copy in the tree changes nothing a reader cares about.
fn by_name(packages: &[Package]) -> BTreeMap<&PackageName, Installed<'_>> {
    let mut names: BTreeMap<&PackageName, Installed<'_>> = BTreeMap::new();
    for package in packages {
        let installed = names.entry(&package.name).or_default();
        let copies = installed.versions.entry(&package.version).or_default();
        copies.install_script |= package.install_script;
        if !package.bundled {
            copies.origins.insert(&package.origin);
        }
        if package.at_top {
            installed.at_top = Some(&package.version);
        }
    }
    names
}

fn compare(base: &[Package], head: &[Package]) -> Summary {
    let known_hosts: BTreeSet<String> = base.iter().filter_map(|p| p.origin.source.as_deref()).map(|s| Site::of(s).host).collect();
    let (base, head) = (by_name(base), by_name(head));
    let none = Installed::default();
    let names: BTreeSet<_> = base.keys().chain(head.keys()).copied().collect();
    let mut changes: Vec<Change> = names
        .into_iter()
        .flat_map(|name| {
            let kinds = package_changes(base.get(name).unwrap_or(&none), head.get(name).unwrap_or(&none), &known_hosts);
            kinds.into_iter().map(|kind| Change { name: name.clone(), kind })
        })
        .collect();
    changes.sort_by_key(|c| c.kind.rank());
    Summary { changes }
}

fn package_changes(base: &Installed<'_>, head: &Installed<'_>, known_hosts: &BTreeSet<String>) -> Vec<ChangeKind> {
    let same_versions = base.versions.iter().filter_map(|(&version, copies)| {
        let alerts = Alert::of_same_release(copies, head.versions.get(version)?);
        (!alerts.is_empty()).then(|| ChangeKind::SameVersion { version: version.clone(), alerts })
    });
    let mut changes: Vec<ChangeKind> = same_versions.chain(moved_versions(base, head, known_hosts)).collect();
    changes.extend(top_copy_swap(base, head, &changes));
    changes
}

fn updated(from: &Version, to: &Version, base: &Installed<'_>, head: &Installed<'_>) -> ChangeKind {
    let none = Copies::default();
    let (old, new) = (base.versions.get(from).unwrap_or(&none), head.versions.get(to).unwrap_or(&none));
    ChangeKind::Updated { from: from.clone(), to: to.clone(), bump: Bump::between(from, to), alerts: Alert::of_update(from, to, old, new) }
}

/// The copy a project loads changed version though both versions stay somewhere in the tree.
fn top_copy_swap(base: &Installed<'_>, head: &Installed<'_>, changes: &[ChangeKind]) -> Option<ChangeKind> {
    let (from, to) = base.at_top.zip(head.at_top).filter(|(from, to)| from != to)?;
    let shown = changes.iter().any(|change| matches!(change, ChangeKind::Updated { from: f, to: t, .. } if f == from && t == to));
    (!shown).then(|| updated(from, to, base, head))
}

fn only_in<'a>(versions: &Installed<'a>, other: &Installed<'a>) -> Vec<&'a Version> {
    let mut only: Vec<&Version> = versions.versions.keys().filter(|v| !other.versions.contains_key(*v)).copied().collect();
    only.sort_by_key(|v| (v.numbers(), v.0.as_str()));
    only
}

/// A gone version pairs with a new one it is compatible with; what is left pairs only when the counts match,
/// since an extra version is a package added or removed, not an update.
fn moved_versions(base: &Installed<'_>, head: &Installed<'_>, known_hosts: &BTreeSet<String>) -> Vec<ChangeKind> {
    let mut new = only_in(head, base);
    let mut changes = vec![];
    let mut left = vec![];
    for from in only_in(base, head) {
        match new.iter().position(|to| Bump::between(from, to) != Bump::Breaking) {
            Some(at) => changes.push(updated(from, new.remove(at), base, head)),
            None => left.push(from),
        }
    }
    if left.len() == new.len() {
        changes.extend(left.into_iter().zip(new).map(|(from, to)| updated(from, to, base, head)));
        return changes;
    }
    let added = |version: &Version| {
        let alerts = head.versions.get(version).map_or_else(Vec::new, |copies| Alert::of_added(copies, known_hosts));
        ChangeKind::Added { version: version.clone(), alerts }
    };
    changes.extend(new.into_iter().map(added));
    changes.extend(left.into_iter().map(|v| ChangeKind::Removed { version: v.clone() }));
    changes
}

/// The names of the rows that warn, for a format's tests.
#[cfg(test)]
fn warned_names(path: &str, base: &str, head: &str) -> Vec<String> {
    let summary = summarize(path, Some(base), Some(head)).unwrap();
    summary.changes.iter().filter(|c| c.kind.warns()).map(|c| c.name.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(name: &str, version: &str, checksum: &str) -> Package {
        let origin = Origin { checksum: Some(checksum.into()), source: Some("registry+https://index.example".into()) };
        Package::new(name.into(), version.into(), origin)
    }

    fn sourced(version: &str, source: &str) -> Package {
        let same = package("a", version, "same");
        Package { origin: Origin { source: Some(source.into()), ..same.origin }, ..same }
    }

    fn version(text: &str) -> Version {
        Version(text.into())
    }

    fn updated(from: &str, to: &str, bump: Bump) -> ChangeKind {
        ChangeKind::Updated { from: version(from), to: version(to), bump, alerts: vec![] }
    }

    fn kinds(base: &[Package], head: &[Package]) -> Vec<ChangeKind> {
        compare(base, head).changes.into_iter().map(|c| c.kind).collect()
    }

    #[test]
    fn a_pre_release_is_older_than_its_release() {
        assert!(version("2.0.0-rc.1").is_older_than(&version("2.0.0")));
        assert!(!version("2.0.0").is_older_than(&version("2.0.0-rc.1")));
    }

    #[test]
    fn pre_releases_order_by_semver_identifiers() {
        assert!(version("1.0.0-rc.2").is_older_than(&version("1.0.0-rc.10")));
        assert!(version("1.0.0-1").is_older_than(&version("1.0.0-alpha")));
        assert!(version("1.0.0-alpha").is_older_than(&version("1.0.0-alpha.1")));
    }

    #[test]
    fn build_metadata_does_not_order_versions() {
        assert!(!version("1.0.0").is_older_than(&version("1.0.0+x")) && !version("1.0.0+x").is_older_than(&version("1.0.0")));
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
        let warning = |alerts| vec![ChangeKind::SameVersion { version: version("1.0.0"), alerts }];
        assert_eq!(kinds(&[package("a", "1.0.0", "x")], &[package("a", "1.0.0", "y")]), warning(vec![Alert::ChecksumChanged]));
        assert_eq!(kinds(&[sourced("1.0.0", "x")], &[sourced("1.0.0", "y")]), warning(vec![Alert::SourceChanged]));
        let bare = Package { origin: Origin { checksum: None, source: None }, ..package("a", "1.0.0", "x") };
        assert_eq!(kinds(&[package("a", "1.0.0", "x")], &[bare]), warning(vec![Alert::ChecksumChanged, Alert::SourceChanged]));
    }

    #[test]
    fn a_warning_stays_when_the_same_package_also_updates_another_version() {
        let base = [package("a", "1.0.0", "x"), package("a", "2.0.0", "x")];
        let head = [package("a", "1.0.0", "y"), package("a", "2.1.0", "x")];
        let warning = ChangeKind::SameVersion { version: version("1.0.0"), alerts: vec![Alert::ChecksumChanged] };
        assert_eq!(kinds(&base, &head), vec![warning, updated("2.0.0", "2.1.0", Bump::Minor)]);
    }

    #[test]
    fn a_second_version_of_a_name_is_an_addition_not_an_update() {
        let base = [package("rand", "0.8.5", "x")];
        let head = [package("rand", "0.8.5", "x"), package("rand", "0.9.0", "x")];
        assert_eq!(kinds(&base, &head), vec![ChangeKind::Added { version: version("0.9.0"), alerts: vec![] }]);
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
