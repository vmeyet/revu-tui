//! The call tree: changed public symbols as roots, the changed symbols they call under them, linked
//! through at most two unchanged functions of the changed files; private ones nobody changed calls go last.
use super::{Change, Symbol};
use crate::forge::Side;
use std::collections::HashMap;

/// Unchanged functions shown in a row between two changed ones; a longer chain folds into one row.
const BRIDGES: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Under each symbol, the symbols it calls.
    Calls,
    /// Under each symbol, the symbols that call it.
    CalledBy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    /// A changed symbol, by its index in the changes, shown in full here.
    Changed(usize),
    /// A changed symbol shown in full elsewhere.
    Seen(usize),
    /// An unchanged function linking two changed symbols.
    Bridge(String),
    /// Unchanged calls folded away, how many.
    Fold(usize),
    /// A call back to a symbol above it.
    Cycle(String),
    /// The changed symbols no changed symbol links to.
    Unreached,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    pub item: Item,
    /// The name matched several symbols: the link may be wrong.
    pub unsure: bool,
    /// Where the call linking it to its parent is.
    pub site: Option<Site>,
    pub children: Vec<Branch>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Site {
    pub path: String,
    pub side: Side,
    pub line: u32,
}

/// One symbol of the graph: a change, or an unchanged symbol of a changed file, read on `side`.
struct Node<'a> {
    path: &'a str,
    symbol: &'a Symbol,
    change: Option<usize>,
    side: Side,
}

/// A link to follow: the node it leads to, and the call making it, in node `caller` at `line`.
#[derive(Clone, Copy)]
struct Link {
    to: usize,
    caller: usize,
    line: u32,
    unsure: bool,
}

struct Graph<'a> {
    nodes: Vec<Node<'a>>,
    links: Vec<Vec<Link>>,
}

pub(super) fn build(changes: &[Change], unchanged: &[(String, Symbol)], direction: Direction) -> Vec<Branch> {
    let graph = Graph::new(changes, unchanged, direction);
    let linked = graph.linked_from_changed();
    let mut walk = Walk { graph: &graph, seen: changes.iter().map(Change::public).collect() };
    let mut roots: Vec<Branch> = (0..changes.len()).filter(|&change| changes[change].public()).map(|change| walk.root(change)).collect();
    let first: Vec<usize> = (0..changes.len()).filter(|&change| !walk.seen[change] && !linked[change]).collect();
    let unreached: Vec<Branch> = first.into_iter().chain(0..changes.len()).filter_map(|change| walk.unseen(change)).collect();
    if !unreached.is_empty() {
        roots.push(Branch { children: unreached, ..Branch::of(Item::Unreached) });
    }
    roots
}

/// One walk down the graph; it remembers the changes it showed in full, so a later one shows a reference.
struct Walk<'g> {
    graph: &'g Graph<'g>,
    seen: Vec<bool>,
}

impl Direction {
    pub fn reversed(self) -> Self {
        match self {
            Self::Calls => Self::CalledBy,
            Self::CalledBy => Self::Calls,
        }
    }
}

impl Branch {
    fn of(item: Item) -> Self {
        Self { item, unsure: false, site: None, children: vec![] }
    }
}

impl<'a> Graph<'a> {
    /// Changes come first, so a change's node is its index; a call links to every symbol of its name.
    fn new(changes: &'a [Change], unchanged: &'a [(String, Symbol)], direction: Direction) -> Self {
        let nodes: Vec<Node> = changes
            .iter()
            .enumerate()
            .map(|(index, c)| Node { path: &c.path, symbol: &c.symbol, change: Some(index), side: c.side() })
            .chain(unchanged.iter().map(|(path, symbol)| Node { path, symbol, change: None, side: Side::New }))
            .collect();
        let mut named: HashMap<&str, Vec<usize>> = HashMap::new();
        for (index, node) in nodes.iter().enumerate() {
            named.entry(node.symbol.short_name()).or_default().push(index);
        }
        let mut links = vec![vec![]; nodes.len()];
        for (caller, node) in nodes.iter().enumerate() {
            for call in &node.symbol.calls {
                let called = named.get(call.name.as_str()).map_or(&[][..], Vec::as_slice);
                for &to in called {
                    let link = Link { to, caller, line: call.line, unsure: called.len() > 1 };
                    match direction {
                        Direction::Calls => push_once(&mut links[caller], link),
                        Direction::CalledBy => push_once(&mut links[to], Link { to: caller, ..link }),
                    }
                }
            }
        }
        Self { nodes, links }
    }

    fn change(&self, node: usize) -> Option<usize> {
        self.nodes[node].change
    }

    fn name(&self, node: usize) -> String {
        self.nodes[node].symbol.name.clone()
    }

    fn site(&self, link: &Link) -> Site {
        let caller = &self.nodes[link.caller];
        Site { path: caller.path.to_owned(), side: caller.side, line: link.line }
    }

    /// The changes another change links to directly.
    fn linked_from_changed(&self) -> Vec<bool> {
        let mut linked = vec![false; self.nodes.len()];
        for (from, links) in self.links.iter().enumerate().filter(|(from, _)| self.change(*from).is_some()) {
            for link in links.iter().filter(|link| link.to != from) {
                linked[link.to] = true;
            }
        }
        linked
    }

    /// The changed nodes reached from unchanged `start` through unchanged ones only, each with how many unchanged calls lead there.
    fn beyond(&self, start: usize, path: &[usize]) -> Vec<(usize, usize)> {
        let mut reached: Vec<(usize, usize)> = vec![];
        let mut visited = vec![false; self.nodes.len()];
        let mut queue = std::collections::VecDeque::from([(start, 1)]);
        visited[start] = true;
        while let Some((node, hops)) = queue.pop_front() {
            for link in &self.links[node] {
                match self.change(link.to) {
                    Some(_) if !reached.iter().any(|(n, _)| *n == link.to) => reached.push((link.to, hops)),
                    None if !visited[link.to] && !path.contains(&link.to) => {
                        visited[link.to] = true;
                        queue.push_back((link.to, hops + 1));
                    }
                    _ => {}
                }
            }
        }
        reached
    }
}

impl Walk<'_> {
    /// A root, shown in full whether seen or not: roots are marked seen before the walk so a call to one shows a reference.
    fn root(&mut self, change: usize) -> Branch {
        Branch { children: self.children(change, 0, &[change]), ..Branch::of(Item::Changed(change)) }
    }

    /// A change not shown yet, in full.
    fn unseen(&mut self, change: usize) -> Option<Branch> {
        (!self.seen[change]).then(|| self.changed(change, &[]))
    }

    /// What hangs under `from`, `path` being the nodes above it and `bridges` the unchanged ones right above.
    fn children(&mut self, from: usize, bridges: usize, path: &[usize]) -> Vec<Branch> {
        let graph = self.graph;
        let mut found = vec![];
        for link in &graph.links[from] {
            let linked = |branch: Branch| Branch { site: Some(graph.site(link)), unsure: link.unsure, ..branch };
            match graph.change(link.to) {
                Some(_) => found.push(linked(self.changed(link.to, path))),
                None if path.contains(&link.to) => {}
                None if bridges < BRIDGES => {
                    let children = self.children(link.to, bridges + 1, &[path, &[link.to]].concat());
                    if !children.is_empty() {
                        found.push(linked(Branch { children, ..Branch::of(Item::Bridge(graph.name(link.to))) }));
                    }
                }
                None => {
                    for (node, hops) in graph.beyond(link.to, path) {
                        found.push(linked(Branch { children: vec![self.changed(node, path)], ..Branch::of(Item::Fold(hops)) }));
                    }
                }
            }
        }
        found
    }

    /// A change, whose node is its index: in full the first time, a reference after, a cycle when it is above.
    fn changed(&mut self, change: usize, path: &[usize]) -> Branch {
        if path.contains(&change) {
            return Branch::of(Item::Cycle(self.graph.name(change)));
        }
        if self.seen[change] {
            return Branch::of(Item::Seen(change));
        }
        self.seen[change] = true;
        Branch { children: self.children(change, 0, &[path, &[change]].concat()), ..Branch::of(Item::Changed(change)) }
    }
}

/// The first call between two symbols makes the link; later ones add nothing.
fn push_once(links: &mut Vec<Link>, link: Link) {
    if !links.iter().any(|l| l.to == link.to) {
        links.push(link);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::super::{Reading, read};
    use super::*;

    /// Each branch as one indented line: `name`, `?` when unsure.
    fn sketch(reading: &Reading, direction: Direction) -> Vec<String> {
        fn walk(reading: &Reading, branches: &[Branch], depth: usize, out: &mut Vec<String>) {
            for branch in branches {
                let name = |change: usize| reading.changes[change].symbol.name.clone();
                let text = match &branch.item {
                    Item::Changed(change) => name(*change),
                    Item::Seen(change) => format!("({})", name(*change)),
                    Item::Bridge(name) => format!("· {name}()"),
                    Item::Fold(hops) => format!("… {hops} calls"),
                    Item::Cycle(name) => format!("↺ {name}"),
                    Item::Unreached => "unreached".to_owned(),
                };
                out.push(format!("{}{text}{}", "  ".repeat(depth), if branch.unsure { " ?" } else { "" }));
                walk(reading, &branch.children, depth + 1, out);
            }
        }
        let mut out = vec![];
        walk(reading, reading.tree(direction), 0, &mut out);
        out
    }

    /// One Python file whose base is `head` without the `added` functions, each block apart by a blank line.
    fn python(path: &str, added: &[&str], head: &str) -> (String, String, String) {
        let kept = |block: &&str| !added.iter().any(|name| block.starts_with(&format!("def {name}(")));
        let base = head.split("\n\n").filter(kept).collect::<Vec<_>>().join("\n\n");
        (path.to_owned(), base, head.to_owned())
    }

    #[test]
    fn a_changed_private_symbol_hangs_under_the_public_one_calling_it_and_a_cycle_is_cut() {
        let head = "def pay():\n    return _charge()\n\ndef _charge():\n    return pay()\n";
        let reading = read(&[python("pay.py", &["pay", "_charge"], head)]);
        assert_eq!(sketch(&reading, Direction::Calls), vec!["pay", "  _charge", "    ↺ pay"]);
    }

    #[test]
    fn a_name_several_symbols_carry_is_an_unsure_link() {
        let reading = read(&[
            python("a.py", &["pay", "_save"], "def pay():\n    return _save()\n\ndef _save():\n    return 1\n"),
            python("b.py", &["_save"], "def _save():\n    return 2\n"),
        ]);
        assert_eq!(sketch(&reading, Direction::Calls), ["pay", "  _save ?", "  _save ?"]);
    }

    #[test]
    fn two_unchanged_functions_bridge_and_a_longer_chain_folds() {
        let head = "def pay():\n    return a()\n\ndef a():\n    return b()\n\ndef b():\n    return c()\n\ndef c():\n    return d()\n\ndef d():\n    return _end()\n\ndef _end():\n    return 1\n";
        let reading = read(&[python("pay.py", &["pay", "_end"], head)]);
        assert_eq!(sketch(&reading, Direction::Calls), vec!["pay", "  · a()", "    · b()", "      … 2 calls", "        _end"]);
    }

    #[test]
    fn an_unchanged_function_leading_nowhere_changed_is_not_shown() {
        let head = "def pay():\n    return log()\n\ndef log():\n    return 1\n";
        let reading = read(&[python("pay.py", &["pay"], head)]);
        assert_eq!(sketch(&reading, Direction::Calls), vec!["pay"]);
    }

    #[test]
    fn private_symbols_no_change_calls_go_last_under_unreached_and_a_second_parent_shows_a_reference() {
        let head = "def pay():\n    return _fee()\n\ndef refund():\n    return _fee()\n\ndef _fee():\n    return 1\n\ndef _orphan():\n    return 2\n";
        let reading = read(&[python("pay.py", &["pay", "refund", "_fee", "_orphan"], head)]);
        assert_eq!(sketch(&reading, Direction::Calls), vec!["pay", "  _fee", "refund", "  (_fee)", "unreached", "  _orphan"]);
    }

    #[test]
    fn called_by_lists_the_callers_of_each_changed_symbol() {
        let base = "def charge(card):\n    return 1\n\ndef pay():\n    return charge(1)\n";
        let head = "def charge(card, amount):\n    return 1\n\ndef pay():\n    return charge(1, 2)\n";
        let reading = read(&[("pay.py".to_owned(), base.to_owned(), head.to_owned())]);
        assert_eq!(sketch(&reading, Direction::CalledBy), vec!["charge", "  (pay)", "pay"]);
        let site = reading.tree(Direction::CalledBy)[0].children[0].site.clone().unwrap();
        assert_eq!(site, Site { path: "pay.py".into(), side: Side::New, line: 5 });
    }
}
