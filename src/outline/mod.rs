//! The outline: which functions, methods and classes an MR changes. Each side of a file is read with
//! its grammar's own tags query, then the symbols of both sides are compared by kind and name.
use crate::syntax;
use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::OnceLock;
use tree_sitter::Parser;
use tree_sitter_tags::{TagsConfiguration, TagsContext};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Kind {
    Class,
    Function,
    Method,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Symbol {
    pub(crate) kind: Kind,
    /// Led by the classes around it: `Cart.total`.
    pub(crate) name: String,
    /// Its whole definition.
    text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    Removed,
    Added,
    Changed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Change {
    /// The file's path at head.
    pub(crate) path: String,
    pub(crate) state: State,
    /// The symbol at head; at base when removed.
    pub(crate) symbol: Symbol,
}

/// What the outline read in the MR: the changed symbols, by file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Reading {
    pub(crate) changes: Vec<Change>,
}

/// Reads each file, given as its path with its text at base and at head, empty on a side it does not exist.
pub(crate) fn read(files: &[(String, String, String)]) -> Reading {
    Reading { changes: files.iter().flat_map(|(path, base, head)| file(path, base, head)).collect() }
}

/// One syntax language with a tags query, named as in [`syntax::LANGUAGES`], which says which files it reads.
struct Tags {
    language: &'static str,
    grammar: fn() -> tree_sitter::Language,
    queries: &'static [&'static str],
    loaded: OnceLock<Option<TagsConfiguration>>,
}

impl Tags {
    const fn new(language: &'static str, grammar: fn() -> tree_sitter::Language, queries: &'static [&'static str]) -> Self {
        Self { language, grammar, queries, loaded: OnceLock::new() }
    }

    fn configuration(&self) -> Option<&TagsConfiguration> {
        self.loaded.get_or_init(|| TagsConfiguration::new((self.grammar)(), &self.queries.join("\n"), "").ok()).as_ref()
    }
}

/// TypeScript's own query only adds what JavaScript lacks: signatures, abstract classes.
const TYPESCRIPT_QUERIES: &[&str] = &[tree_sitter_javascript::TAGS_QUERY, tree_sitter_typescript::TAGS_QUERY];

static TAGS: [Tags; 4] = [
    Tags::new("TypeScript", || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(), TYPESCRIPT_QUERIES),
    Tags::new("TSX", || tree_sitter_typescript::LANGUAGE_TSX.into(), TYPESCRIPT_QUERIES),
    Tags::new("JavaScript", || tree_sitter_javascript::LANGUAGE.into(), &[tree_sitter_javascript::TAGS_QUERY]),
    Tags::new("Python", || tree_sitter_python::LANGUAGE.into(), &[tree_sitter_python::TAGS_QUERY]),
];

fn tags_for(path: &str) -> Option<&'static Tags> {
    let language = syntax::language_for(path, &syntax::LANGUAGES)?;
    TAGS.iter().find(|tags| tags.language == language.name)
}

/// Whether the outline can read this file.
pub(crate) fn readable(path: &str) -> bool {
    tags_for(path).is_some()
}

/// The symbols that changed between the two sides of `path`.
fn file(path: &str, base: &str, head: &str) -> Vec<Change> {
    let Some(tags) = tags_for(path) else { return vec![] };
    compare(path, symbols(tags, base), symbols(tags, head))
}

/// Every function, method and class of `source`.
fn symbols(tags: &Tags, source: &str) -> Vec<Symbol> {
    let Some(config) = tags.configuration() else { return vec![] };
    let mut parser = Parser::new();
    let Some(tree) = parser.set_language(&config.language).ok().and_then(|()| parser.parse(source, None)) else { return vec![] };
    let mut context = TagsContext::new();
    let Ok((found, _)) = context.generate_tags(config, source.as_bytes(), None) else { return vec![] };
    let node_at = |range: &Range<usize>| tree.root_node().descendant_for_byte_range(range.start, range.end);
    let mut definitions: Vec<(Kind, Range<usize>, Range<usize>)> = vec![];
    for tag in found.filter_map(Result::ok).filter(|tag| tag.is_definition) {
        if let Some(kind) = kind_named(config.syntax_type_name(tag.syntax_type_id))
            && node_at(&tag.range).is_some_and(|node| node.kind() != "pair")
        {
            definitions.push((kind, tag.range, tag.name_range));
        }
    }
    let inside = |inner: &Range<usize>, outer: &Range<usize>| outer.start <= inner.start && inner.end <= outer.end;
    definitions
        .iter()
        .map(|(kind, range, name)| {
            let classes: Vec<&str> = definitions
                .iter()
                .filter(|(k, r, _)| *k == Kind::Class && r != range && inside(range, r))
                .map(|(_, _, n)| &source[n.clone()])
                .collect();
            let kind = if *kind == Kind::Function && !classes.is_empty() { Kind::Method } else { *kind };
            let name = classes.iter().copied().chain([&source[name.clone()]]).collect::<Vec<_>>().join(".");
            Symbol { kind, name, text: source[range.clone()].to_owned() }
        })
        .collect()
}

fn kind_named(name: &str) -> Option<Kind> {
    match name {
        "class" => Some(Kind::Class),
        "function" => Some(Kind::Function),
        "method" => Some(Kind::Method),
        _ => None,
    }
}

/// Symbols matched by kind and name. A class lists only when it comes or goes: its methods carry its body.
fn compare(path: &str, base: Vec<Symbol>, head: Vec<Symbol>) -> Vec<Change> {
    let keyed =
        |symbols: Vec<Symbol>| -> BTreeMap<(Kind, String), Symbol> { symbols.into_iter().map(|s| ((s.kind, s.name.clone()), s)).collect() };
    let (mut old, new) = (keyed(base), keyed(head));
    let change = |state, symbol: Symbol| Change { path: path.to_owned(), state, symbol };
    let mut changes = vec![];
    for (key, after) in new {
        match old.remove(&key) {
            None => changes.push(change(State::Added, after)),
            Some(before) if before.text != after.text && after.kind != Kind::Class => changes.push(change(State::Changed, after)),
            Some(_) => {}
        }
    }
    changes.extend(old.into_values().map(|before| change(State::Removed, before)));
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(path: &str, source: &str) -> Vec<(Kind, String)> {
        symbols(tags_for(path).unwrap(), source).into_iter().map(|s| (s.kind, s.name)).collect()
    }

    fn states(path: &str, base: &str, head: &str) -> Vec<(State, String)> {
        file(path, base, head).into_iter().map(|c| (c.state, c.symbol.name)).collect()
    }

    #[test]
    fn python_methods_are_named_after_their_class() {
        let source = "class Cart:\n    def total(self):\n        return 1\n\ndef checkout(cart):\n    return cart.pay()\n";
        assert_eq!(
            found("shop/cart.py", source),
            vec![(Kind::Class, "Cart".into()), (Kind::Method, "Cart.total".into()), (Kind::Function, "checkout".into())]
        );
    }

    #[test]
    fn typescript_reads_javascript_definitions_too() {
        let source = "export function charge(a: number) {\n  return a;\n}\nexport class Wallet {\n  refund(id: string) { return id; }\n}\n";
        assert_eq!(
            found("src/pay.ts", source),
            vec![(Kind::Function, "charge".into()), (Kind::Class, "Wallet".into()), (Kind::Method, "Wallet.refund".into())]
        );
    }

    #[test]
    fn tsx_and_javascript_read_too_and_other_files_not() {
        assert_eq!(found("src/Badge.tsx", "export function Badge() { return <b/>; }").len(), 1);
        assert_eq!(found("src/badge.js", "function badge() {}").len(), 1);
        assert!(!readable("package.json") && !readable("README.md") && !readable("db/schema.sql"));
        assert_eq!(file("package.json", "{}", "{\"a\": 1}"), vec![]);
    }

    #[test]
    fn each_state_is_found_and_unchanged_symbols_are_left_out() {
        let base = "def kept(a):\n    return a\n\ndef body(a):\n    return a\n\ndef gone(a):\n    return a\n";
        let head = "def kept(a):\n    return a\n\ndef body(a):\n    return a + 1\n\ndef fresh():\n    pass\n";
        assert_eq!(
            states("m.py", base, head),
            vec![(State::Changed, "body".into()), (State::Added, "fresh".into()), (State::Removed, "gone".into())]
        );
    }

    #[test]
    fn an_object_literal_property_holding_a_function_is_not_a_symbol() {
        let hook = "export function useWidgets(id: string) {\n  const { data, reload } = useQuery(id);\n  return { reload: () => void reload(), data };\n}\n";
        assert_eq!(found("src/hooks.ts", hook), vec![(Kind::Function, "useWidgets".into())]);
    }
}
