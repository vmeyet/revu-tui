//! The outline: which functions of an MR's Python files change. Each side of a file is read with
//! its grammar's own tags query, then the functions of both sides are compared by name.
use std::collections::{BTreeMap, BTreeSet};
use tree_sitter_tags::{TagsConfiguration, TagsContext};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Change {
    /// The file's path at head.
    pub(crate) path: String,
    pub(crate) name: String,
}

/// What the outline read in the MR: the changed functions, by file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Reading {
    pub(crate) changes: Vec<Change>,
}

/// Reads each file, given as its path with its text at base and at head, empty on a side it does not exist.
pub(crate) fn read(files: &[(String, String, String)]) -> Reading {
    Reading { changes: files.iter().flat_map(|(path, base, head)| file(path, base, head)).collect() }
}

/// Whether the outline can read this file.
pub(crate) fn readable(path: &str) -> bool {
    std::path::Path::new(path).extension().is_some_and(|extension| extension.eq_ignore_ascii_case("py"))
}

/// The functions that differ between the two sides of `path`, by name.
fn file(path: &str, base: &str, head: &str) -> Vec<Change> {
    if !readable(path) {
        return vec![];
    }
    let (old, new) = (functions(base), functions(head));
    let names: BTreeSet<&String> = old.keys().chain(new.keys()).collect();
    names
        .into_iter()
        .filter(|name| old.get(*name) != new.get(*name))
        .map(|name| Change { path: path.to_owned(), name: name.clone() })
        .collect()
}

/// Every function of `source`, by name, with its whole definition.
fn functions(source: &str) -> BTreeMap<String, String> {
    let Ok(config) = TagsConfiguration::new(tree_sitter_python::LANGUAGE.into(), tree_sitter_python::TAGS_QUERY, "") else {
        return BTreeMap::new();
    };
    let mut context = TagsContext::new();
    let Ok((found, _)) = context.generate_tags(&config, source.as_bytes(), None) else { return BTreeMap::new() };
    found
        .filter_map(Result::ok)
        .filter(|tag| tag.is_definition && config.syntax_type_name(tag.syntax_type_id) == "function")
        .map(|tag| (source[tag.name_range].to_owned(), source[tag.range].to_owned()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_functions_that_differ_are_listed() {
        let base = "def kept(a):\n    return a\n\ndef body(a):\n    return a\n\ndef gone(a):\n    return a\n";
        let head = "def kept(a):\n    return a\n\ndef body(a):\n    return a + 1\n\ndef fresh():\n    pass\n";
        let names: Vec<String> = file("m.py", base, head).into_iter().map(|change| change.name).collect();
        assert_eq!(names, ["body", "fresh", "gone"]);
    }

    #[test]
    fn other_files_are_not_read() {
        assert!(!readable("package.json") && !readable("src/pay.ts"));
        assert_eq!(file("package.json", "{}", "{\"a\": 1}"), vec![]);
    }
}
