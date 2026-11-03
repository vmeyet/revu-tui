//! `package-lock.json`, the `packages` map of lockfile versions 2 and 3.
use super::raw_lines::{EntryLines, significant};
use super::{Origin, Package, PackageName, Site, Version, is_semver};
use anyhow::{Context as _, Result, bail, ensure};
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
    if let Some(path) = packages.keys().find(|path| !is_plain_path(path)) {
        bail!("package-lock.json has an entry at {path:?}, which is no plain install path");
    }
    let packages: Vec<Package> =
        packages.iter().filter_map(|(path, entry)| package_at(path, entry, &packages).transpose()).collect::<Result<_>>()?;
    if lock.dependencies.is_empty() {
        return Ok(packages);
    }
    let listed: Listed<'_> = packages
        .iter()
        .map(|p| (p.name.0.as_str(), p.version.0.as_str(), p.origin.checksum.as_deref(), p.origin.source.as_deref()))
        .collect();
    ensure_agrees(&lock.dependencies, &listed)?;
    Ok(packages.into_iter().map(drawn_twice).collect())
}

/// The older section writes each drawn field again; `ensure_agrees` proved it says the same, so a row accounts for both.
fn drawn_twice(package: Package) -> Package {
    let drawn = package.lines.drawn.iter().chain(&package.lines.drawn).cloned().collect();
    Package { lines: EntryLines { drawn, ..package.lines }, ..package }
}

/// Older npm installs from `dependencies`, which the summary does not read: only a row `packages` says the same of reads.
fn ensure_agrees(dependencies: &BTreeMap<String, Legacy>, listed: &Listed<'_>) -> Result<()> {
    for (name, entry) in dependencies {
        let row = (name.as_str(), entry.version.as_str(), entry.integrity.as_deref(), entry.resolved.as_deref());
        let explained = is_semver(&entry.version) && listed.contains(&row) && !(entry.bundled && entry.resolved.is_some());
        ensure!(explained, "the dependencies section of package-lock.json disagrees with its packages on {name}");
        ensure_agrees(&entry.dependencies, listed)?;
    }
    Ok(())
}

/// A path npm itself writes: relative, with no step it would have to resolve, and `..` only to climb above the project at the start.
fn is_plain_path(path: &str) -> bool {
    let plain = |step: &str| {
        let other_spelling = step != "node_modules" && step.eq_ignore_ascii_case("node_modules");
        !(step.is_empty() || step == "." || step == ".." || step.contains('\\') || other_spelling)
    };
    path.is_empty() || path.trim_start_matches("../").split('/').all(plain)
}

/// The fields that start with `_`, written out: npm reads some of them as a source, and nothing proves what the rest do.
fn underscore_fields(written: &serde_json::Value) -> Vec<String> {
    let fields = written.as_object().into_iter().flatten();
    fields.filter(|(key, _)| key.starts_with('_')).map(|(key, value)| format!("{key}={value}")).collect()
}

/// The folder and the package name of a path that installs under `node_modules`.
fn installed_at(path: &str) -> Option<(&str, &str)> {
    path.rsplit_once("node_modules/").filter(|(folder, _)| folder.is_empty() || folder.ends_with('/'))
}

/// The package at `path`. The project and its workspace folders install nothing, but one with a source of its own is no such thing.
fn package_at(path: &str, written: &serde_json::Value, packages: &BTreeMap<String, serde_json::Value>) -> Result<Option<Package>> {
    let entry = Entry::deserialize(written).with_context(|| format!("{path} does not parse"))?;
    let installed = installed_at(path);
    let underscored = underscore_fields(written).join(" ");
    if installed.is_none() && !entry.has_origin() && underscored.is_empty() {
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
    let site = if underscored.is_empty() { site } else { Site::Unproven { shown: underscored.clone() } };
    let source =
        if underscored.is_empty() { entry.resolved } else { Some(format!("{underscored} {}", entry.resolved.unwrap_or_default())) };
    Ok(Some(Package {
        name: PackageName(name.to_owned()),
        version,
        site,
        install_script: entry.install_script,
        place: path.to_owned(),
        lines: lines_of(path, written),
        checksum_has_form: entry.integrity.as_deref().is_some_and(is_integrity),
        origin: Origin { checksum: entry.integrity, source },
    }))
}

/// The lines npm writes for this entry, which a row splits by whether it draws them.
fn lines_of(path: &str, written: &serde_json::Value) -> EntryLines {
    let pretty = format!("{written:#}");
    let (drawn, rest) = pretty
        .lines()
        .filter_map(significant)
        .map(str::to_owned)
        .partition(|line| ["\"version\":", "\"resolved\":", "\"integrity\":"].iter().any(|field| line.starts_with(field)));
    EntryLines { key: format!("{}: {{", serde_json::Value::from(path)), drawn, rest }
}

/// A sha512 or, for packages old enough to have nothing else, a sha1; npm may write both.
fn is_integrity(text: &str) -> bool {
    let is_hash = |hash: &str| match hash.split_once('-') {
        Some(("sha512", value)) => is_base64(value, 86, "=="),
        Some(("sha1", value)) => is_base64(value, 27, "="),
        _ => false,
    };
    text.split(' ').all(is_hash)
}

fn is_base64(value: &str, length: usize, padding: &str) -> bool {
    value
        .strip_suffix(padding)
        .is_some_and(|body| body.len() == length && body.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/'))
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
    use super::super::{Alert, ChangeKind, compare, summarize, warned_names};
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
    /// A well-formed sha512 value, of no real file.
    const HASH: &str = "sha512-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==";
    const GOOD: &str = r#""version":"1.0.0","resolved":"https://registry.npmjs.org/a/-/a-1.0.0.tgz","integrity":"sha512-aaaa""#;
    const NESTED_LINK: &str = r#""node_modules/a":{"resolved":"../../evil","link":true},"node_modules/b/node_modules/a":{GOOD}"#;

    fn lock(entries: &str) -> String {
        format!(r#"{{"lockfileVersion":3,"packages":{{"":{{"name":"widgets"}},{}}}}}"#, entries.replace("GOOD", GOOD).replace("HASH", HASH))
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
        let head =
            lock(r#""node_modules/a":{"version":"1.0.1","resolved":"https://registry.npmjs.org/a/-/a-1.0.1.tgz","integrity":"HASH"}"#);
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
            r#""node_modules/a":{GOOD},"node_modules/b":{"version":"1.0.0","resolved":"https://registry.npmjs.org/b/-/b-1.0.0.tgz","integrity":"HASH"}"#,
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
        let downgrade = |kind: &ChangeKind| matches!(kind, ChangeKind::Updated { from, to, .. } if from.0 == "2.0.0" && to.0 == "1.0.0");
        assert!(summary.changes.iter().any(|c| downgrade(&c.kind)), "{summary:?}");
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
            r#""node_modules/a":{GOOD},"node_modules/x":{"version":"1.0.0","resolved":"https://registry.npmjs.org/x/-/x-1.0.0.tgz","integrity":"HASH"},"node_modules/x/node_modules/a":{"version":"1.0.0","inBundle":true}"#,
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

    const PUBLIC: &str = PUBLIC_REGISTRY;

    fn entry(path: &str, fields: &str) -> String {
        format!(r#""{path}":{{{fields}}}"#)
    }

    fn at(version: &str, resolved: &str) -> String {
        format!(r#""version":"{version}","resolved":"{resolved}","integrity":"{HASH}""#)
    }

    fn public(name: &str, version: &str) -> String {
        let base_name = name.rsplit('/').next().unwrap_or(name);
        at(version, &format!("{PUBLIC}/{name}/-/{base_name}-{version}.tgz"))
    }

    fn a_is(fields: &str) -> String {
        lock(&entry("node_modules/a", fields))
    }

    /// The rows that warn when `a` bumps from the public 1.0.0 to `version` at `url`.
    fn bump_to(version: &str, url: &str) -> Vec<String> {
        warned_names(PATH, &good_a(), &a_is(&at(version, url)))
    }

    /// The rows that warn when `path` appears beside the public `a` with these fields.
    fn new_at(path: &str, fields: &str) -> Vec<String> {
        warned_names(PATH, &good_a(), &lock(&format!(r#""node_modules/a":{{GOOD}},{}"#, entry(path, fields))))
    }

    fn new_package(fields: &str) -> Vec<String> {
        new_at("node_modules/new", fields)
    }

    /// The names that warn in a recorded case, read as added when it has no base.
    macro_rules! case {
        ($stem:literal) => {
            warned_names(
                PATH,
                include_str!(concat!("fixtures/npm_cases/", $stem, ".base.json")),
                include_str!(concat!("fixtures/npm_cases/", $stem, ".head.json")),
            )
        };
        (added $stem:literal) => {
            warned_when_added(include_str!(concat!("fixtures/npm_cases/", $stem, ".head.json")))
        };
    }

    fn warned_when_added(head: &str) -> Vec<String> {
        let summary = summarize(PATH, None, Some(head)).unwrap();
        summary.changes.iter().filter(|c| c.kind.warns()).map(|c| c.name.to_string()).collect()
    }

    #[test]
    fn an_added_lock_on_the_public_registry_does_not_warn() {
        assert_eq!(case!(added "control"), [""; 0]);
    }

    #[test]
    fn a_bump_resolved_to_another_package_on_the_same_registry_warns() {
        assert_eq!(case!("x01_other_pkg_same_registry"), ["a"]);
    }

    #[test]
    fn a_bump_to_another_package_warns_with_a_name_field_too() {
        assert_eq!(case!("x01b_with_name_field"), ["a"]);
    }

    #[test]
    fn a_new_transitive_copy_from_another_host_flagged_bundled_warns() {
        assert_eq!(case!("x02_new_transitive_inbundle_evil"), ["b", "new"], "the parent's own tarball changed in this case too");
    }

    #[test]
    fn a_direct_dependency_from_another_host_flagged_bundled_warns() {
        assert_eq!(case!(added "x02_direct_dep_inbundle_evil"), ["new"]);
    }

    #[test]
    fn a_top_copy_flagged_bundled_warns_while_a_nested_copy_stays_good() {
        assert_eq!(case!("x02c_top_copy_evil_inbundle_nested_stays"), ["a"]);
    }

    #[test]
    fn an_added_lock_on_another_host_shaped_like_the_registry_warns() {
        assert_eq!(case!(added "x03_added_lock_registry_shaped_evil"), ["a"]);
    }

    #[test]
    fn a_bump_from_https_to_http_warns() {
        assert_eq!(case!("x06_http"), ["a"]);
    }

    #[test]
    fn a_v_prefixed_version_warns() {
        assert_eq!(case!("x07_v_prefix_version"), ["a"]);
    }

    #[test]
    fn an_entry_with_a_source_outside_node_modules_warns() {
        assert_eq!(case!("x09_outside_node_modules"), ["vendor/x"]);
    }

    #[test]
    fn copies_swapping_their_origins_warn_at_each_path() {
        assert_eq!(case!("x10b_origin_swap"), ["a"]);
    }

    #[test]
    fn a_workspace_folder_with_no_source_makes_no_row() {
        let head = lock(r#""node_modules/a":{GOOD},"packages/tool":{"version":"0.1.0"}"#);
        assert_eq!(summarize(PATH, Some(&good_a()), Some(&head)).unwrap().changes, vec![]);
    }

    #[test]
    fn a_slash_dash_slash_elsewhere_in_the_url_proves_nothing() {
        assert_eq!(bump_to("1.0.1", &format!("{PUBLIC}/-/a-1.0.1.tgz")), ["a"]);
        assert_eq!(bump_to("1.0.1", &format!("{PUBLIC}/a/-/evil/a-1.0.1.tgz")), ["a"]);
    }

    #[test]
    fn a_url_for_another_version_warns() {
        assert_eq!(bump_to("1.0.1", &format!("{PUBLIC}/a/-/a-1.0.0.tgz")), ["a"]);
    }

    #[test]
    fn a_url_with_a_query_or_fragment_warns() {
        assert_eq!(bump_to("1.0.1", &format!("{PUBLIC}/a/-/a-1.0.1.tgz?x=1")), ["a"]);
        assert_eq!(bump_to("1.0.1", &format!("{PUBLIC}/a/-/a-1.0.1.tgz#x")), ["a"]);
    }

    #[test]
    fn a_url_with_credentials_or_a_port_warns() {
        assert_eq!(bump_to("1.0.1", "https://nina@registry.npmjs.org/a/-/a-1.0.1.tgz"), ["a"]);
        assert_eq!(bump_to("1.0.1", "https://registry.npmjs.org:8443/a/-/a-1.0.1.tgz"), ["a"]);
    }

    #[test]
    fn a_url_a_parser_would_rewrite_warns() {
        assert_eq!(bump_to("1.0.1", &format!("{PUBLIC}/evil/../a/-/a-1.0.1.tgz")), ["a"]);
        assert_eq!(bump_to("1.0.1", "https://REGISTRY.npmjs.org/a/-/a-1.0.1.tgz"), ["a"]);
    }

    /// The rows that warn on a bump inside `base`, which the old side already used: only a form that proves nothing can warn.
    fn bump_inside(base: &str) -> Vec<String> {
        let a = |version: &str| a_is(&at(version, &format!("{base}/a/-/a-{version}.tgz")));
        warned_names(PATH, &a("1.0.0"), &a("1.0.1"))
    }

    #[test]
    fn a_base_that_already_used_http_stays_unproven() {
        assert_eq!(bump_inside("http://npm.acme.example"), ["a"]);
    }

    #[test]
    fn a_base_that_already_held_credentials_stays_unproven() {
        assert_eq!(bump_inside("https://nina@npm.acme.example"), ["a"]);
        assert_eq!(bump_inside("https://:secret@npm.acme.example"), ["a"]);
    }

    #[test]
    fn a_base_that_already_held_a_query_or_fragment_stays_unproven() {
        assert_eq!(bump_inside("https://npm.acme.example/x?y="), ["a"]);
        assert_eq!(bump_inside("https://npm.acme.example/x#y"), ["a"]);
    }

    #[test]
    fn a_base_a_parser_would_rewrite_stays_unproven() {
        assert_eq!(bump_inside("https://npm.acme.example/evil/.."), ["a"]);
    }

    #[test]
    fn a_url_going_on_past_the_tarball_path_warns() {
        assert_eq!(bump_to("1.0.1", &format!("{PUBLIC}/a/-/a-1.0.1.tgz/evil.tgz")), ["a"]);
    }

    #[test]
    fn a_name_with_a_slash_and_no_scope_warns() {
        assert_eq!(new_at("node_modules/x/y", &at("1.0.0", &format!("{PUBLIC}/x/y/-/y-1.0.0.tgz"))), ["x/y"]);
    }

    #[test]
    fn a_name_field_other_than_the_path_warns_though_the_url_fits_the_path() {
        assert_eq!(new_package(&format!(r#""name":"evil",{}"#, public("new", "1.0.0"))), ["new"]);
    }

    #[test]
    fn a_folder_merely_ending_like_node_modules_installs_nothing() {
        let head = lock(r#""node_modules/a":{GOOD},"vendor_node_modules/x":{"version":"1.0.0"}"#);
        assert_eq!(summarize(PATH, Some(&good_a()), Some(&head)).unwrap().changes, vec![]);
    }

    #[test]
    fn a_prefix_on_the_public_host_is_another_base_and_warns() {
        assert_eq!(bump_to("1.0.1", &format!("{PUBLIC}/evil/a/-/a-1.0.1.tgz")), ["a"]);
    }

    #[test]
    fn a_package_on_a_private_registry_the_base_already_uses_does_not_warn() {
        let private = "https://npm.acme.example/api/npm/virtual";
        let a = |version: &str| entry("node_modules/a", &at(version, &format!("{private}/a/-/a-{version}.tgz")));
        let new = entry("node_modules/b", &at("1.0.0", &format!("{private}/b/-/b-1.0.0.tgz")));
        assert_eq!(warned_names(PATH, &lock(&a("1.0.0")), &lock(&format!("{},{new}", a("1.0.1")))), [""; 0]);
    }

    #[test]
    fn a_first_private_registry_warns() {
        assert_eq!(new_package(&at("1.0.0", "https://npm.acme.example/new/-/new-1.0.0.tgz")), ["new"]);
    }

    #[test]
    fn another_prefix_on_a_known_private_host_warns() {
        let base = a_is(&at("1.0.0", "https://npm.acme.example/one/a/-/a-1.0.0.tgz"));
        let head = a_is(&at("1.0.1", "https://npm.acme.example/two/a/-/a-1.0.1.tgz"));
        assert_eq!(warned_names(PATH, &base, &head), ["a"]);
    }

    #[test]
    fn both_scoped_url_forms_do_not_warn() {
        let scoped = |url: String| new_at("node_modules/@acme/new", &at("1.0.0", &url));
        assert_eq!(scoped(format!("{PUBLIC}/@acme/new/-/new-1.0.0.tgz")), [""; 0]);
        assert_eq!(scoped(format!("{PUBLIC}/@acme%2fnew/-/new-1.0.0.tgz")), [""; 0]);
    }

    #[test]
    fn a_scoped_url_of_another_scope_warns() {
        assert_eq!(new_at("node_modules/@acme/new", &at("1.0.0", &format!("{PUBLIC}/@evil/new/-/new-1.0.0.tgz"))), ["@acme/new"]);
    }

    #[test]
    fn an_alias_warns() {
        let head = a_is(&format!(r#""name":"b",{}"#, public("b", "1.0.0")));
        assert_eq!(warned_names(PATH, &good_a(), &head), ["a"]);
    }

    #[test]
    fn a_bundled_entry_with_an_integrity_warns() {
        assert_eq!(new_at("node_modules/a/node_modules/new", r#""version":"1.0.0","integrity":"sha512-any","inBundle":true"#), ["new"]);
    }

    #[test]
    fn a_bundled_entry_with_a_resolved_alone_warns() {
        assert_eq!(
            new_at("node_modules/a/node_modules/new", r#""version":"1.0.0","resolved":"https://evil.example/x.tgz","inBundle":true"#),
            ["new"]
        );
    }

    #[test]
    fn a_bundled_entry_with_a_link_alone_warns() {
        assert_eq!(new_at("node_modules/a/node_modules/new", r#""version":"1.0.0","link":true,"inBundle":true"#), ["new"]);
    }

    #[test]
    fn a_bump_onto_a_bundled_copy_with_no_origin_warns() {
        let head = lock(&format!(
            "{},{}",
            entry("node_modules/x", &public("x", "1.0.0")),
            entry("node_modules/x/node_modules/a", r#""version":"1.0.1","inBundle":true"#)
        ));
        let base = lock(&format!("{},{}", entry("node_modules/x", &public("x", "1.0.0")), entry("node_modules/a", &public("a", "1.0.0"))));
        assert_eq!(warned_names(PATH, &base, &head), ["a"]);
    }

    #[test]
    fn a_bundled_entry_that_is_a_link_warns() {
        assert_eq!(new_at("node_modules/a/node_modules/new", r#""resolved":"../../evil","link":true,"inBundle":true"#), ["new"]);
    }

    #[test]
    fn a_bundled_copy_keeps_its_origin_in_the_comparison() {
        let fields = format!(r#"{},"inBundle":true"#, at("1.0.0", "https://evil.example/x.tgz"));
        assert_eq!(new_at("node_modules/z/node_modules/a", &fields), ["a"]);
    }

    #[test]
    fn a_bundled_entry_at_the_top_has_no_package_to_ship_it_and_warns() {
        assert_eq!(new_package(r#""version":"1.0.0","inBundle":true"#), ["new"]);
    }

    #[test]
    fn a_bundled_entry_under_a_package_the_lock_does_not_hold_warns() {
        assert_eq!(new_at("node_modules/ghost/node_modules/new", r#""version":"1.0.0","inBundle":true"#), ["new"]);
    }

    #[test]
    fn a_new_bundled_entry_with_no_source_does_not_warn() {
        assert_eq!(new_at("node_modules/a/node_modules/new", r#""version":"1.0.0","inBundle":true"#), [""; 0]);
    }

    const REPO: &str = "git+ssh://git@github.com/acme/a.git";

    fn on_git(version: &str, repo: &str, commit: &str) -> String {
        a_is(&format!(r#""version":"{version}","resolved":"{repo}#{commit}""#))
    }

    #[test]
    fn a_git_bump_inside_one_repository_does_not_warn() {
        assert_eq!(warned_names(PATH, &on_git("1.0.0", REPO, "aaa"), &on_git("1.1.0", REPO, "bbb")), [""; 0]);
    }

    #[test]
    fn a_git_bump_to_a_fork_on_the_same_host_warns() {
        let fork = on_git("1.1.0", "git+ssh://git@github.com/evil/a.git", "bbb");
        assert_eq!(warned_names(PATH, &on_git("1.0.0", REPO, "aaa"), &fork), ["a"]);
    }

    #[test]
    fn git_with_no_commit_warns() {
        let head = a_is(&format!(r#""version":"1.1.0","resolved":"{REPO}""#));
        assert_eq!(warned_names(PATH, &on_git("1.0.0", REPO, "aaa"), &head), ["a"]);
        assert_eq!(warned_names(PATH, &on_git("1.0.0", REPO, "aaa"), &on_git("1.1.0", REPO, "")), ["a"]);
    }

    #[test]
    fn a_base_with_no_sources_leaves_only_the_public_registry_ordinary() {
        let base = lock(r#""node_modules/z":{"version":"1.0.0"}"#);
        let with = |fields: &str| lock(&format!(r#""node_modules/z":{{"version":"1.0.0"}},{}"#, entry("node_modules/n", fields)));
        assert_eq!(warned_names(PATH, &base, &with(&at("1.0.0", "https://evil.example/n/-/n-1.0.0.tgz"))), ["n"]);
        assert_eq!(warned_names(PATH, &base, &with(&public("n", "1.0.0"))), [""; 0]);
    }

    fn with_version(version: &str) -> Vec<String> {
        bump_to(version, &format!("{PUBLIC}/a/-/a-{version}.tgz"))
    }

    #[test]
    fn a_version_with_a_prefix_warns_though_its_url_fits() {
        assert_eq!(with_version("v1.0.1"), ["a"]);
        assert_eq!(with_version("=1.0.1"), ["a"]);
    }

    #[test]
    fn a_version_with_a_leading_zero_warns_though_its_url_fits() {
        assert_eq!(with_version("01.0.1"), ["a"]);
    }

    #[test]
    fn a_version_in_fullwidth_digits_warns() {
        assert_eq!(bump_to("\u{ff11}.0.1", &format!("{PUBLIC}/a/-/a-1.0.1.tgz")), ["a"]);
    }

    #[test]
    fn a_pre_release_and_a_build_of_the_right_form_do_not_warn() {
        assert_eq!(with_version("1.0.1-rc.1"), [""; 0]);
        assert_eq!(with_version("1.0.1+x"), [""; 0]);
    }

    #[test]
    fn build_metadata_with_a_new_integrity_warns_though_its_url_fits() {
        let head = a_is(r#""version":"1.0.0+x","resolved":"https://registry.npmjs.org/a/-/a-1.0.0+x.tgz","integrity":"sha512-evil""#);
        assert_eq!(warned_names(PATH, &good_a(), &head), ["a"]);
    }

    #[test]
    fn a_build_pairs_with_its_release_before_any_other_version() {
        let nested = |under: &str, fields: &str| entry(&format!("node_modules/{under}/node_modules/a"), fields);
        let base = lock(&format!("{},{}", nested("b", &public("a", "1.0.0")), nested("c", &public("a", "1.0.1"))));
        let build = at("1.0.1+x", "https://registry.npmjs.org/a/-/a-1.0.1+x.tgz").replace(HASH, "sha512-evil");
        let head = lock(&format!("{},{}", nested("d", &build), nested("e", &public("a", "1.0.2"))));
        let changes = summarize(PATH, Some(&base), Some(&head)).unwrap().changes;
        assert!(changes.iter().any(|c| c.kind.alerts().contains(&Alert::ChecksumChanged)), "{changes:?}");
    }

    #[test]
    fn a_new_package_with_no_integrity_warns() {
        assert_eq!(new_package(&format!(r#""version":"1.0.0","resolved":"{PUBLIC}/new/-/new-1.0.0.tgz""#)), ["new"]);
    }

    #[test]
    fn a_new_package_with_an_empty_integrity_warns() {
        assert_eq!(new_package(&format!(r#""version":"1.0.0","resolved":"{PUBLIC}/new/-/new-1.0.0.tgz","integrity":"""#)), ["new"]);
    }

    #[test]
    fn a_new_package_with_no_source_warns() {
        assert_eq!(new_package(r#""version":"1.0.0""#), ["new"]);
    }

    #[test]
    fn a_new_package_with_an_install_script_warns() {
        assert_eq!(new_package(&format!(r#"{},"hasInstallScript":true"#, public("new", "1.0.0"))), ["new"]);
    }

    #[test]
    fn nested_copies_swapping_versions_show_a_row() {
        let tree = |one: &str, two: &str| {
            let nested = |under: &str, version: &str| entry(&format!("node_modules/{under}/node_modules/a"), &public("a", version));
            lock(&format!("{},{}", nested("b", one), nested("c", two)))
        };
        let changes = summarize(PATH, Some(&tree("1.0.0", "2.0.0")), Some(&tree("2.0.0", "1.0.0"))).unwrap().changes;
        assert_eq!(changes.len(), 2, "{changes:?}");
    }

    #[test]
    fn copies_trading_two_known_registries_warn_though_each_version_keeps_the_same_set() {
        let private = |version: &str| at(version, &format!("https://npm.acme.example/a/-/a-{version}.tgz"));
        let tree = |top: String, nested: String| {
            lock(&format!("{},{}", entry("node_modules/a", &top), entry("node_modules/b/node_modules/a", &nested)))
        };
        let (base, head) = (tree(public("a", "1.0.0"), private("1.0.0")), tree(private("2.0.0"), public("a", "2.0.0")));
        assert_eq!(warned_names(PATH, &base, &head), ["a"]);
    }

    #[test]
    fn a_v2_row_with_a_url_as_its_version_is_refused() {
        assert!(parse(&lock_v2(r#""version":"https://evil.example/x.tgz","integrity":"sha512-evil""#)).is_err());
    }

    #[test]
    fn a_v2_row_with_git_as_its_version_is_refused() {
        let nested = format!(r#"{GOOD},"dependencies":{{"b":{{"version":"git+ssh://git@example.com/b.git#abc"}}}}"#);
        assert!(parse(&lock_v2(&nested)).is_err());
    }

    #[test]
    fn a_bundled_v2_row_with_a_resolved_is_refused() {
        assert!(
            parse(&lock_v2(r#""version":"1.0.0","resolved":"https://evil.example/x.tgz","integrity":"sha512-evil","bundled":true"#))
                .is_err()
        );
    }

    #[test]
    fn a_bundled_v2_row_with_a_resolved_is_refused_though_packages_says_the_same() {
        let same = r#""version":"1.0.0","resolved":"https://evil.example/x.tgz","integrity":"sha512-evil""#;
        let text = format!(
            r#"{{"lockfileVersion":2,"packages":{{"node_modules/a":{{{same},"inBundle":true}}}},"dependencies":{{"a":{{{same},"bundled":true}}}}}}"#
        );
        assert!(parse(&text).is_err());
    }

    #[test]
    fn a_bundled_v2_row_packages_does_not_list_is_refused() {
        let nested = format!(r#"{GOOD},"dependencies":{{"b":{{"version":"9.0.0","bundled":true}}}}"#);
        assert!(parse(&lock_v2(&nested)).is_err());
    }

    #[test]
    fn a_bundled_v2_row_packages_lists_reads() {
        let text = format!(
            r#"{{"lockfileVersion":2,"packages":{{"node_modules/a":{{{GOOD}}},"node_modules/a/node_modules/b":{{"version":"9.0.0","inBundle":true}}}},"dependencies":{{"a":{{{GOOD},"dependencies":{{"b":{{"version":"9.0.0","bundled":true}}}}}}}}}}"#
        );
        assert_eq!(parse(&text).unwrap().len(), 2);
    }

    #[test]
    fn a_v2_row_with_a_version_that_is_not_semver_is_refused_though_packages_says_the_same() {
        let same = r#""version":"v1.0.0","resolved":"https://registry.npmjs.org/a/-/a-1.0.0.tgz","integrity":"sha512-aaaa""#;
        let text = format!(r#"{{"lockfileVersion":2,"packages":{{"node_modules/a":{{{same}}}}},"dependencies":{{"a":{{{same}}}}}}}"#);
        assert!(parse(&text).is_err());
    }

    #[test]
    fn a_bundled_copy_of_a_known_version_with_underscore_fields_warns() {
        assert_eq!(case!("n_sameversion"), ["a"]);
    }

    #[test]
    fn a_bundled_copy_with_underscore_fields_warns_beside_a_routine_update() {
        assert_eq!(case!("n_sameversion_decoy"), ["a"]);
    }

    #[test]
    fn a_bundled_copy_of_a_new_version_with_underscore_fields_warns() {
        assert_eq!(case!("n_newversion"), ["a"]);
    }

    #[test]
    fn a_bundled_copy_with_a_proto_field_warns() {
        assert_eq!(case!("n_proto"), ["a"]);
    }

    #[test]
    fn an_underscore_field_appearing_on_an_ordinary_entry_warns() {
        let head = a_is(&format!(r#"{GOOD},"_resolved":"https://evil.example/x.tgz""#));
        assert_eq!(warned_names(PATH, &good_a(), &head), ["a"]);
    }

    #[test]
    fn an_underscore_field_changing_on_an_entry_warns() {
        let with = |url: &str| a_is(&format!(r#"{GOOD},"_resolved":"{url}""#));
        assert_eq!(warned_names(PATH, &with("https://one.example/x.tgz"), &with("https://two.example/x.tgz")), ["a"]);
    }

    #[test]
    fn an_underscore_field_on_a_new_package_warns() {
        assert_eq!(new_package(&format!(r#"{},"_anything":1"#, public("new", "1.0.0"))), ["new"]);
    }

    #[test]
    fn an_underscore_field_outside_node_modules_warns() {
        assert_eq!(new_at("vendor/x", r#""version":"1.0.0","_resolved":"https://evil.example/x.tgz""#), ["vendor/x"]);
    }

    #[test]
    fn a_dependency_whose_name_starts_with_an_underscore_does_not_warn() {
        assert_eq!(new_package(&format!(r#"{},"dependencies":{{"_lodash":"1.0.0"}}"#, public("new", "1.0.0"))), [""; 0]);
    }

    #[test]
    fn an_integrity_that_is_no_hash_warns() {
        let (bad_letters, half_good) = (HASH.replace('A', "!"), format!("{HASH} not-a-hash"));
        let odd =
            ["not-a-hash", "md5-abcd", "sha512-", "sha1-", " ", "sha512-AAAA==", "sha512-AAAA== sha1-AAAA=", &bad_letters, &half_good];
        for integrity in odd {
            let url = format!("{PUBLIC}/new/-/new-1.0.0.tgz");
            assert_eq!(
                new_package(&format!(r#""version":"1.0.0","resolved":"{url}","integrity":"{integrity}""#)),
                ["new"],
                "{integrity:?}"
            );
        }
    }

    const SHA1: &str = "sha1-AAAAAAAAAAAAAAAAAAAAAAAAAAA=";

    #[test]
    fn a_well_formed_sha1_does_not_warn_as_old_packages_only_have_that() {
        let url = format!("{PUBLIC}/new/-/new-1.0.0.tgz");
        assert_eq!(new_package(&format!(r#""version":"1.0.0","resolved":"{url}","integrity":"{SHA1}""#)), [""; 0]);
    }

    #[test]
    fn two_well_formed_hashes_do_not_warn() {
        let url = format!("{PUBLIC}/new/-/new-1.0.0.tgz");
        assert_eq!(new_package(&format!(r#""version":"1.0.0","resolved":"{url}","integrity":"{SHA1} {HASH}""#)), [""; 0]);
    }

    /// Top at `top` and nested at `nested`, each of 1.0.0 with `odd` as its integrity and 2.0.0 well formed.
    fn traded_versions(top: &str, nested: &str, odd_base: &str, odd_head: &str) -> [String; 2] {
        let copy = |version: &str, odd: &str| {
            let url = format!("{PUBLIC}/a/-/a-{version}.tgz");
            let integrity = if version == "1.0.0" { odd } else { HASH };
            format!(r#""version":"{version}","resolved":"{url}","integrity":"{integrity}""#)
        };
        let tree = |one: &str, two: &str, odd: &str| {
            lock(&format!("{},{}", entry("node_modules/a", &copy(one, odd)), entry("node_modules/b/node_modules/a", &copy(two, odd))))
        };
        [tree(top, nested, odd_base), tree(nested, top, odd_head)]
    }

    #[test]
    fn an_odd_integrity_the_base_holds_for_that_version_does_not_warn() {
        let [base, head] = traded_versions("2.0.0", "1.0.0", "md5-abcd", "md5-abcd");
        assert_eq!(warned_names(PATH, &base, &head), [""; 0]);
    }

    #[test]
    fn an_odd_integrity_the_base_does_not_hold_for_that_version_warns() {
        let [base, head] = traded_versions("2.0.0", "1.0.0", "md5-abcd", "md5-dcba");
        assert!(warned_names(PATH, &base, &head).contains(&"a".to_owned()));
    }

    #[test]
    fn a_key_that_is_no_plain_install_path_is_refused() {
        assert!(parse(include_str!("fixtures/npm_cases/n_top_hijack.head.json")).is_err());
        for key in [
            "./node_modules/a",
            "node_modules//a",
            "node_modules/a/",
            "node_modules/./a",
            "x/../node_modules/a",
            "NODE_MODULES/a",
            r"node_modules\\a",
            ".",
        ] {
            assert!(parse(&lock(&entry(key, GOOD))).is_err(), "{key}");
        }
    }

    #[test]
    fn a_path_above_the_project_reads_as_npm_writes_it() {
        assert_eq!(parse(&lock(r#""../lib":{"version":"1.0.0"},"../lib/node_modules/a":{GOOD}"#)).unwrap().len(), 1);
    }
}
