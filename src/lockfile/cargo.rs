//! `Cargo.lock`, the `[[package]]` tables of its format up to version 4.
use super::{Origin, Package};
use anyhow::{Context as _, Result, ensure};
use serde::Deserialize;

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
    let package = |entry: Entry| Package::new(entry.name, entry.version, Origin { checksum: entry.checksum, source: entry.source });
    Ok(lock.package.into_iter().map(package).collect())
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

    const REGISTRY: &str = "registry+https://github.com/rust-lang/crates.io-index";

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

    #[test]
    fn a_format_not_read_whole_is_refused() {
        assert!(parse("version = 5\n").is_err());
        assert!(parse("[metadata]\n\"checksum a 1.0.0 (registry)\" = \"aaaa\"\n").is_err());
    }
}
