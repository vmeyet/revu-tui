//! The outline: which functions, methods and classes an MR changes. Each side of a file is read with
//! its grammar's own tags query, then the symbols of both sides are compared by kind and name.
use crate::syntax;
use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::OnceLock;
use tree_sitter::{Node, Parser};
use tree_sitter_tags::{TagsConfiguration, TagsContext};

/// The share of body words a removed and an added symbol must have in common to read as a rename.
const RENAME_SIMILARITY: f32 = 0.8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Class,
    Function,
    Method,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub kind: Kind,
    /// Led by the classes around it: `Cart.total`.
    pub name: String,
    /// The definition up to its body, whitespace collapsed.
    pub signature: String,
    pub public: bool,
    /// Its first and last line, from 1.
    pub lines: (u32, u32),
    body: String,
}

/// How a symbol changed, riskiest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum State {
    Removed,
    Signature,
    Added,
    Renamed,
    Body,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    /// The file's path at head.
    pub path: String,
    pub state: State,
    /// The symbol at head; at base when removed.
    pub symbol: Symbol,
    /// The symbol at base, for a changed signature or a rename.
    pub before: Option<Symbol>,
}

impl Change {
    /// A public symbol gone, or called differently now.
    pub fn breaking(&self) -> bool {
        self.symbol.public && matches!(self.state, State::Removed | State::Signature)
    }

    /// Public removals, signatures and additions first; private ones after renames and body changes.
    fn risk(&self) -> (bool, State, u32) {
        let private = !self.symbol.public && self.state <= State::Added;
        (private, self.state, self.symbol.lines.0)
    }
}

/// One file to outline: its path at base, absent when added, and at head, absent when deleted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sides {
    pub base: Option<String>,
    pub head: Option<String>,
}

/// One syntax language with a tags query, named as in [`syntax::LANGUAGES`], which says which files it reads.
struct Tags {
    language: &'static str,
    grammar: fn() -> tree_sitter::Language,
    queries: &'static [&'static str],
    public: fn(Node, &str) -> bool,
    loaded: OnceLock<Option<TagsConfiguration>>,
}

impl Tags {
    const fn new(
        language: &'static str,
        grammar: fn() -> tree_sitter::Language,
        queries: &'static [&'static str],
        public: fn(Node, &str) -> bool,
    ) -> Self {
        Self { language, grammar, queries, public, loaded: OnceLock::new() }
    }

    fn configuration(&self) -> Option<&TagsConfiguration> {
        self.loaded.get_or_init(|| TagsConfiguration::new((self.grammar)(), &self.queries.join("\n"), "").ok()).as_ref()
    }
}

/// TypeScript's own query only adds what JavaScript lacks: signatures, abstract classes.
const TYPESCRIPT_QUERIES: &[&str] = &[tree_sitter_javascript::TAGS_QUERY, tree_sitter_typescript::TAGS_QUERY];

static TAGS: [Tags; 4] = [
    Tags::new("TypeScript", || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(), TYPESCRIPT_QUERIES, exported),
    Tags::new("TSX", || tree_sitter_typescript::LANGUAGE_TSX.into(), TYPESCRIPT_QUERIES, exported),
    Tags::new("JavaScript", || tree_sitter_javascript::LANGUAGE.into(), &[tree_sitter_javascript::TAGS_QUERY], exported),
    Tags::new("Python", || tree_sitter_python::LANGUAGE.into(), &[tree_sitter_python::TAGS_QUERY], unprefixed),
];

/// TypeScript and JavaScript: inside an `export`.
fn exported(node: Node, _name: &str) -> bool {
    std::iter::successors(node.parent(), Node::parent).any(|n| n.kind() == "export_statement")
}

/// Python: a name without a leading `_`.
fn unprefixed(_node: Node, name: &str) -> bool {
    !name.starts_with('_')
}

fn tags_for(path: &str) -> Option<&'static Tags> {
    let language = syntax::language_for(path, &syntax::LANGUAGES)?;
    TAGS.iter().find(|tags| tags.language == language.name)
}

/// Whether the outline can read this file.
pub fn readable(path: &str) -> bool {
    tags_for(path).is_some()
}

/// The symbols that changed between the two sides of `path`, riskiest first; an absent side is empty.
pub fn changes(path: &str, base: &str, head: &str) -> Vec<Change> {
    let Some(tags) = tags_for(path) else { return vec![] };
    let mut changes = compare(path, symbols(tags, base), symbols(tags, head));
    changes.sort_by_key(Change::risk);
    changes
}

/// Every function, method and class of `source`.
fn symbols(tags: &Tags, source: &str) -> Vec<Symbol> {
    let Some(config) = tags.configuration() else { return vec![] };
    let mut parser = Parser::new();
    let Some(tree) = parser.set_language(&config.language).ok().and_then(|()| parser.parse(source, None)) else { return vec![] };
    let mut context = TagsContext::new();
    let Ok((found, _)) = context.generate_tags(config, source.as_bytes(), None) else { return vec![] };
    let definitions: Vec<(Kind, Range<usize>, Range<usize>)> = found
        .filter_map(Result::ok)
        .filter(|tag| tag.is_definition)
        .filter_map(|tag| Some((kind_named(config.syntax_type_name(tag.syntax_type_id))?, tag.range, tag.name_range)))
        .collect();
    definitions
        .iter()
        .filter_map(|(kind, range, name)| {
            let node = tree.root_node().descendant_for_byte_range(range.start, range.end)?;
            let classes: Vec<&str> = definitions
                .iter()
                .filter(|(k, r, _)| *k == Kind::Class && r != range && r.start <= range.start && range.end <= r.end)
                .map(|(_, _, n)| &source[n.clone()])
                .collect();
            let kind = if *kind == Kind::Function && !classes.is_empty() { Kind::Method } else { *kind };
            Some(symbol(node, source, kind, &classes, &source[name.clone()], tags))
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

fn symbol(node: Node, source: &str, kind: Kind, classes: &[&str], name: &str, tags: &Tags) -> Symbol {
    let body = node.child_by_field_name("body").or_else(|| node.child_by_field_name("value")?.child_by_field_name("body"));
    let body_start = body.map_or(node.end_byte(), |b| b.start_byte());
    let line = |row: usize| u32::try_from(row + 1).unwrap_or(u32::MAX);
    Symbol {
        kind,
        name: classes.iter().copied().chain([name]).collect::<Vec<_>>().join("."),
        signature: collapsed(&source[node.start_byte()..body_start]).trim_end_matches(':').trim_end().to_owned(),
        public: (tags.public)(node, name),
        lines: (line(node.start_position().row), line(node.end_position().row)),
        body: collapsed(&source[body_start..node.end_byte()]),
    }
}

fn collapsed(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Symbols matched by kind and name; a removed one and an added one alike enough read as a rename.
/// A class lists only when it comes, goes or changes its signature: its methods carry its body.
fn compare(path: &str, base: Vec<Symbol>, head: Vec<Symbol>) -> Vec<Change> {
    let keyed =
        |symbols: Vec<Symbol>| -> BTreeMap<(Kind, String), Symbol> { symbols.into_iter().map(|s| ((s.kind, s.name.clone()), s)).collect() };
    let (mut old, new) = (keyed(base), keyed(head));
    let change = |state, symbol: Symbol, before: Option<Symbol>| Change { path: path.to_owned(), state, symbol, before };
    let mut changes = vec![];
    let mut added = vec![];
    for (key, after) in new {
        match old.remove(&key) {
            None => added.push(after),
            Some(before) if before.signature != after.signature => changes.push(change(State::Signature, after, Some(before))),
            Some(before) if before.body != after.body && after.kind != Kind::Class => changes.push(change(State::Body, after, None)),
            Some(_) => {}
        }
    }
    let mut removed: Vec<Symbol> = old.into_values().collect();
    removed.sort_by_key(|s| s.lines.0);
    for before in removed {
        match renamed_to(&before, &added) {
            Some(index) => changes.push(change(State::Renamed, added.remove(index), Some(before))),
            None => changes.push(change(State::Removed, before, None)),
        }
    }
    changes.extend(added.into_iter().map(|after| change(State::Added, after, None)));
    changes
}

/// The added symbol most alike `before`, of its kind and scope, when alike enough.
fn renamed_to(before: &Symbol, added: &[Symbol]) -> Option<usize> {
    added
        .iter()
        .enumerate()
        .filter(|(_, after)| after.kind == before.kind && scope(&after.name) == scope(&before.name))
        .map(|(index, after)| (index, similarity(&before.body, &after.body)))
        .filter(|(_, ratio)| *ratio >= RENAME_SIMILARITY)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(index, _)| index)
}

fn scope(name: &str) -> &str {
    name.rsplit_once('.').map_or("", |(scope, _)| scope)
}

/// How much of two bodies' words they share, from 0 to 1; nothing alike when either has none.
fn similarity(old: &str, new: &str) -> f32 {
    let (old, new) = (words(old), words(new));
    if old.is_empty() || new.is_empty() {
        return 0.0;
    }
    similar::TextDiff::from_slices(&old, &new).ratio()
}

fn words(text: &str) -> Vec<&str> {
    text.split(|c: char| !c.is_alphanumeric() && c != '_').filter(|word| !word.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn found(path: &str, source: &str) -> Vec<(Kind, String, String, bool)> {
        symbols(tags_for(path).unwrap(), source).into_iter().map(|s| (s.kind, s.name, s.signature, s.public)).collect()
    }

    fn states(path: &str, base: &str, head: &str) -> Vec<(State, String)> {
        changes(path, base, head).into_iter().map(|c| (c.state, c.symbol.name)).collect()
    }

    const CART: &str = "class Cart:\n    def total(self, items):\n        return sum(i.price for i in items)\n\n    def _round(self, x):\n        return round(x, 2)\n\ndef checkout(cart) -> bool:\n    return cart.pay()\n";

    #[test]
    fn python_methods_are_named_after_their_class_and_a_leading_underscore_is_private() {
        assert_eq!(
            found("shop/cart.py", CART),
            vec![
                (Kind::Class, "Cart".into(), "class Cart".into(), true),
                (Kind::Method, "Cart.total".into(), "def total(self, items)".into(), true),
                (Kind::Method, "Cart._round".into(), "def _round(self, x)".into(), false),
                (Kind::Function, "checkout".into(), "def checkout(cart) -> bool".into(), true),
            ]
        );
    }

    #[test]
    fn typescript_reads_javascript_definitions_and_only_exported_ones_are_public() {
        let source = "export function charge(amount: number): Receipt {\n  return pay(amount);\n}\nfunction helper() {}\nexport class Wallet {\n  refund(id: string) { return id; }\n}\nexport const total = (a: number) => {\n  return a;\n};\n";
        assert_eq!(
            found("src/pay.ts", source),
            vec![
                (Kind::Function, "charge".into(), "function charge(amount: number): Receipt".into(), true),
                (Kind::Function, "helper".into(), "function helper()".into(), false),
                (Kind::Class, "Wallet".into(), "class Wallet".into(), true),
                (Kind::Method, "Wallet.refund".into(), "refund(id: string)".into(), true),
                (Kind::Function, "total".into(), "total = (a: number) =>".into(), true),
            ]
        );
    }

    #[test]
    fn tsx_and_javascript_read_too_and_other_files_not() {
        assert_eq!(found("src/Badge.tsx", "export function Badge() { return <b/>; }").len(), 1);
        assert!(!found("src/badge.js", "function badge() {}")[0].3, "not exported");
        assert!(!readable("package.json") && !readable("README.md") && !readable("db/schema.sql"));
        assert_eq!(changes("package.json", "{}", "{\"a\": 1}"), vec![]);
    }

    #[test]
    fn each_state_is_found_and_unchanged_symbols_are_left_out() {
        let base = "def kept(a):\n    return a\n\ndef body(a):\n    return a\n\ndef sig(a):\n    return a\n\ndef gone(a):\n    return a\n";
        let head =
            "def kept(a):\n    return a\n\ndef body(a):\n    return a + 1\n\ndef sig(a, b):\n    return a\n\ndef fresh():\n    pass\n";
        assert_eq!(
            states("m.py", base, head),
            vec![
                (State::Removed, "gone".into()),
                (State::Signature, "sig".into()),
                (State::Added, "fresh".into()),
                (State::Body, "body".into())
            ]
        );
    }

    #[test]
    fn a_symbol_with_the_same_body_under_a_new_name_is_a_rename() {
        let body = "    total = 0\n    for item in items:\n        total += item.price * item.count\n    return total\n";
        let changes = changes("m.py", &format!("def sum_items(items):\n{body}"), &format!("def total_of(items):\n{body}"));
        assert_eq!(changes.len(), 1);
        assert_eq!((changes[0].state, changes[0].symbol.name.as_str()), (State::Renamed, "total_of"));
        assert_eq!(changes[0].before.as_ref().unwrap().name, "sum_items");
    }

    #[test]
    fn below_the_similarity_threshold_a_rename_shows_as_removed_and_added() {
        let base = "def sum_items(items):\n    total = 0\n    for item in items:\n        total += item.price\n    return total\n";
        let head = "def total_of(orders):\n    return fetch(orders).amount\n";
        assert_eq!(states("m.py", base, head), vec![(State::Removed, "sum_items".into()), (State::Added, "total_of".into())]);
    }

    #[test]
    fn a_rename_stays_inside_its_class() {
        let body = "        return [x for x in self.items if x.ok]\n";
        let base = format!("class A:\n    def old(self):\n{body}\nclass B:\n    pass\n");
        let head = format!("class A:\n    pass\nclass B:\n    def new(self):\n{body}");
        assert_eq!(states("m.py", &base, &head), vec![(State::Removed, "A.old".into()), (State::Added, "B.new".into())]);
    }

    #[test]
    fn public_risk_comes_first_then_renames_and_bodies_then_private_changes() {
        let base = "def _gone():\n    return 1\n\ndef gone():\n    return 2\n\ndef body(a):\n    return a\n";
        let head = "def body(a):\n    return a + 1\n\ndef _fresh():\n    return 3\n\ndef fresh():\n    return 4\n";
        assert_eq!(
            states("m.py", base, head),
            vec![
                (State::Removed, "gone".into()),
                (State::Added, "fresh".into()),
                (State::Body, "body".into()),
                (State::Removed, "_gone".into()),
                (State::Added, "_fresh".into()),
            ]
        );
    }

    #[test]
    fn breaking_means_a_public_symbol_removed_or_called_differently() {
        let changes = changes("m.py", "def a(x):\n    pass\n\ndef _b():\n    pass\n", "def a(x, y):\n    pass\n");
        assert_eq!(changes.iter().map(Change::breaking).collect::<Vec<_>>(), vec![true, false]);
    }
}
