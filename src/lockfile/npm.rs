//! `package-lock.json`, the `packages` map of lockfile versions 2 and 3.
use super::{Origin, Package, PackageName, Version};
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
    let package = |version| Package { name: PackageName(name.to_owned()), version: Version(version), origin };
    Some(entry.version.map(package).with_context(|| format!("{path} has no version")))
}

#[cfg(test)]
mod tests {
    use super::super::{ChangeKind, compare};
    use super::*;

    fn named<'a>(packages: &'a [Package], name: &str) -> Vec<&'a Package> {
        packages.iter().filter(|p| p.name.0 == name).collect()
    }

    #[test]
    fn nested_copies_read_under_their_package_name_and_the_project_is_left_out() {
        let packages = parse(include_str!("fixtures/package_base.json")).unwrap();
        assert_eq!(named(&packages, "ms").len(), 2, "the top copy and the one nested under debug");
        assert_eq!(named(&packages, "@acme/left-pad")[0].origin.checksum.as_deref(), Some("sha512-aaaa"));
        assert!(named(&packages, "widgets").is_empty() && named(&packages, "tool").is_empty());
    }

    #[test]
    fn copies_moving_in_the_tree_show_no_change_and_a_changed_integrity_warns() {
        let base = parse(include_str!("fixtures/package_base.json")).unwrap();
        let head = parse(include_str!("fixtures/package_head.json")).unwrap();
        let changes = compare(&base, &head).changes;
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert_eq!(changes[0].name.0, "@acme/left-pad");
        assert!(matches!(changes[0].kind, ChangeKind::Warning { .. }));
    }

    #[test]
    fn version_2_reads_its_packages_map_too() {
        let text = r#"{"lockfileVersion": 2, "packages": {"node_modules/a": {"version": "1.0.0"}}, "dependencies": {}}"#;
        assert_eq!(parse(text).unwrap().len(), 1);
    }

    #[test]
    fn a_format_not_read_whole_is_refused() {
        assert!(parse(r#"{"lockfileVersion": 1, "dependencies": {}}"#).is_err());
        assert!(parse(r#"{"lockfileVersion": 3}"#).is_err());
        assert!(parse(r#"{"lockfileVersion": 3, "packages": {"node_modules/a": {}}}"#).is_err());
    }
}
