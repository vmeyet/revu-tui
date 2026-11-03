//! `package-lock.json`, the `packages` map of lockfile versions 2 and 3.
use super::{Origin, Package, PackageName, Site, Version};
use anyhow::{Context as _, Result, ensure};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

pub(super) const PUBLIC_REGISTRY: &str = "https://registry.npmjs.org";

#[derive(Deserialize)]
struct Lock {
    #[serde(rename = "lockfileVersion")]
    version: u32,
    packages: Option<BTreeMap<String, serde_json::Value>>,
    #[serde(default)]
    dependencies: BTreeMap<String, Legacy>,
}

#[derive(Deserialize)]
struct Entry {
    name: Option<String>,
    version: Option<String>,
    resolved: Option<String>,
    integrity: Option<String>,
    #[serde(default)]
    link: bool,
    #[serde(default, rename = "inBundle")]
    bundled: bool,
    #[serde(default, rename = "hasInstallScript")]
    install_script: bool,
}

impl Entry {
    fn has_origin(&self) -> bool {
        self.resolved.is_some() || self.integrity.is_some() || self.link
    }
}

/// A row of the older `dependencies` section version 2 keeps beside `packages`, for older npm.
#[derive(Deserialize)]
struct Legacy {
    version: String,
    resolved: Option<String>,
    integrity: Option<String>,
    #[serde(default)]
    bundled: bool,
    #[serde(default)]
    dependencies: BTreeMap<String, Legacy>,
}

/// What `packages` says of a package; the older section must say the same.
type Listed<'a> = BTreeSet<(&'a str, &'a str, Option<&'a str>, Option<&'a str>)>;

pub(super) fn parse(text: &str) -> Result<Vec<Package>> {
    let lock: Lock = serde_json::from_str(text).context("package-lock.json does not parse")?;
    ensure!(matches!(lock.version, 2 | 3), "package-lock.json version {} is not supported", lock.version);
    let packages = lock.packages.context("package-lock.json has no packages")?;
    let packages: Vec<Package> =
        packages.iter().filter_map(|(path, entry)| package_at(path, entry, &packages).transpose()).collect::<Result<_>>()?;
    let listed: Listed<'_> = packages
        .iter()
        .map(|p| (p.name.0.as_str(), p.version.0.as_str(), p.origin.checksum.as_deref(), p.origin.source.as_deref()))
        .collect();
    ensure_agrees(&lock.dependencies, &listed)?;
    Ok(packages)
}

/// Older npm installs from `dependencies`, which the summary does not read: a row `packages` does not explain is refused.
/// A version that is not semver (git, a folder, an alias) has no row to compare with.
fn ensure_agrees(dependencies: &BTreeMap<String, Legacy>, listed: &Listed<'_>) -> Result<()> {
    for (name, entry) in dependencies {
        let row = (name.as_str(), entry.version.as_str(), entry.integrity.as_deref(), entry.resolved.as_deref());
        let explained = entry.bundled || Version(entry.version.clone()).numbers().is_none() || listed.contains(&row);
        ensure!(explained, "the dependencies section of package-lock.json disagrees with its packages on {name}");
        ensure_agrees(&entry.dependencies, listed)?;
    }
    Ok(())
}

/// The folder and the package name of a path that installs under `node_modules`.
fn installed_at(path: &str) -> Option<(&str, &str)> {
    path.rsplit_once("node_modules/").filter(|(folder, _)| folder.is_empty() || folder.ends_with('/'))
}

/// The package at `path`. The project and its workspace folders install nothing, but one with a source of its own is no such thing.
fn package_at(path: &str, written: &serde_json::Value, packages: &BTreeMap<String, serde_json::Value>) -> Result<Option<Package>> {
    let entry = Entry::deserialize(written).with_context(|| format!("{path} does not parse"))?;
    let installed = installed_at(path);
    if installed.is_none() && !entry.has_origin() {
        return Ok(None);
    }
    let version = if entry.link { Some("link".to_owned()) } else { entry.version.clone() };
    let version = Version(version.with_context(|| format!("{path} has no version"))?);
    let (name, site) = match installed {
        Some((folder, name)) => {
            let inside_a_package = !folder.is_empty() && packages.contains_key(folder.trim_end_matches('/'));
            (name, site_of(inside_a_package, name, &version, &entry))
        }
        None => (path, unproven(&entry)),
    };
    Ok(Some(Package {
        name: PackageName(name.to_owned()),
        version,
        site,
        install_script: entry.install_script,
        at_top: installed.is_some_and(|(folder, _)| folder.is_empty()),
        origin: Origin { checksum: entry.integrity, source: entry.resolved },
    }))
}

fn unproven(entry: &Entry) -> Site {
    Site::Unproven { shown: entry.resolved.clone().unwrap_or_else(|| "no source".to_owned()) }
}

/// A bundled copy comes with the package around it, so it needs one and can have nothing of its own; an alias is another package
/// than its path says.
fn site_of(inside_a_package: bool, name: &str, version: &Version, entry: &Entry) -> Site {
    if entry.name.as_deref().is_some_and(|given| given != name) {
        return unproven(entry);
    }
    if entry.bundled {
        let shipped = inside_a_package && !entry.has_origin();
        return if shipped { Site::Bundled } else { unproven(entry) };
    }
    let Some(resolved) = &entry.resolved else { return unproven(entry) };
    if entry.link {
        return Site::Link { target: resolved.clone() };
    }
    Site::git(resolved).or_else(|| registry_site(resolved, name, version)).unwrap_or_else(|| unproven(entry))
}

/// The registry behind a tarball URL, when the URL is https, says what it means with nothing a parser would rewrite,
/// and ends in the one path a registry gives this name and version.
fn registry_site(resolved: &str, name: &str, version: &Version) -> Option<Site> {
    let url = url::Url::parse(resolved).ok()?;
    let plain = url.scheme() == "https"
        && url.as_str() == resolved
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none();
    if !plain {
        return None;
    }
    let base = tarball_paths(name, version).iter().find_map(|path| resolved.strip_suffix(path.as_str()))?;
    Some(Site::Registry { base: base.to_owned() })
}

/// `<name>/-/<basename>-<version>.tgz`; a scope may be written with its slash encoded.
fn tarball_paths(name: &str, version: &Version) -> Vec<String> {
    match name.split_once('/') {
        None => vec![format!("/{name}/-/{name}-{version}.tgz")],
        Some((scope, base)) if scope.starts_with('@') && !base.contains('/') => {
            vec![format!("/{scope}/{base}/-/{base}-{version}.tgz"), format!("/{scope}%2f{base}/-/{base}-{version}.tgz")]
        }
        Some(_) => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::super::{ChangeKind, compare, summarize, warned_names};
    use super::*;

    fn named<'a>(packages: &'a [Package], name: &str) -> Vec<&'a Package> {
        packages.iter().filter(|p| p.name.0 == name).collect()
    }

    #[test]
    fn nested_copies_read_under_their_package_name_and_the_project_is_left_out() {
        let packages = parse(include_str!("fixtures/package_base.json")).unwrap();
        assert_eq!(named(&packages, "ms").len(), 2, "the top copy and the one nested under debug");
        assert_eq!(named(&packages, "@acme/left-pad")[0].origin.checksum.as_deref(), Some("sha512-aaaa"));
        assert_eq!(named(&packages, "widgets"), Vec::<&Package>::new());
    }

    #[test]
    fn copies_moving_in_the_tree_show_no_change_and_a_changed_integrity_warns() {
        let base = parse(include_str!("fixtures/package_base.json")).unwrap();
        let head = parse(include_str!("fixtures/package_head.json")).unwrap();
        let changes = compare(&base, &head, PUBLIC_REGISTRY);
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert_eq!(changes[0].name.0, "@acme/left-pad");
        assert!(matches!(changes[0].kind, ChangeKind::SameVersion { .. }));
    }

    #[test]
    fn version_2_reads_its_packages_map_too() {
        let text = r#"{"lockfileVersion": 2, "packages": {"node_modules/a": {"version": "1.0.0"}}, "dependencies": {}}"#;
        assert_eq!(parse(text).unwrap().len(), 1);
    }

    const PATH: &str = "package-lock.json";
    const GOOD: &str = r#""version":"1.0.0","resolved":"https://registry.npmjs.org/a/-/a-1.0.0.tgz","integrity":"sha512-aaaa""#;
    const NESTED_LINK: &str = r#""node_modules/a":{"resolved":"../../evil","link":true},"node_modules/b/node_modules/a":{GOOD}"#;

    fn lock(entries: &str) -> String {
        format!(r#"{{"lockfileVersion":3,"packages":{{"":{{"name":"widgets"}},{}}}}}"#, entries.replace("GOOD", GOOD))
    }

    fn good_a() -> String {
        lock(r#""node_modules/a":{GOOD}"#)
    }

    #[test]
    fn a_bump_to_another_host_without_integrity_warns() {
        let base = lock(
            r#""node_modules/left-pad":{"version":"1.3.0","resolved":"https://registry.npmjs.org/left-pad/-/left-pad-1.3.0.tgz","integrity":"sha512-aaaa"}"#,
        );
        let head = lock(r#""node_modules/left-pad":{"version":"1.3.1","resolved":"https://evil.example/x.tgz"}"#);
        assert_eq!(warned_names(PATH, &base, &head), ["left-pad"]);
    }

    #[test]
    fn a_bump_on_the_same_registry_does_not_warn() {
        let head = lock(
            r#""node_modules/a":{"version":"1.0.1","resolved":"https://registry.npmjs.org/a/-/a-1.0.1.tgz","integrity":"sha512-bbbb"}"#,
        );
        assert_eq!(warned_names(PATH, &good_a(), &head), [""; 0]);
    }

    #[test]
    fn a_new_package_off_the_registry_warns() {
        let head = lock(r#""node_modules/a":{GOOD},"node_modules/b":{"version":"1.0.0","resolved":"https://evil.example/x.tgz"}"#);
        assert_eq!(warned_names(PATH, &good_a(), &head), ["b"]);
    }

    #[test]
    fn a_new_package_on_a_new_host_shaped_like_a_registry_warns() {
        let head =
            lock(r#""node_modules/a":{GOOD},"node_modules/b":{"version":"1.0.0","resolved":"https://evil.example/b/-/b-1.0.0.tgz"}"#);
        assert_eq!(warned_names(PATH, &good_a(), &head), ["b"]);
    }

    #[test]
    fn a_new_tarball_on_a_known_host_off_the_registry_path_warns() {
        let head = lock(r#""node_modules/a":{GOOD},"node_modules/b":{"version":"1.0.0","resolved":"https://registry.npmjs.org/b.tgz"}"#);
        assert_eq!(warned_names(PATH, &good_a(), &head), ["b"]);
    }

    #[test]
    fn a_new_package_on_a_known_registry_does_not_warn() {
        let head = lock(
            r#""node_modules/a":{GOOD},"node_modules/b":{"version":"1.0.0","resolved":"https://registry.npmjs.org/b/-/b-1.0.0.tgz","integrity":"sha512-b"}"#,
        );
        assert_eq!(warned_names(PATH, &good_a(), &head), [""; 0]);
    }

    #[test]
    fn build_metadata_with_a_new_integrity_and_host_warns() {
        let head = lock(r#""node_modules/a":{"version":"1.0.0+x","resolved":"https://evil.example/x.tgz","integrity":"sha512-evil"}"#);
        assert_eq!(warned_names(PATH, &good_a(), &head), ["a"]);
    }

    #[test]
    fn build_metadata_with_only_a_new_integrity_warns() {
        let head = lock(
            r#""node_modules/a":{"version":"1.0.0+x","resolved":"https://registry.npmjs.org/a/-/a-1.0.0.tgz","integrity":"sha512-evil"}"#,
        );
        assert_eq!(warned_names(PATH, &good_a(), &head), ["a"]);
    }

    #[test]
    fn an_invisible_character_in_a_version_does_not_hide_a_new_integrity() {
        let head = lock(
            "\"node_modules/a\":{\"version\":\"1.0.0\u{200b}\",\"resolved\":\"https://registry.npmjs.org/a/-/a-1.0.0.tgz\",\"integrity\":\"sha512-evil\"}",
        );
        assert_eq!(warned_names(PATH, &good_a(), &head), ["a"]);
    }

    #[test]
    fn a_top_copy_turned_link_warns_while_a_nested_copy_stays() {
        let base = lock(r#""node_modules/a":{GOOD},"node_modules/b/node_modules/a":{GOOD}"#);
        assert!(warned_names(PATH, &base, &lock(NESTED_LINK)).contains(&"a".to_owned()));
    }

    #[test]
    fn the_only_copy_turned_link_warns() {
        let head = lock(r#""node_modules/a":{"resolved":"vendor/evil","link":true},"vendor/evil":{"version":"1.0.0"}"#);
        assert_eq!(warned_names(PATH, &good_a(), &head), ["a"]);
    }

    #[test]
    fn a_link_turned_registry_copy_warns() {
        let base = lock(r#""node_modules/a":{"resolved":"vendor/a","link":true}"#);
        assert_eq!(warned_names(PATH, &base, &good_a()), ["a"]);
    }

    #[test]
    fn a_new_dependency_that_is_a_link_warns() {
        let head = lock(r#""node_modules/a":{GOOD},"node_modules/b":{"resolved":"../../evil","link":true}"#);
        assert_eq!(warned_names(PATH, &good_a(), &head), ["b"]);
    }

    #[test]
    fn an_unchanged_link_does_not_warn() {
        let both = lock(r#""node_modules/a":{"resolved":"vendor/a","link":true}"#);
        assert_eq!(warned_names(PATH, &both, &both), [""; 0]);
    }

    #[test]
    fn swapping_the_top_copy_with_a_nested_one_shows_the_downgrade() {
        let one = r#""version":"1.0.0","resolved":"https://registry.npmjs.org/a/-/a-1.tgz","integrity":"sha512-one""#;
        let two = r#""version":"2.0.0","resolved":"https://registry.npmjs.org/a/-/a-2.tgz","integrity":"sha512-two""#;
        let tree = |top: &str, nested: &str| lock(&format!(r#""node_modules/a":{{{top}}},"node_modules/b/node_modules/a":{{{nested}}}"#));
        let summary = summarize(PATH, Some(&tree(two, one)), Some(&tree(one, two))).unwrap();
        assert_eq!(summary.changes.len(), 1, "{summary:?}");
        assert!(matches!(&summary.changes[0].kind, ChangeKind::Updated { from, to, .. } if from.0 == "2.0.0" && to.0 == "1.0.0"));
    }

    fn lock_v2(dependency: &str) -> String {
        format!(r#"{{"lockfileVersion":2,"packages":{{"node_modules/a":{{{GOOD}}}}},"dependencies":{{"a":{{{dependency}}}}}}}"#)
    }

    #[test]
    fn a_v2_dependencies_section_that_disagrees_with_packages_is_refused() {
        assert!(parse(&lock_v2(r#""version":"1.0.0","resolved":"https://evil.example/x.tgz","integrity":"sha512-evil""#)).is_err());
    }

    #[test]
    fn a_nested_v2_dependency_that_disagrees_with_packages_is_refused() {
        let nested = format!(r#"{GOOD},"dependencies":{{"b":{{"version":"9.0.0"}}}}"#);
        assert!(parse(&lock_v2(&nested)).is_err());
    }

    #[test]
    fn a_bundled_v2_dependency_needs_no_row_in_packages() {
        let nested = format!(r#"{GOOD},"dependencies":{{"b":{{"version":"9.0.0","bundled":true}}}}"#);
        assert_eq!(parse(&lock_v2(&nested)).unwrap().len(), 1);
    }

    #[test]
    fn a_v2_dependency_on_git_needs_no_row_in_packages() {
        let nested = format!(r#"{GOOD},"dependencies":{{"b":{{"version":"git+ssh://git@example.com/b.git#abc"}}}}"#);
        assert_eq!(parse(&lock_v2(&nested)).unwrap().len(), 1);
    }

    #[test]
    fn version_1_is_refused_even_with_a_packages_map() {
        assert!(parse(r#"{"lockfileVersion":1,"packages":{}}"#).is_err());
    }

    #[test]
    fn a_v2_dependencies_section_that_agrees_with_packages_reads() {
        assert_eq!(parse(&lock_v2(GOOD)).unwrap().len(), 1);
    }

    #[test]
    fn an_install_script_going_true_warns() {
        assert_eq!(warned_names(PATH, &good_a(), &lock(r#""node_modules/a":{GOOD,"hasInstallScript":true}"#)), ["a"]);
    }

    #[test]
    fn an_install_script_going_away_makes_no_row() {
        let base = lock(r#""node_modules/a":{GOOD,"hasInstallScript":true}"#);
        assert_eq!(summarize(PATH, Some(&base), Some(&good_a())).unwrap().changes, vec![]);
    }

    #[test]
    fn a_new_name_or_dependency_list_makes_no_row() {
        let head = lock(r#""node_modules/a":{"name":"other",GOOD,"dependencies":{"c":"*"}}"#);
        assert_eq!(summarize(PATH, Some(&good_a()), Some(&head)).unwrap().changes, vec![]);
    }

    #[test]
    fn a_bundled_copy_of_a_known_version_does_not_warn() {
        let head = lock(
            r#""node_modules/a":{GOOD},"node_modules/x":{"version":"1.0.0","resolved":"https://registry.npmjs.org/x/-/x-1.0.0.tgz","integrity":"sha512-x"},"node_modules/x/node_modules/a":{"version":"1.0.0","inBundle":true}"#,
        );
        assert_eq!(warned_names(PATH, &good_a(), &head), [""; 0]);
    }

    #[test]
    fn a_copy_turning_bundled_warns() {
        let head = lock(r#""node_modules/x/node_modules/a":{"version":"1.0.0","inBundle":true}"#);
        assert_eq!(warned_names(PATH, &good_a(), &head), ["a"]);
    }

    #[test]
    fn a_format_not_read_whole_is_refused() {
        assert!(parse(r#"{"lockfileVersion": 1, "dependencies": {}}"#).is_err());
        assert!(parse(r#"{"lockfileVersion": 3}"#).is_err());
        assert!(parse(r#"{"lockfileVersion": 3, "packages": {"node_modules/a": {}}}"#).is_err());
    }
}
