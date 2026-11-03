//! What a lockfile change means package by package: pure, the two texts in and a [`Summary`] out.
//!
//! A row is plain only when its entry is proven ordinary; anything else warns, or the parser refuses the file and the raw
//! diff shows. This is a reading aid: a clean summary does not prove that a lockfile is safe.
mod cargo;
mod npm;

use anyhow::{Context as _, Result};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// One entry per format: adding a lockfile is a parser and a row here.
const FORMATS: [Format; 2] = [
    Format { file_name: "Cargo.lock", parse: cargo::parse, public_registry: cargo::PUBLIC_REGISTRY },
    Format { file_name: "package-lock.json", parse: npm::parse, public_registry: npm::PUBLIC_REGISTRY },
];

struct Format {
    file_name: &'static str,
    parse: fn(&str) -> Result<Vec<Package>>,
    /// The one registry ordinary before the old side has shown any other.
    public_registry: &'static str,
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
        release_numbers(self.0.split(['-', '+']).next()?)
    }

    /// Whether both are semver and this one comes before `other`; a pre-release comes before its release.
    pub(crate) fn is_older_than(&self, other: &Self) -> bool {
        let (Some(mine), Some(theirs)) = (self.numbers(), other.numbers()) else { return false };
        mine.cmp(&theirs).then_with(|| pre_release_order(self.pre_release(), other.pre_release())) == Ordering::Less
    }

    fn pre_release(&self) -> Option<&str> {
        self.0.split('+').next()?.split_once('-').map(|(_, identifiers)| identifiers)
    }

    /// The version without its build metadata: two with the same one read as the same release.
    fn release(&self) -> &str {
        self.0.split('+').next().unwrap_or_default()
    }
}

/// Major, minor and patch of a text that is nothing else.
fn release_numbers(release: &str) -> Option<[u64; 3]> {
    let number = |part: &str| is_number(part).then(|| part.parse().ok()).flatten();
    let parts: Vec<u64> = release.split('.').map(number).collect::<Option<_>>()?;
    parts.try_into().ok()
}

fn is_numeric(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

fn is_number(text: &str) -> bool {
    is_numeric(text) && (text.len() == 1 || !text.starts_with('0'))
}

fn is_identifier(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Whether `text` is semver to the letter: no prefix, no leading zero, nothing but ASCII.
fn is_semver(text: &str) -> bool {
    let (rest, build) = text.split_once('+').map_or((text, None), |(rest, build)| (rest, Some(build)));
    let (release, pre) = rest.split_once('-').map_or((rest, None), |(release, pre)| (release, Some(pre)));
    let pre_part = |part: &str| is_identifier(part) && (!is_numeric(part) || is_number(part));
    release_numbers(release).is_some()
        && pre.is_none_or(|pre| pre.split('.').all(pre_part))
        && build.is_none_or(|build| build.split('.').all(is_identifier))
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

/// Where a copy's code comes from, as far as its format can prove it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Site {
    /// The project's own code: no source and no checksum.
    Project,
    /// Shipped inside another package, so it has no origin of its own.
    Bundled,
    /// An entry of the form its registry must write; the base is all that stands before the package's own part.
    Registry {
        base: String,
    },
    /// A git source up to its commit.
    Git {
        repo: String,
    },
    Link {
        target: String,
    },
    /// Nothing proves what this is, so it never reads as ordinary.
    Unproven {
        shown: String,
    },
}

impl Site {
    /// `git+<url>#<commit>`, the form both formats write for git.
    fn git(source: &str) -> Option<Self> {
        let (repo, commit) = source.split_once('#')?;
        (repo.starts_with("git+") && !commit.is_empty()).then(|| Self::Git { repo: repo.to_owned() })
    }

    /// What a registry or a repository is known by.
    fn known_key(&self) -> Option<&str> {
        match self {
            Self::Registry { base } => Some(base),
            Self::Git { repo } => Some(repo),
            Self::Project | Self::Bundled | Self::Link { .. } | Self::Unproven { .. } => None,
        }
    }

    /// Ordinary means the project's own, or a base the format makes public or the old side already used.
    fn is_ordinary(&self, known: &Known<'_>) -> bool {
        matches!(self, Self::Project | Self::Bundled) || self.known_key().is_some_and(|key| known.contains(key))
    }

    fn describe(&self) -> String {
        match self {
            Self::Project => "the project".to_owned(),
            Self::Bundled => "a bundle".to_owned(),
            Self::Registry { base } => base.clone(),
            Self::Git { repo } => repo.clone(),
            Self::Link { target } => format!("a link to {target}"),
            Self::Unproven { shown } => shown.clone(),
        }
    }
}

/// The registries and git repositories a new entry may come from and still read as ordinary.
type Known<'a> = BTreeSet<&'a str>;

fn known_sites<'a>(base: &'a [Package], public_registry: &'a str) -> Known<'a> {
    base.iter().filter_map(|package| package.site.known_key()).chain([public_registry]).collect()
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
    site: Site,
    /// Runs code when it installs.
    install_script: bool,
    /// What tells this copy from every other, across both sides: the install path where a format has one, so a copy that
    /// keeps its place but changes is a change whatever the rest of the tree holds.
    place: String,
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
    /// The new version comes from a site the old one did not.
    SourceMoved {
        to: String,
    },
    /// A registry copy that pins no checksum.
    ChecksumMissing,
    /// The source is not proven ordinary.
    UnknownSource {
        from: String,
    },
    /// The version is not strict semver, so nothing can be read from it.
    NotSemver,
}

impl Alert {
    /// The alerts between the copies of one release, whatever its version text says.
    fn of_same_release(base: &Copies<'_>, head: &Copies<'_>) -> Vec<Self> {
        let script = Self::of_install_script(base, head);
        if base.origins() == head.origins() {
            return script.into_iter().collect();
        }
        let (checksums, sources) = (base.checksums() != head.checksums(), base.sources() != head.sources());
        let checksum = (checksums || !sources).then_some(Self::ChecksumChanged);
        let source = (sources || !checksums).then_some(Self::SourceChanged);
        [checksum, source, script].into_iter().flatten().collect()
    }

    fn of_install_script(base: &Copies<'_>, head: &Copies<'_>) -> Option<Self> {
        (head.install_script() && !base.install_script()).then_some(Self::InstallScript)
    }

    /// A link has no version of its own to read.
    fn of_version(version: &Version, head: &Copies<'_>) -> Option<Self> {
        let versioned = head.0.iter().any(|copy| !matches!(copy.site, Site::Link { .. }));
        (versioned && !is_semver(&version.0)).then_some(Self::NotSemver)
    }

    fn of_checksum(head: &Copies<'_>) -> Option<Self> {
        let unpinned =
            |copy: &&Package| matches!(copy.site, Site::Registry { .. }) && copy.origin.checksum.as_deref().is_none_or(str::is_empty);
        head.0.iter().any(unpinned).then_some(Self::ChecksumMissing)
    }

    fn of_source(head: &Copies<'_>, known: &Known<'_>) -> Option<Self> {
        let unknown: Vec<String> = head.sites().iter().filter(|site| !site.is_ordinary(known)).map(|site| site.describe()).collect();
        (!unknown.is_empty()).then(|| Self::UnknownSource { from: unknown.join(", ") })
    }

    /// Copies with no site of their own, all bundled, come from nowhere a new version can have moved from.
    fn of_move(base: &Copies<'_>, head: &Copies<'_>) -> Option<Self> {
        let (old, new) = (base.sites(), head.sites());
        let moved = !old.is_empty() && (new.is_empty() || !new.is_subset(&old));
        moved.then(|| Self::SourceMoved { to: head.describe_sites() })
    }

    /// The alerts of a move from `from` to `to`.
    fn of_update(from: &Version, to: &Version, base: &Copies<'_>, head: &Copies<'_>, known: &Known<'_>) -> Vec<Self> {
        let proof = [Self::of_version(to, head), Self::of_checksum(head)].into_iter().flatten();
        if from.release() == to.release() {
            return Self::of_same_release(base, head).into_iter().chain(Self::of_source(head, known)).chain(proof).collect();
        }
        let source = Self::of_move(base, head).or_else(|| Self::of_source(head, known));
        source.into_iter().chain(proof).chain(Self::of_install_script(base, head)).collect()
    }

    /// A new copy has nothing to compare with: all it can be is proven.
    fn of_added(version: &Version, head: &Copies<'_>, known: &Known<'_>) -> Vec<Self> {
        let script = head.install_script().then_some(Self::InstallScript);
        [Self::of_source(head, known), Self::of_version(version, head), Self::of_checksum(head), script].into_iter().flatten().collect()
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

    /// Two changes the same row can say: the same version, or the same move.
    fn is_same_row(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::SameVersion { version, .. }, Self::SameVersion { version: other, .. }) => version == other,
            (Self::Updated { from, to, .. }, Self::Updated { from: other_from, to: other_to, .. }) => from == other_from && to == other_to,
            _ => false,
        }
    }

    fn add_alerts(&mut self, more: &[Alert]) {
        if let Self::SameVersion { alerts, .. } | Self::Updated { alerts, .. } | Self::Added { alerts, .. } = self {
            let new: Vec<Alert> = more.iter().filter(|alert| !alerts.contains(alert)).cloned().collect();
            alerts.extend(new);
        }
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
    Ok(Summary { changes: compare(&read(base)?, &read(head)?, format.public_registry) })
}

/// The copies of one version of a package; a bundled copy has no origin of its own, so it adds none.
#[derive(Default)]
struct Copies<'a>(Vec<&'a Package>);

impl Copies<'_> {
    fn with_origin(&self) -> impl Iterator<Item = &Package> {
        self.0.iter().copied().filter(|copy| copy.site != Site::Bundled)
    }

    fn origins(&self) -> BTreeSet<&Origin> {
        self.with_origin().map(|copy| &copy.origin).collect()
    }

    fn checksums(&self) -> BTreeSet<Option<&str>> {
        self.with_origin().map(|copy| copy.origin.checksum.as_deref()).collect()
    }

    fn sources(&self) -> BTreeSet<Option<&str>> {
        self.with_origin().map(|copy| copy.origin.source.as_deref()).collect()
    }

    fn sites(&self) -> BTreeSet<&Site> {
        self.with_origin().map(|copy| &copy.site).collect()
    }

    fn install_script(&self) -> bool {
        self.0.iter().any(|copy| copy.install_script)
    }

    fn describe_sites(&self) -> String {
        let sites: Vec<String> = self.sites().iter().map(|site| site.describe()).collect();
        if sites.is_empty() { Site::Bundled.describe() } else { sites.join(", ") }
    }
}

/// Every copy of one package name on one side.
#[derive(Default)]
struct Installed<'a> {
    versions: BTreeMap<&'a Version, Copies<'a>>,
    places: BTreeMap<&'a str, &'a Package>,
}

/// Copies of a version collapse into one: moving a copy in the tree changes nothing a reader cares about.
fn by_name(packages: &[Package]) -> BTreeMap<&PackageName, Installed<'_>> {
    let mut names: BTreeMap<&PackageName, Installed<'_>> = BTreeMap::new();
    for package in packages {
        let installed = names.entry(&package.name).or_default();
        installed.versions.entry(&package.version).or_default().0.push(package);
        installed.places.insert(&package.place, package);
    }
    names
}

fn compare(base: &[Package], head: &[Package], public_registry: &str) -> Vec<Change> {
    let known = known_sites(base, public_registry);
    let (base, head) = (by_name(base), by_name(head));
    let none = Installed::default();
    let names: BTreeSet<_> = base.keys().chain(head.keys()).copied().collect();
    let mut changes: Vec<Change> = names
        .into_iter()
        .flat_map(|name| {
            let kinds = package_changes(base.get(name).unwrap_or(&none), head.get(name).unwrap_or(&none), &known);
            kinds.into_iter().map(|kind| Change { name: name.clone(), kind })
        })
        .collect();
    changes.sort_by_key(|c| c.kind.rank());
    changes
}

fn package_changes(base: &Installed<'_>, head: &Installed<'_>, known: &Known<'_>) -> Vec<ChangeKind> {
    let same_versions = base.versions.iter().filter_map(|(&version, copies)| same_version(version, copies, head.versions.get(version)?));
    let changes = same_versions.chain(moved_versions(base, head, known)).chain(place_changes(base, head, known));
    in_one_row_each(changes)
}

fn same_version(version: &Version, old: &Copies<'_>, new: &Copies<'_>) -> Option<ChangeKind> {
    let alerts = Alert::of_same_release(old, new);
    (!alerts.is_empty()).then(|| ChangeKind::SameVersion { version: version.clone(), alerts })
}

/// Each row once, with every alert any comparison found for it.
fn in_one_row_each(changes: impl Iterator<Item = ChangeKind>) -> Vec<ChangeKind> {
    let mut rows: Vec<ChangeKind> = vec![];
    for change in changes {
        match rows.iter_mut().find(|row| row.is_same_row(&change)) {
            Some(row) => row.add_alerts(change.alerts()),
            None => rows.push(change),
        }
    }
    rows
}

fn updated(from: &Version, to: &Version, old: &Copies<'_>, new: &Copies<'_>, known: &Known<'_>) -> ChangeKind {
    ChangeKind::Updated {
        from: from.clone(),
        to: to.clone(),
        bump: Bump::between(from, to),
        alerts: Alert::of_update(from, to, old, new, known),
    }
}

/// A copy that changes in place, which the sets of versions and origins hide when copies trade them.
fn place_changes(base: &Installed<'_>, head: &Installed<'_>, known: &Known<'_>) -> Vec<ChangeKind> {
    let changed_at = |(place, new): (&&str, &&Package)| {
        let old = base.places.get(place)?;
        let (old_copy, new_copy) = (Copies(vec![*old]), Copies(vec![*new]));
        if old.version != new.version {
            return Some(updated(&old.version, &new.version, &old_copy, &new_copy, known));
        }
        same_version(&new.version, &old_copy, &new_copy)
    };
    head.places.iter().filter_map(changed_at).collect()
}

fn only_in<'a>(versions: &Installed<'a>, other: &Installed<'a>) -> Vec<&'a Version> {
    let mut only: Vec<&Version> = versions.versions.keys().filter(|v| !other.versions.contains_key(*v)).copied().collect();
    only.sort_by_key(|v| (v.numbers(), v.0.as_str()));
    only
}

/// The moves `fits` allows, each new version taken once, and the versions left on either side.
struct Paired<'a> {
    moves: Vec<(&'a Version, &'a Version)>,
    gone: Vec<&'a Version>,
    new: Vec<&'a Version>,
}

fn paired<'a>(gone: Vec<&'a Version>, new: Vec<&'a Version>, fits: fn(&Version, &Version) -> bool) -> Paired<'a> {
    let (mut moves, mut left, mut new) = (vec![], vec![], new);
    for from in gone {
        match new.iter().position(|to| fits(from, to)) {
            Some(at) => moves.push((from, new.remove(at))),
            None => left.push(from),
        }
    }
    Paired { moves, gone: left, new }
}

/// A gone version pairs with a new one it is compatible with; what is left pairs only when the counts match,
/// since an extra version is a package added or removed, not an update.
fn moved_versions(base: &Installed<'_>, head: &Installed<'_>, known: &Known<'_>) -> Vec<ChangeKind> {
    let none = Copies::default();
    let update = |(from, to): (&Version, &Version)| {
        updated(from, to, base.versions.get(from).unwrap_or(&none), head.versions.get(to).unwrap_or(&none), known)
    };
    let same = paired(only_in(base, head), only_in(head, base), |from, to| from.release() == to.release());
    let compatible = paired(same.gone, same.new, |from, to| Bump::between(from, to) != Bump::Breaking);
    let moves = same.moves.into_iter().chain(compatible.moves);
    if compatible.gone.len() == compatible.new.len() {
        return moves.chain(compatible.gone.into_iter().zip(compatible.new)).map(update).collect();
    }
    let added = |version: &Version| ChangeKind::Added {
        version: version.clone(),
        alerts: Alert::of_added(version, head.versions.get(version).unwrap_or(&none), known),
    };
    let removed = |version: &Version| ChangeKind::Removed { version: version.clone() };
    moves.map(update).chain(compatible.new.into_iter().map(added)).chain(compatible.gone.into_iter().map(removed)).collect()
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

    const REGISTRY: &str = "registry+https://index.example";

    fn package(name: &str, version: &str, checksum: &str) -> Package {
        Package {
            name: PackageName(name.into()),
            version: Version(version.into()),
            origin: Origin { checksum: Some(checksum.into()), source: Some(REGISTRY.into()) },
            site: Site::Registry { base: REGISTRY.into() },
            install_script: false,
            place: format!("{name} {version}"),
        }
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
        compare(base, head, REGISTRY).into_iter().map(|c| c.kind).collect()
    }

    #[test]
    fn a_pre_release_is_older_than_its_release() {
        assert!(version("2.0.0-rc.1").is_older_than(&version("2.0.0")));
        assert!(!version("2.0.0").is_older_than(&version("2.0.0-rc.1")));
    }

    #[test]
    fn only_semver_to_the_letter_is_semver() {
        for good in ["1.0.0", "0.0.0", "1.2.3-rc.1", "1.2.3-0", "1.2.3-a-b", "1.2.3+x.01", "1.2.3-rc+x"] {
            assert!(is_semver(good), "{good}");
        }
        for bad in ["v1.0.0", "=1.0.0", "01.0.0", "1.0", "1.0.0.0", "+1.0.0", "1.0.0-", "1.0.0-01", "1.0.0+", "1.0.0+a+b", "1.0.0 "] {
            assert!(!is_semver(bad), "{bad}");
        }
        for hidden in ["\u{ff11}.0.0", "1.0.0\u{200b}", "1.0.0\u{2060}", "1.0.0-\u{e9}", "99999999999999999999.0.0"] {
            assert!(!is_semver(hidden), "{hidden}");
        }
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
        let names: Vec<String> = compare(&base.map(row), &head.map(row), REGISTRY).iter().map(|c| c.name.to_string()).collect();
        assert_eq!(names, ["odd", "zeta", "major-a", "major-b", "minor", "new", "gone"]);
    }

    #[test]
    fn an_update_that_warns_reads_before_a_breaking_one() {
        let base = [package("a", "1.0.0", "x"), package("z", "1.0.0", "x")];
        let z = package("z", "1.0.1", "x");
        let moved = Package { site: Site::Git { repo: "git+https://evil.example/z".into() }, ..z };
        let head = [package("a", "2.0.0", "x"), moved];
        let names: Vec<String> = compare(&base, &head, REGISTRY).iter().map(|c| c.name.to_string()).collect();
        assert_eq!(names, ["z", "a"]);
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
    fn copies_trading_checksums_and_sources_warn_though_both_sets_stay_the_same() {
        let copy = |checksum: &str, source: &str| Package {
            origin: Origin { checksum: Some(checksum.into()), source: Some(source.into()) },
            ..package("a", "1.0.0", "x")
        };
        let warning =
            vec![ChangeKind::SameVersion { version: version("1.0.0"), alerts: vec![Alert::ChecksumChanged, Alert::SourceChanged] }];
        assert_eq!(kinds(&[copy("x", "one"), copy("y", "two")], &[copy("x", "two"), copy("y", "one")]), warning);
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
