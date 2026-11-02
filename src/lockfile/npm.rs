//! `package-lock.json`, the `packages` map of lockfile versions 2 and 3.
use super::{Origin, Package};
use anyhow::{Context as _, Result, ensure};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Lock {
    #[serde(rename = "lockfileVersion")]
    version: u32,
    packages: Option<BTreeMap<String, Entry>>,
}

#[derive(Deserialize)]
struct Entry {
    version: Option<String>,
    resolved: Option<String>,
    integrity: Option<String>,
    #[serde(default)]
    link: bool,
}

pub(super) fn parse(text: &str) -> Result<Vec<Package>> {
    let lock: Lock = serde_json::from_str(text).context("package-lock.json does not parse")?;
    ensure!(matches!(lock.version, 2 | 3), "package-lock.json version {} is not supported", lock.version);
    let packages = lock.packages.context("package-lock.json has no packages")?;
    packages.into_iter().filter_map(|(path, entry)| package_at(&path, entry)).collect()
}

/// The package installed at `path`; the project, its workspace folders and links to them install nothing.
fn package_at(path: &str, entry: Entry) -> Option<Result<Package>> {
    if entry.link {
        return None;
    }
    let (_, name) = path.rsplit_once("node_modules/")?;
    let origin = Origin { checksum: entry.integrity, source: entry.resolved };
    let package = |version| Package::new(name.to_owned(), version, origin);
    Some(entry.version.map(package).with_context(|| format!("{path} has no version")))
}

#[cfg(test)]
mod tests {
    use super::super::{ChangeKind, compare, warned_names};
    use super::*;

    fn named<'a>(packages: &'a [Package], name: &str) -> Vec<&'a Package> {
        packages.iter().filter(|p| p.name.0 == name).collect()
    }

    #[test]
    fn nested_copies_read_under_their_package_name_and_the_project_is_left_out() {
        let packages = parse(include_str!("fixtures/package_base.json")).unwrap();
        assert_eq!(named(&packages, "ms").len(), 2, "the top copy and the one nested under debug");
        assert_eq!(named(&packages, "@acme/left-pad")[0].origin.checksum.as_deref(), Some("sha512-aaaa"));
        assert_eq!((named(&packages, "widgets").len(), named(&packages, "tool").len()), (0, 0));
    }

    #[test]
    fn copies_moving_in_the_tree_show_no_change_and_a_changed_integrity_warns() {
        let base = parse(include_str!("fixtures/package_base.json")).unwrap();
        let head = parse(include_str!("fixtures/package_head.json")).unwrap();
        let changes = compare(&base, &head).changes;
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
    fn a_new_package_on_a_known_registry_does_not_warn() {
        let head =
            lock(r#""node_modules/a":{GOOD},"node_modules/b":{"version":"1.0.0","resolved":"https://registry.npmjs.org/b/-/b-1.0.0.tgz"}"#);
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
    fn a_format_not_read_whole_is_refused() {
        assert!(parse(r#"{"lockfileVersion": 1, "dependencies": {}}"#).is_err());
        assert!(parse(r#"{"lockfileVersion": 3}"#).is_err());
        assert!(parse(r#"{"lockfileVersion": 3, "packages": {"node_modules/a": {}}}"#).is_err());
    }
}
