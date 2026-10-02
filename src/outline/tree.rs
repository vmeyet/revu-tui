//! The call tree: the changed symbols nothing changed calls as roots, what they call under them in full,
//! linked through at most two unchanged functions of the changed files, or through all of them in the whole stack.
use super::{Change, State, Symbol};
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
    /// An unchanged function linking two changed symbols, or any in the whole stack.
    Bridge(String),
    /// Unchanged calls folded away, how many.
    Fold(usize),
    /// A call back to a symbol above it.
    Cycle(String),
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

/// How the tree is cut: which changes may be roots, and whether every unchanged function shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    pub direction: Direction,
    /// Private changes may be roots too.
    pub all: bool,
    /// Every unchanged function called, not only those linking two changes.
    pub stack: bool,
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

/// Which part of the MR a tree shows: its code, or its tests and the code each one calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Code,
    Tests,
}

/// Code roots first among the changes nothing changed reaches, then any change still unseen, both riskiest first.
/// Test roots are every changed test symbol, in order; nothing links into a test file.
pub(super) fn build(changes: &[Change], unchanged: &[(String, Symbol)], shape: Shape, section: Section) -> Vec<Branch> {
    let direction = if section == Section::Tests { Direction::Calls } else { shape.direction };
    let graph = Graph::new(changes, unchanged, direction, section);
    let in_section = |change: usize| super::is_test(&changes[change].path) == (section == Section::Tests);
    let mut walk = Walk { graph: &graph, seen: vec![false; graph.nodes.len()], stack: shape.stack };
    if section == Section::Tests {
        return (0..changes.len()).filter(|&change| in_section(change)).filter_map(|change| walk.unseen(change)).collect();
    }
    let reached = graph.reached_from_changed();
    let candidates: Vec<usize> =
        (0..changes.len()).filter(|&change| in_section(change) && (shape.all || changes[change].public())).collect();
    let first: Vec<usize> = candidates.iter().copied().filter(|&change| !reached[change]).collect();
    first.into_iter().chain(candidates).filter_map(|change| walk.unseen(change)).collect()
}

/// One walk down the graph; it remembers the symbols it showed in full, so a later one shows a reference.
struct Walk<'g> {
    graph: &'g Graph<'g>,
    seen: Vec<bool>,
    stack: bool,
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

/// The names a change answers to on head and on base: an added one is only on head, a removed one only on base.
fn names(change: &Change) -> (Option<&str>, Option<&str>) {
    let head = Some(change.symbol.short_name());
    match change.state {
        State::Added => (head, None),
        State::Removed => (None, head),
        State::Renamed => (head, change.before.as_ref().map(Symbol::short_name)),
        State::Signature | State::Body => (head, head),
    }
}

impl<'a> Graph<'a> {
    /// Changes come first, so a change's node is its index; a call links to every code symbol of its name on its own side.
    /// Test symbols call from the tests section only.
    fn new(changes: &'a [Change], unchanged: &'a [(String, Symbol)], direction: Direction, section: Section) -> Self {
        let nodes: Vec<Node> = changes
            .iter()
            .enumerate()
            .map(|(index, c)| Node { path: &c.path, symbol: &c.symbol, change: Some(index), side: c.side() })
            .chain(unchanged.iter().map(|(path, symbol)| Node { path, symbol, change: None, side: Side::New }))
            .collect();
        let mut on_head: HashMap<&str, Vec<usize>> = HashMap::new();
        let mut on_base: HashMap<&str, Vec<usize>> = HashMap::new();
        for (index, node) in nodes.iter().enumerate().filter(|(_, node)| !super::is_test(node.path)) {
            let (head, base) = node.change.map_or((Some(node.symbol.short_name()), Some(node.symbol.short_name())), |c| names(&changes[c]));
            if let Some(name) = head {
                on_head.entry(name).or_default().push(index);
            }
            if let Some(name) = base {
                on_base.entry(name).or_default().push(index);
            }
        }
        let mut links = vec![vec![]; nodes.len()];
        let calling = |node: &Node| section == Section::Tests || !super::is_test(node.path);
        for (caller, node) in nodes.iter().enumerate().filter(|(_, node)| calling(node)) {
            let named = if node.side == Side::Old { &on_base } else { &on_head };
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

    /// The changes another change links to, directly or through unchanged functions.
    fn reached_from_changed(&self) -> Vec<bool> {
        let mut reached = vec![false; self.nodes.len()];
        for from in (0..self.nodes.len()).filter(|&node| self.change(node).is_some()) {
            for (to, _) in self.beyond(from, &[from]) {
                reached[to] = true;
            }
        }
        reached
    }

    /// The changed nodes reached from `start` through unchanged ones only, each with how many unchanged calls lead there.
    fn beyond(&self, start: usize, path: &[usize]) -> Vec<(usize, usize)> {
        let mut reached: Vec<(usize, usize)> = vec![];
        let mut visited = vec![false; self.nodes.len()];
        let first = usize::from(self.change(start).is_none());
        let mut queue = std::collections::VecDeque::from([(start, first)]);
        visited[start] = true;
        while let Some((node, hops)) = queue.pop_front() {
            for link in &self.links[node] {
                match self.change(link.to) {
                    Some(_) if !path.contains(&link.to) && !reached.iter().any(|(n, _)| *n == link.to) => reached.push((link.to, hops)),
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
                None if self.stack => found.push(linked(self.unchanged(link.to, path))),
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

    /// In the whole stack, an unchanged function: in full the first time, bare after, a cycle when it is above.
    fn unchanged(&mut self, node: usize, path: &[usize]) -> Branch {
        let name = self.graph.name(node);
        if path.contains(&node) {
            return Branch::of(Item::Cycle(name));
        }
        if self.seen[node] {
            return Branch::of(Item::Bridge(name));
        }
        self.seen[node] = true;
        Branch { children: self.children(node, 0, &[path, &[node]].concat()), ..Branch::of(Item::Bridge(name)) }
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

    const CALLS: Shape = Shape { direction: Direction::Calls, all: false, stack: false };

    /// Each branch as one indented line: `name`, `?` when unsure.
    fn sketch(reading: &Reading, shape: Shape) -> Vec<String> {
        fn walk(reading: &Reading, branches: &[Branch], depth: usize, out: &mut Vec<String>) {
            for branch in branches {
                let name = |change: usize| reading.changes[change].symbol.name.clone();
                let text = match &branch.item {
                    Item::Changed(change) => name(*change),
                    Item::Seen(change) => format!("({})", name(*change)),
                    Item::Bridge(name) => format!("· {name}()"),
                    Item::Fold(hops) => format!("… {hops} calls"),
                    Item::Cycle(name) => format!("↺ {name}"),
                };
                out.push(format!("{}{text}{}", "  ".repeat(depth), if branch.unsure { " ?" } else { "" }));
                walk(reading, &branch.children, depth + 1, out);
            }
        }
        let mut out = vec![];
        walk(reading, &reading.tree(shape, Section::Code), 0, &mut out);
        out
    }

    /// One Python file whose base is `head` without the `added` functions, each block apart by a blank line.
    fn python(path: &str, added: &[&str], head: &str) -> (String, String, String) {
        let kept = |block: &&str| !added.iter().any(|name| block.starts_with(&format!("def {name}(")));
        let base = head.split("\n\n").filter(kept).collect::<Vec<_>>().join("\n\n");
        (path.to_owned(), base, head.to_owned())
    }

    #[test]
    fn a_root_is_a_change_nothing_changed_calls_and_what_it_calls_nests_in_full() {
        let head = "def charge():\n    return _fee()\n\ndef checkout():\n    return charge()\n\ndef refund():\n    return charge()\n\ndef _fee():\n    return 1\n";
        let reading = read(&[python("pay.py", &["charge", "checkout", "refund", "_fee"], head)]);
        assert_eq!(sketch(&reading, CALLS), ["checkout", "  charge", "    _fee", "refund", "  (charge)"]);
    }

    #[test]
    fn a_private_change_nothing_calls_is_a_root_only_with_all() {
        let head = "def pay():\n    return 1\n\ndef _orphan():\n    return 2\n";
        let reading = read(&[python("pay.py", &["pay", "_orphan"], head)]);
        assert_eq!(sketch(&reading, CALLS), ["pay"]);
        assert_eq!(sketch(&reading, Shape { all: true, ..CALLS }), ["pay", "_orphan"]);
    }

    #[test]
    fn a_public_change_only_a_hidden_private_one_calls_is_still_a_root() {
        let head = "def _main():\n    return pay()\n\ndef pay():\n    return 1\n";
        let reading = read(&[python("pay.py", &["_main", "pay"], head)]);
        assert_eq!(sketch(&reading, CALLS), ["pay"]);
        assert_eq!(sketch(&reading, Shape { all: true, ..CALLS }), ["_main", "  pay"]);
    }

    #[test]
    fn a_cycle_is_cut_where_it_comes_back() {
        let head = "def pay():\n    return _charge()\n\ndef _charge():\n    return pay()\n";
        let reading = read(&[python("pay.py", &["pay", "_charge"], head)]);
        assert_eq!(sketch(&reading, CALLS), ["pay", "  _charge", "    ↺ pay"]);
    }

    #[test]
    fn a_name_several_symbols_carry_on_one_side_is_an_unsure_link() {
        let reading = read(&[
            python("a.py", &["pay", "_save"], "def pay():\n    return _save()\n\ndef _save():\n    return 1\n"),
            python("b.py", &["_save"], "def _save():\n    return 2\n"),
        ]);
        assert_eq!(sketch(&reading, CALLS), ["pay", "  _save ?", "  _save ?"]);
    }

    #[test]
    fn a_head_call_links_only_to_head_symbols() {
        let moved = "def to_date(x):\n    return x.day\n";
        let reading = read(&[
            ("a.py".to_owned(), moved.to_owned(), String::new()),
            python("b.py", &["to_date", "is_current"], &format!("{moved}\ndef is_current():\n    return to_date(1)\n")),
        ]);
        let tree = reading.tree(CALLS, Section::Code);
        let current = tree.iter().find(|b| matches!(b.item, Item::Changed(c) if reading.changes[c].symbol.name == "is_current")).unwrap();
        let [call] = current.children.as_slice() else { panic!("one call: {:?}", current.children) };
        let Item::Changed(callee) = call.item else { panic!("a change: {call:?}") };
        assert_eq!((reading.changes[callee].state, reading.changes[callee].path.as_str(), call.unsure), (State::Added, "b.py", false));
    }

    #[test]
    fn two_unchanged_functions_bridge_and_a_longer_chain_folds() {
        let head = "def pay():\n    return a()\n\ndef a():\n    return b()\n\ndef b():\n    return c()\n\ndef c():\n    return d()\n\ndef d():\n    return _end()\n\ndef _end():\n    return 1\n";
        let reading = read(&[python("pay.py", &["pay", "_end"], head)]);
        assert_eq!(sketch(&reading, CALLS), ["pay", "  · a()", "    · b()", "      … 2 calls", "        _end"]);
    }

    #[test]
    fn an_unchanged_function_leading_nowhere_changed_shows_only_in_the_whole_stack() {
        let head = "def pay():\n    return log()\n\ndef log():\n    return fmt() + fmt()\n\ndef fmt():\n    return log()\n";
        let reading = read(&[python("pay.py", &["pay"], head)]);
        assert_eq!(sketch(&reading, CALLS), ["pay"]);
        assert_eq!(sketch(&reading, Shape { stack: true, ..CALLS }), ["pay", "  · log()", "    · fmt()", "      ↺ log"]);
    }

    #[test]
    fn called_by_lists_the_callers_of_each_changed_symbol() {
        let base = "def charge(card):\n    return 1\n\ndef pay():\n    return charge(1)\n";
        let head = "def charge(card, amount):\n    return 1\n\ndef pay():\n    return charge(1, 2)\n";
        let reading = read(&[("pay.py".to_owned(), base.to_owned(), head.to_owned())]);
        let shape = Shape { direction: Direction::CalledBy, ..CALLS };
        assert_eq!(sketch(&reading, shape), ["charge", "  pay"]);
        let site = reading.tree(shape, Section::Code)[0].children[0].site.clone().unwrap();
        assert_eq!(site, Site { path: "pay.py".into(), side: Side::New, line: 5 });
    }

    #[test]
    fn code_ignores_test_files_and_each_test_roots_the_code_it_calls() {
        let reading = read(&[
            python("shop/pay.py", &["pay", "_fee"], "def pay():\n    return _fee() + helper()\n\ndef _fee():\n    return 1\n"),
            python(
                "tests/test_pay.py",
                &["test_pay", "test_fee", "helper"],
                "def helper():\n    return 0\n\ndef test_pay():\n    assert pay() == helper()\n\ndef test_fee():\n    assert _fee()\n",
            ),
        ]);
        let tests = |shape| {
            let mut out = vec![];
            for branch in reading.tree(shape, Section::Tests) {
                let Item::Changed(change) = branch.item else { panic!("a test root: {branch:?}") };
                out.push((reading.changes[change].symbol.name.clone(), branch.children.len()));
            }
            out
        };
        assert_eq!(sketch(&reading, CALLS), ["pay", "  _fee"], "helper() is a test file's");
        assert_eq!(tests(CALLS), [("helper".to_owned(), 0), ("test_pay".to_owned(), 1), ("test_fee".to_owned(), 1)]);
        let under_test_pay = &reading.tree(CALLS, Section::Tests)[1].children[0];
        assert_eq!(under_test_pay.children.len(), 1, "pay in full, with _fee under it");
    }
}
