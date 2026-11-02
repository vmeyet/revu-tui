//! `Cargo.lock`, the `[[package]]` tables of its format up to version 4.
use super::{Origin, Package, PackageName, Version};
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
}

pub(super) fn parse(text: &str) -> Result<Vec<Package>> {
    let lock: Lock = toml::from_str(text).context("Cargo.lock does not parse")?;
    ensure!(matches!(lock.version, None | Some(3 | 4)), "Cargo.lock version {:?} is not supported", lock.version);
    // The oldest format keeps checksums here: read without them, a changed checksum would go unseen.
    ensure!(!lock.metadata.keys().any(|key| key.starts_with("checksum ")), "this Cargo.lock format keeps its checksums apart");
    let package = |entry: Entry| Package {
        name: PackageName(entry.name),
        version: Version(entry.version),
        origin: Origin { checksum: entry.checksum, source: entry.source },
    };
    Ok(lock.package.into_iter().map(package).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lock_reads_every_package_with_its_checksum_and_source() {
        let packages = parse(include_str!("fixtures/cargo_base.lock")).unwrap();
        let tokio = packages.iter().find(|p| p.name.0 == "tokio").unwrap();
        assert_eq!((tokio.version.0.as_str(), tokio.origin.checksum.as_deref()), ("1.40.0", Some("bbbb")));
        assert!(tokio.origin.source.as_deref().is_some_and(|s| s.starts_with("registry+")));
        assert_eq!(packages.iter().filter(|p| p.name.0 == "rand_core").count(), 2, "two versions of one name");
    }

    #[test]
    fn a_format_not_read_whole_is_refused() {
        assert!(parse("version = 5\n").is_err());
        assert!(parse("[metadata]\n\"checksum a 1.0.0 (registry)\" = \"aaaa\"\n").is_err());
    }
}
