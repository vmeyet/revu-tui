//! The call tree: the changed symbols no other change calls as roots, the changes they call under them in full.
use super::{Change, State, Symbol};
use crate::forge::Side;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Item {
    /// A changed symbol, by its index in the changes, shown in full here.
    Changed(usize),
    /// A changed symbol shown in full elsewhere.
    Seen(usize),
    /// A call back to a symbol above it.
    Cycle(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Branch {
    pub(crate) item: Item,
    /// The name matched several symbols: the link may be wrong.
    pub(crate) unsure: bool,
    /// Where the call linking it to its parent is.
    pub(crate) site: Option<Site>,
    pub(crate) children: Vec<Branch>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Site {
    pub(crate) path: String,
    pub(crate) side: Side,
    pub(crate) line: u32,
}

/// A call from one change to another: the change it leads to, and the line of the call.
#[derive(Clone, Copy)]
struct Link {
    to: usize,
    line: u32,
    unsure: bool,
}

struct Graph<'a> {
    changes: &'a [Change],
    links: Vec<Vec<Link>>,
}

/// Which part of the MR a tree shows: its code, or its tests and the code each one calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Section {
    Code,
    Tests,
}

/// Code roots first among the changes no other change calls, then any change still unseen, both riskiest first.
/// Test roots are every changed test symbol, in order; nothing links into a test file.
pub(super) fn build(changes: &[Change], section: Section) -> Vec<Branch> {
    let graph = Graph::new(changes, section);
    let in_section = |change: usize| super::is_test(&changes[change].path) == (section == Section::Tests);
    let mut walk = Walk { graph: &graph, seen: vec![false; changes.len()] };
    if section == Section::Tests {
        return (0..changes.len()).filter(|&change| in_section(change)).filter_map(|change| walk.unseen(change)).collect();
    }
    let reached = graph.called_by_another();
    let candidates: Vec<usize> = (0..changes.len()).filter(|&change| in_section(change)).collect();
    let first: Vec<usize> = candidates.iter().copied().filter(|&change| !reached[change]).collect();
    first.into_iter().chain(candidates).filter_map(|change| walk.unseen(change)).collect()
}

/// One walk down the graph; it remembers the symbols it showed in full, so a later one shows a reference.
struct Walk<'g> {
    graph: &'g Graph<'g>,
    seen: Vec<bool>,
}

impl Branch {
    fn new(item: Item) -> Self {
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
    /// A call links to every code change of its name on its own side; test symbols call from the tests section only.
    fn new(changes: &'a [Change], section: Section) -> Self {
        let mut on_head: HashMap<&str, Vec<usize>> = HashMap::new();
        let mut on_base: HashMap<&str, Vec<usize>> = HashMap::new();
        for (index, change) in changes.iter().enumerate().filter(|(_, change)| !super::is_test(&change.path)) {
            let (head, base) = names(change);
            if let Some(name) = head {
                on_head.entry(name).or_default().push(index);
            }
            if let Some(name) = base {
                on_base.entry(name).or_default().push(index);
            }
        }
        let mut links = vec![vec![]; changes.len()];
        let calling = |change: &Change| section == Section::Tests || !super::is_test(&change.path);
        for (caller, change) in changes.iter().enumerate().filter(|(_, change)| calling(change)) {
            let named = if change.side() == Side::Old { &on_base } else { &on_head };
            for call in &change.symbol.calls {
                let called = named.get(call.name.as_str()).map_or(&[][..], Vec::as_slice);
                for &to in called {
                    push_once(&mut links[caller], Link { to, line: call.line, unsure: called.len() > 1 });
                }
            }
        }
        Self { changes, links }
    }

    fn name(&self, change: usize) -> String {
        self.changes[change].symbol.name.clone()
    }

    fn site(&self, caller: usize, link: &Link) -> Site {
        let change = &self.changes[caller];
        Site { path: change.path.clone(), side: change.side(), line: link.line }
    }

    fn called_by_another(&self) -> Vec<bool> {
        let mut reached = vec![false; self.changes.len()];
        for (caller, links) in self.links.iter().enumerate() {
            for link in links.iter().filter(|link| link.to != caller) {
                reached[link.to] = true;
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

    /// In full the first time, a reference after, a cycle when it is above, `path` being the changes above it.
    fn changed(&mut self, change: usize, path: &[usize]) -> Branch {
        if path.contains(&change) {
            return Branch::new(Item::Cycle(self.graph.name(change)));
        }
        if self.seen[change] {
            return Branch::new(Item::Seen(change));
        }
        self.seen[change] = true;
        let graph = self.graph;
        let below = [path, &[change]].concat();
        let children = graph.links[change]
            .iter()
            .map(|link| Branch { site: Some(graph.site(change, link)), unsure: link.unsure, ..self.changed(link.to, &below) })
            .collect();
        Branch { children, ..Branch::new(Item::Changed(change)) }
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
    use super::super::{Reading, read};
    use super::*;

    /// Each branch as one indented line: `name`, `?` when unsure.
    fn sketch(reading: &Reading) -> Vec<String> {
        fn walk(reading: &Reading, branches: &[Branch], depth: usize, out: &mut Vec<String>) {
            for branch in branches {
                let name = |change: usize| reading.changes[change].symbol.name.clone();
                let text = match &branch.item {
                    Item::Changed(change) => name(*change),
                    Item::Seen(change) => format!("({})", name(*change)),
                    Item::Cycle(name) => format!("↺ {name}"),
                };
                out.push(format!("{}{text}{}", "  ".repeat(depth), if branch.unsure { " ?" } else { "" }));
                walk(reading, &branch.children, depth + 1, out);
            }
        }
        let mut out = vec![];
        walk(reading, &reading.tree(Section::Code), 0, &mut out);
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
        assert_eq!(sketch(&reading), ["checkout", "  charge", "    _fee", "refund", "  (charge)"]);
    }

    #[test]
    fn a_cycle_is_cut_where_it_comes_back() {
        let head = "def pay():\n    return _charge()\n\ndef _charge():\n    return pay()\n";
        let reading = read(&[python("pay.py", &["pay", "_charge"], head)]);
        assert_eq!(sketch(&reading), ["pay", "  _charge", "    ↺ pay"]);
    }

    #[test]
    fn a_name_several_symbols_carry_on_one_side_is_an_unsure_link() {
        let reading = read(&[
            python("a.py", &["pay", "_save"], "def pay():\n    return _save()\n\ndef _save():\n    return 1\n"),
            python("b.py", &["_save"], "def _save():\n    return 2\n"),
        ]);
        assert_eq!(sketch(&reading), ["pay", "  _save ?", "  _save ?"]);
    }

    #[test]
    fn a_head_call_links_only_to_head_symbols() {
        let moved = "def to_date(x):\n    return x.day\n";
        let reading = read(&[
            ("a.py".to_owned(), moved.to_owned(), String::new()),
            python("b.py", &["to_date", "is_current"], &format!("{moved}\ndef is_current():\n    return to_date(1)\n")),
        ]);
        let tree = reading.tree(Section::Code);
        let current = tree.iter().find(|b| matches!(b.item, Item::Changed(c) if reading.changes[c].symbol.name == "is_current")).unwrap();
        let [call] = current.children.as_slice() else { panic!("one call: {:?}", current.children) };
        let Item::Changed(callee) = call.item else { panic!("a change: {call:?}") };
        assert_eq!((reading.changes[callee].state, reading.changes[callee].path.as_str(), call.unsure), (State::Added, "b.py", false));
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
        let tests = || {
            let mut out = vec![];
            for branch in reading.tree(Section::Tests) {
                let Item::Changed(change) = branch.item else { panic!("a test root: {branch:?}") };
                out.push((reading.changes[change].symbol.name.clone(), branch.children.len()));
            }
            out
        };
        assert_eq!(sketch(&reading), ["pay", "  _fee"], "helper() is a test file's");
        assert_eq!(tests(), [("helper".to_owned(), 0), ("test_pay".to_owned(), 1), ("test_fee".to_owned(), 1)]);
        let under_test_pay = &reading.tree(Section::Tests)[1].children[0];
        assert_eq!(under_test_pay.children.len(), 1, "pay in full, with _fee under it");
    }
}
