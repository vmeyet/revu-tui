//! `Cargo.lock`, the `[[package]]` tables of its format up to version 4.
use super::{Origin, Package, PackageName, Site, Version};
use anyhow::{Context as _, Result, ensure};
use serde::Deserialize;

pub(super) const PUBLIC_REGISTRY: &str = "registry+https://github.com/rust-lang/crates.io-index";

#[derive(Deserialize)]
struct Lock {
    version: Option<u32>,
    #[serde(default)]
    package: Vec<Entry>,
    #[serde(default)]
    metadata: toml::Table,
}

#[derive(Deserialize)]
struct Entry {
    name: String,
    version: String,
    source: Option<String>,
    checksum: Option<String>,
    replace: Option<String>,
}

pub(super) fn parse(text: &str) -> Result<Vec<Package>> {
    let lock: Lock = toml::from_str(text).context("Cargo.lock does not parse")?;
    ensure!(matches!(lock.version, None | Some(3 | 4)), "Cargo.lock version {:?} is not supported", lock.version);
    // The oldest format keeps checksums here: read without them, a changed checksum would go unseen.
    ensure!(!lock.metadata.keys().any(|key| key.starts_with("checksum ")), "this Cargo.lock format keeps its checksums apart");
    // A replaced package loads other code than its row says, and only the raw diff shows the whole of it.
    ensure!(lock.package.iter().all(|entry| entry.replace.is_none()), "this Cargo.lock replaces a package");
    Ok(lock.package.into_iter().map(package_of).collect())
}

fn package_of(entry: Entry) -> Package {
    let site = site_of(entry.source.as_deref(), entry.checksum.as_deref());
    Package {
        name: PackageName(entry.name),
        version: Version(entry.version),
        origin: Origin { checksum: entry.checksum, source: entry.source },
        site,
        install_script: false,
        at_top: false,
    }
}

fn site_of(source: Option<&str>, checksum: Option<&str>) -> Site {
    let Some(source) = source else {
        return if checksum.is_none() { Site::Project } else { Site::Unproven { shown: "a checksum with no source".to_owned() } };
    };
    if source.starts_with("registry+") || source.starts_with("sparse+") {
        return Site::Registry { base: source.to_owned() };
    }
    Site::git(source).unwrap_or_else(|| Site::Unproven { shown: source.to_owned() })
}

#[cfg(test)]
mod tests {
    use super::super::warned_names;
    use super::*;

    #[test]
    fn a_lock_reads_every_package_with_its_checksum_and_source() {
        let packages = parse(include_str!("fixtures/cargo_base.lock")).unwrap();
        let tokio = packages.iter().find(|p| p.name.0 == "tokio").unwrap();
        assert_eq!((tokio.version.0.as_str(), tokio.origin.checksum.as_deref()), ("1.40.0", Some("bbbb")));
        assert!(tokio.origin.source.as_deref().is_some_and(|s| s.starts_with("registry+")));
        assert_eq!(packages.iter().filter(|p| p.name.0 == "rand_core").count(), 2, "two versions of one name");
    }

    const REGISTRY: &str = PUBLIC_REGISTRY;

    fn lock(entries: &[&str]) -> String {
        format!("version = 4\n{}", entries.concat())
    }

    fn entry(name: &str, version: &str, source: &str, checksum: &str) -> String {
        format!("[[package]]\nname = \"{name}\"\nversion = \"{version}\"\nsource = \"{source}\"\nchecksum = \"{checksum}\"\n")
    }

    #[test]
    fn a_bump_onto_git_without_a_checksum_warns() {
        let base = lock(&[&entry("serde", "1.0.0", REGISTRY, "aaaa")]);
        let git = "[[package]]\nname = \"serde\"\nversion = \"1.0.1\"\nsource = \"git+https://evil.example/serde#abc\"\n";
        assert_eq!(warned_names("Cargo.lock", &base, &lock(&[git])), ["serde"]);
    }

    #[test]
    fn a_bump_that_loses_its_checksum_warns() {
        let base = lock(&[&entry("serde", "1.0.0", REGISTRY, "aaaa")]);
        let bare = format!("[[package]]\nname = \"serde\"\nversion = \"1.0.1\"\nsource = \"{REGISTRY}\"\n");
        assert_eq!(warned_names("Cargo.lock", &base, &lock(&[&bare])), ["serde"]);
    }

    #[test]
    fn a_bump_inside_one_registry_does_not_warn() {
        let (base, head) = (entry("serde", "1.0.0", REGISTRY, "aaaa"), entry("serde", "1.0.1", REGISTRY, "bbbb"));
        assert_eq!(warned_names("Cargo.lock", &lock(&[&base]), &lock(&[&head])), [""; 0]);
    }

    #[test]
    fn a_new_package_from_git_warns() {
        let git = "[[package]]\nname = \"tool\"\nversion = \"1.0.0\"\nsource = \"git+https://example.com/tool#abc\"\n";
        assert_eq!(warned_names("Cargo.lock", &lock(&[]), &lock(&[git])), ["tool"]);
    }

    #[test]
    fn a_new_workspace_member_does_not_warn() {
        let member = "[[package]]\nname = \"tool\"\nversion = \"1.0.0\"\n";
        assert_eq!(warned_names("Cargo.lock", &lock(&[]), &lock(&[member])), [""; 0]);
    }

    #[test]
    fn a_lock_with_a_replace_is_refused() {
        let replaced = "[[package]]\nname = \"a\"\nversion = \"1.0.0\"\nreplace = \"evil 1.0.0 (git+https://evil.example#1)\"\n";
        assert!(parse(&lock(&[replaced])).is_err());
    }

    fn new_beside_serde(entry_text: &str) -> Vec<String> {
        let serde = entry("serde", "1.0.0", REGISTRY, "aaaa");
        warned_names("Cargo.lock", &lock(&[&serde]), &lock(&[&serde, entry_text]))
    }

    #[test]
    fn a_new_package_from_crates_io_does_not_warn() {
        assert_eq!(new_beside_serde(&entry("tool", "1.0.0", REGISTRY, "bbbb")), [""; 0]);
    }

    #[test]
    fn a_new_package_from_a_registry_the_base_does_not_use_warns() {
        assert_eq!(new_beside_serde(&entry("tool", "1.0.0", "registry+https://evil.example/index", "bbbb")), ["tool"]);
    }

    #[test]
    fn a_new_package_from_a_near_copy_of_the_crates_io_string_warns() {
        assert_eq!(new_beside_serde(&entry("tool", "1.0.0", &format!("{REGISTRY}/"), "bbbb")), ["tool"]);
    }

    #[test]
    fn a_new_package_from_the_exact_private_registry_the_base_uses_does_not_warn() {
        let private = "sparse+https://crates.acme.example/index/";
        let old = entry("one", "1.0.0", private, "aaaa");
        assert_eq!(warned_names("Cargo.lock", &lock(&[&old]), &lock(&[&old, &entry("two", "1.0.0", private, "bbbb")])), [""; 0]);
    }

    #[test]
    fn a_new_package_with_no_checksum_warns() {
        assert_eq!(new_beside_serde(&format!("[[package]]\nname = \"tool\"\nversion = \"1.0.0\"\nsource = \"{REGISTRY}\"\n")), ["tool"]);
    }

    #[test]
    fn a_checksum_with_no_source_warns() {
        assert_eq!(new_beside_serde("[[package]]\nname = \"tool\"\nversion = \"1.0.0\"\nchecksum = \"bbbb\"\n"), ["tool"]);
    }

    #[test]
    fn a_source_of_no_known_form_warns() {
        assert_eq!(new_beside_serde(&entry("tool", "1.0.0", "directory+/tmp/evil", "bbbb")), ["tool"]);
    }

    #[test]
    fn a_version_that_is_not_semver_warns() {
        assert_eq!(new_beside_serde(&entry("tool", "v1.0.0", REGISTRY, "bbbb")), ["tool"]);
    }

    #[test]
    fn an_added_lock_from_another_registry_warns() {
        let head = lock(&[&entry("serde", "1.0.0", "registry+https://evil.example/index", "bbbb")]);
        let summary = crate::lockfile::summarize("Cargo.lock", None, Some(&head)).unwrap();
        assert!(summary.changes.iter().all(|c| c.kind.warns()) && summary.changes.len() == 1, "{summary:?}");
    }

    fn git(version: &str, repo: &str, commit: &str) -> String {
        format!("[[package]]\nname = \"s\"\nversion = \"{version}\"\nsource = \"git+{repo}#{commit}\"\n")
    }

    #[test]
    fn a_git_bump_inside_one_repository_does_not_warn() {
        let (base, head) = (git("1.0.0", "https://github.com/acme/s", "aaa"), git("1.1.0", "https://github.com/acme/s", "bbb"));
        assert_eq!(warned_names("Cargo.lock", &lock(&[&base]), &lock(&[&head])), [""; 0]);
    }

    #[test]
    fn a_git_bump_to_a_fork_on_the_same_host_warns() {
        let (base, head) = (git("1.0.0", "https://github.com/acme/s", "aaa"), git("1.1.0", "https://github.com/evil/s", "bbb"));
        assert_eq!(warned_names("Cargo.lock", &lock(&[&base]), &lock(&[&head])), ["s"]);
    }

    #[test]
    fn a_new_package_from_a_repository_the_base_uses_does_not_warn() {
        let old = git("1.0.0", "https://github.com/acme/s", "aaa");
        let new = "[[package]]\nname = \"s-macros\"\nversion = \"1.0.0\"\nsource = \"git+https://github.com/acme/s#aaa\"\n";
        assert_eq!(warned_names("Cargo.lock", &lock(&[&old]), &lock(&[&old, new])), [""; 0]);
    }

    #[test]
    fn a_bump_onto_the_repository_of_another_package_warns() {
        let other = git("1.0.0", "https://github.com/acme/other", "aaa").replace("\"s\"", "\"other\"");
        let (base, head) = (entry("s", "1.0.0", REGISTRY, "aaaa"), git("1.0.1", "https://github.com/acme/other", "aaa"));
        assert_eq!(warned_names("Cargo.lock", &lock(&[&other, &base]), &lock(&[&other, &head])), ["s"]);
    }

    #[test]
    fn a_format_not_read_whole_is_refused() {
        assert!(parse("version = 5\n").is_err());
        assert!(parse("[metadata]\n\"checksum a 1.0.0 (registry)\" = \"aaaa\"\n").is_err());
    }
}
