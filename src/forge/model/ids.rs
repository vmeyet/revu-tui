//! The names an MR, its project, a commit and a thread go by.
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Display, serde as a plain JSON string, and a test-only `From<&str>` for a text newtype whose
/// `parse` keeps its invariant: reading JSON goes through `parse` too.
macro_rules! parsed_text {
    ($name:ident, $what:literal) => {
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = String::deserialize(deserializer)?;
                $name::parse(&text).ok_or_else(|| serde::de::Error::custom(format!("`{text}` is not {}", $what)))
            }
        }

        #[cfg(test)]
        impl From<&str> for $name {
            fn from(text: &str) -> Self {
                $name::parse(text).unwrap_or_else(|| panic!("`{text}` is not {}", $what))
            }
        }
    };
}

/// The one way an MR is addressed: the path of its project (`group/sub/project`, `owner/repo`)
/// and its number there.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct MrKey {
    /// The host it lives on, when that is not the one `revu` started with: a queue can hold several.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) host: Option<String>,
    pub(crate) project: ProjectPath,
    pub(crate) number: u64,
}

impl MrKey {
    pub(crate) fn new(project: impl Into<ProjectPath>, number: u64) -> Self {
        Self { host: None, project: project.into(), number }
    }
}

/// A project's path on its forge: `group/sub/project` on GitLab, `owner/repo` on GitHub.
/// Two or more `/`-separated names of ASCII letters, digits, `-`, `_` and `.`, none of them `.`
/// or `..`, so it can name a cache folder. Stored as a plain JSON string.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ProjectPath(Arc<str>);

impl ProjectPath {
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let name = |part: &str| {
            !matches!(part, "" | "." | "..") && part.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        };
        (text.contains('/') && text.split('/').all(name)).then(|| Self(text.into()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

parsed_text!(ProjectPath, "a project path");

/// A commit id: 1 to 64 hex digits, never anything else, so it can name a cache file.
/// Stored as a plain JSON string.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Sha(Arc<str>);

impl Sha {
    /// SHA-256 object names are 64 digits; SHA-1 ones 40.
    const MAX_LEN: usize = 64;

    pub(crate) fn parse(text: &str) -> Option<Self> {
        let valid = (1..=Self::MAX_LEN).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_hexdigit());
        valid.then(|| Self(text.into()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

parsed_text!(Sha, "a commit sha");

/// A thread's id as its forge names it: GitLab's discussion id, GitHub's review thread node id.
/// Any text the forge sent; stored as a plain JSON string.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct ThreadId(Arc<str>);

impl ThreadId {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ThreadId {
    fn from(id: &str) -> Self {
        Self(id.into())
    }
}

impl From<String> for ThreadId {
    fn from(id: String) -> Self {
        Self(id.into())
    }
}

impl std::fmt::Display for ThreadId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::Refs;

    #[test]
    fn a_sha_holds_hex_digits_only_so_no_cache_path_comes_from_anything_else() {
        assert_eq!(Sha::parse("0123abcDEF").map(|sha| sha.to_string()).as_deref(), Some("0123abcDEF"));
        for bad in ["", "../etc", "ab/cd", "beef ", "g", &"a".repeat(65)] {
            assert_eq!(Sha::parse(bad), None, "{bad:?}");
        }
        assert!(serde_json::from_str::<Refs>(r#"{"base": "aaaa", "start": "aaaa", "head": "../../x"}"#).is_err());
    }

    #[test]
    fn a_project_path_holds_plain_names_only_so_no_cache_path_climbs_out() {
        for good in ["acme/widgets", "acme/sub.group/My_Widgets-2"] {
            assert_eq!(ProjectPath::parse(good).map(|path| path.to_string()).as_deref(), Some(good));
        }
        for bad in [
            "",
            "solo",
            "acme/",
            "/acme/widgets",
            "acme//widgets",
            "acme/..",
            "./widgets",
            "acme/wid gets",
            "acme+widgets",
            "acme\\widgets",
        ] {
            assert_eq!(ProjectPath::parse(bad), None, "{bad:?}");
        }
        assert!(serde_json::from_str::<MrKey>(r#"{"project": "../../etc", "number": 1}"#).is_err());
    }
}
