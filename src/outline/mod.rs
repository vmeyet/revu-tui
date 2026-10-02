//! The outline: which functions, methods and classes an MR changes. Each side of a file is read with
//! its grammar's own tags query, then the symbols of both sides are compared by kind and name, and
//! the calls the query finds link them into a tree.
mod tree;

pub use tree::{Branch, Direction, Item};

use crate::forge::Side;
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
    calls: Vec<Call>,
}

/// A call inside a symbol's body: the name called, and the line it is on.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Call {
    name: String,
    line: u32,
}

impl Symbol {
    /// The name without the classes around it, as a call names it.
    fn short_name(&self) -> &str {
        self.name.rsplit('.').next().unwrap_or(&self.name)
    }
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
    /// Public at head, or at base: a symbol that stopped being exported still counts.
    pub fn public(&self) -> bool {
        self.symbol.public || self.before.as_ref().is_some_and(|before| before.public)
    }

    /// The side its lines are on: base for a removed symbol, head for every other.
    pub fn side(&self) -> Side {
        if self.state == State::Removed { Side::Old } else { Side::New }
    }

    /// A public symbol gone, or called differently now.
    pub fn breaking(&self) -> bool {
        self.public() && matches!(self.state, State::Removed | State::Signature)
    }

    /// Public removals, signatures and additions first; private ones after renames and body changes.
    fn risk(&self) -> (bool, State, u32) {
        let private = !self.public() && self.state <= State::Added;
        (private, self.state, self.symbol.lines.0)
    }
}

/// What the outline read in the MR: the changed symbols, by file then risk, and the call tree over them both ways.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reading {
    pub changes: Vec<Change>,
    calls: Vec<Branch>,
    called_by: Vec<Branch>,
}

impl Reading {
    pub fn tree(&self, direction: Direction) -> &[Branch] {
        match direction {
            Direction::Calls => &self.calls,
            Direction::CalledBy => &self.called_by,
        }
    }
}

/// Reads each file, given as its path with its text at base and at head, empty on a side it does not exist.
pub fn read(files: &[(String, String, String)]) -> Reading {
    let mut changes = vec![];
    let mut unchanged = vec![];
    for (path, base, head) in files {
        let (changed, kept) = file(path, base, head);
        changes.extend(changed);
        unchanged.extend(kept.into_iter().map(|symbol| (path.clone(), symbol)));
    }
    let calls = tree::build(&changes, &unchanged, Direction::Calls);
    let called_by = tree::build(&changes, &unchanged, Direction::CalledBy);
    Reading { changes, calls, called_by }
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

/// Where the signature starts: at the `export` that holds the definition itself, so dropping it changes the signature.
fn signature_start(node: Node) -> usize {
    std::iter::successors(node.parent(), Node::parent)
        .take_while(|n| !matches!(n.kind(), "class_body" | "statement_block" | "program"))
        .find(|n| n.kind() == "export_statement")
        .map_or(node.start_byte(), |export| export.start_byte())
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

/// The symbols that changed between the two sides of `path`, riskiest first, and those of head that did not.
fn file(path: &str, base: &str, head: &str) -> (Vec<Change>, Vec<Symbol>) {
    let Some(tags) = tags_for(path) else { return (vec![], vec![]) };
    let (mut changes, unchanged) = compare(path, symbols(tags, base), symbols(tags, head));
    changes.sort_by_key(Change::risk);
    (changes, unchanged)
}

/// Every function, method and class of `source`, each with the calls in its body.
fn symbols(tags: &Tags, source: &str) -> Vec<Symbol> {
    let Some(config) = tags.configuration() else { return vec![] };
    let mut parser = Parser::new();
    let Some(tree) = parser.set_language(&config.language).ok().and_then(|()| parser.parse(source, None)) else { return vec![] };
    let mut context = TagsContext::new();
    let Ok((found, _)) = context.generate_tags(config, source.as_bytes(), None) else { return vec![] };
    let mut definitions: Vec<(Kind, Range<usize>, Range<usize>)> = vec![];
    let mut calls: Vec<(Range<usize>, Call)> = vec![];
    for tag in found.filter_map(Result::ok) {
        let kind = config.syntax_type_name(tag.syntax_type_id);
        match (tag.is_definition, kind_named(kind)) {
            (true, Some(kind)) => definitions.push((kind, tag.range, tag.name_range)),
            (false, _) if kind == "call" => {
                calls.push((tag.range, Call { name: source[tag.name_range].to_owned(), line: line(tag.span.start.row) }));
            }
            _ => {}
        }
    }
    let inside = |inner: &Range<usize>, outer: &Range<usize>| outer.start <= inner.start && inner.end <= outer.end;
    let mut owned: Vec<Vec<Call>> = vec![vec![]; definitions.len()];
    for (at, call) in calls {
        let innermost = definitions.iter().enumerate().filter(|(_, (_, r, _))| inside(&at, r)).min_by_key(|(_, (_, r, _))| r.len());
        if let Some((index, _)) = innermost {
            owned[index].push(call);
        }
    }
    definitions
        .iter()
        .zip(owned)
        .filter_map(|((kind, range, name), calls)| {
            let node = tree.root_node().descendant_for_byte_range(range.start, range.end)?;
            let classes: Vec<&str> = definitions
                .iter()
                .filter(|(k, r, _)| *k == Kind::Class && r != range && inside(range, r))
                .map(|(_, _, n)| &source[n.clone()])
                .collect();
            let kind = if *kind == Kind::Function && !classes.is_empty() { Kind::Method } else { *kind };
            Some(Symbol { calls, ..symbol(node, source, kind, &classes, &source[name.clone()], tags) })
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
    Symbol {
        kind,
        name: classes.iter().copied().chain([name]).collect::<Vec<_>>().join("."),
        signature: collapsed(&source[signature_start(node)..body_start]).trim_end_matches(':').trim_end().to_owned(),
        public: (tags.public)(node, name),
        lines: (line(node.start_position().row), line(node.end_position().row)),
        body: collapsed(&source[body_start..node.end_byte()]),
        calls: vec![],
    }
}

/// A tree-sitter row as a line number, from 1.
fn line(row: usize) -> u32 {
    u32::try_from(row + 1).unwrap_or(u32::MAX)
}

fn collapsed(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Symbols matched by kind and name, and those that did not change; a removed one and an added one alike enough read as a rename.
/// A class lists only when it comes, goes or changes its signature: its methods carry its body.
fn compare(path: &str, base: Vec<Symbol>, head: Vec<Symbol>) -> (Vec<Change>, Vec<Symbol>) {
    let keyed =
        |symbols: Vec<Symbol>| -> BTreeMap<(Kind, String), Symbol> { symbols.into_iter().map(|s| ((s.kind, s.name.clone()), s)).collect() };
    let (mut old, new) = (keyed(base), keyed(head));
    let change = |state, symbol: Symbol, before: Option<Symbol>| Change { path: path.to_owned(), state, symbol, before };
    let mut changes = vec![];
    let mut added = vec![];
    let mut unchanged = vec![];
    for (key, after) in new {
        match old.remove(&key) {
            None => added.push(after),
            Some(before) if before.signature != after.signature => changes.push(change(State::Signature, after, Some(before))),
            Some(before) if before.body != after.body && after.kind != Kind::Class => changes.push(change(State::Body, after, None)),
            Some(_) => unchanged.push(after),
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
    (changes, unchanged)
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

    fn changes(path: &str, base: &str, head: &str) -> Vec<Change> {
        file(path, base, head).0
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
                (Kind::Function, "charge".into(), "export function charge(amount: number): Receipt".into(), true),
                (Kind::Function, "helper".into(), "function helper()".into(), false),
                (Kind::Class, "Wallet".into(), "export class Wallet".into(), true),
                (Kind::Method, "Wallet.refund".into(), "refund(id: string)".into(), true),
                (Kind::Function, "total".into(), "export const total = (a: number) =>".into(), true),
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

    #[test]
    fn dropping_an_export_is_a_breaking_signature_change() {
        let changes = changes(
            "src/pay.ts",
            "export function charge(a: number) {\n  return a;\n}\n",
            "function charge(a: number) {\n  return a;\n}\n",
        );
        assert_eq!((changes[0].state, changes[0].breaking()), (State::Signature, true));
    }

    #[test]
    fn calls_belong_to_the_innermost_definition_around_them() {
        let symbols = symbols(tags_for("m.py").unwrap(), CART);
        let calls = |name: &str| {
            symbols.iter().find(|s| s.name == name).unwrap().calls.iter().map(|c| (c.name.clone(), c.line)).collect::<Vec<_>>()
        };
        assert_eq!(calls("Cart.total"), vec![("sum".to_owned(), 3)]);
        assert_eq!(calls("checkout"), vec![("pay".to_owned(), 9)]);
        assert_eq!(calls("Cart"), vec![]);
    }
}
