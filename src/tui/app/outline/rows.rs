//! What the outline pane draws, built once whenever its state changes, so a frame only slices it.
use crate::diff::words::{Segment, text_segments};
use crate::outline::{Branch, Change, Item, Reading, Shape, Site, State};
use std::collections::BTreeSet;

/// Every row of the pane, and the entries among them the cursor walks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rows {
    pub lines: Vec<PaneLine>,
    pub entries: Vec<Entry>,
    /// The line each entry is on.
    pub line_of: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneLine {
    /// Breaking, added and renamed changes among those counted.
    Counts([usize; 3]),
    Blank,
    /// The file the flat list's next changes are in.
    Path(String),
    Entry(usize),
    /// The old signature turned into the new one, under a `~` entry, `indent` columns in.
    Signature {
        indent: usize,
        segments: Vec<Segment>,
    },
}

/// One row the cursor stops on: a change of the flat list, or a branch of the tree after the lines drawn before it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub lines: String,
    /// The branch's item; none in the flat list.
    pub item: Option<Item>,
    /// The change the row shows, by its index in the reading.
    pub change: Option<usize>,
    pub unsure: bool,
    pub site: Option<Site>,
    pub folded: bool,
    pub(super) children: bool,
    pub(super) at: Vec<usize>,
}

impl Entry {
    /// A change of the list, or one shown in full in the tree, not a reference to it.
    pub fn full(&self) -> bool {
        self.change.is_some() && self.item.as_ref().is_none_or(|item| matches!(item, Item::Changed(_)))
    }
}

/// How the pane shows the reading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct View {
    pub shape: Shape,
    pub flat: bool,
}

impl Rows {
    pub(super) fn build(reading: &Reading, view: View, folded: &BTreeSet<Vec<usize>>) -> Self {
        let counted: Vec<usize> = (0..reading.changes.len()).filter(|&c| view.shape.all || reading.changes[c].public()).collect();
        let entries = if view.flat { flat(&counted) } else { tree(&reading.tree(view.shape), folded) };
        let mut lines = vec![PaneLine::Counts(counts(reading, &counted)), PaneLine::Blank];
        let mut line_of = vec![];
        let mut path = None;
        for (index, entry) in entries.iter().enumerate() {
            let change = entry.change.map(|c| &reading.changes[c]);
            if let Some(change) = change.filter(|c| view.flat && path != Some(&c.path)) {
                if path.is_some() {
                    lines.push(PaneLine::Blank);
                }
                path = Some(&change.path);
                lines.push(PaneLine::Path(change.path.clone()));
            }
            line_of.push(lines.len());
            lines.push(PaneLine::Entry(index));
            if let Some(signature) = change.filter(|_| entry.full()).and_then(|c| signature(c, 4 + entry.lines.chars().count())) {
                lines.push(signature);
            }
        }
        Self { lines, entries, line_of }
    }
}

fn counts(reading: &Reading, counted: &[usize]) -> [usize; 3] {
    let count = |wanted: fn(&Change) -> bool| counted.iter().filter(|&&c| wanted(&reading.changes[c])).count();
    [count(Change::breaking), count(|c| c.state == State::Added), count(|c| c.state == State::Renamed)]
}

fn signature(change: &Change, indent: usize) -> Option<PaneLine> {
    let before = change.before.as_ref().filter(|_| change.state == State::Signature)?;
    Some(PaneLine::Signature { indent, segments: text_segments(&before.signature, &change.symbol.signature) })
}

fn flat(counted: &[usize]) -> Vec<Entry> {
    let entry = |&change| Entry {
        lines: String::new(),
        item: None,
        change: Some(change),
        unsure: false,
        site: None,
        folded: false,
        children: false,
        at: vec![],
    };
    counted.iter().map(entry).collect()
}

fn tree(roots: &[Branch], folded: &BTreeSet<Vec<usize>>) -> Vec<Entry> {
    roots.iter().enumerate().flat_map(|(index, root)| entries_of(root, String::new(), "", &[index], folded)).collect()
}

/// `branch` after `lines`, then its children unless folded, `lead` drawn before theirs.
fn entries_of(branch: &Branch, lines: String, lead: &str, at: &[usize], folded: &BTreeSet<Vec<usize>>) -> Vec<Entry> {
    let closed = folded.contains(at);
    let change = match branch.item {
        Item::Changed(index) | Item::Seen(index) => Some(index),
        _ => None,
    };
    let mut entries = vec![Entry {
        lines,
        item: Some(branch.item.clone()),
        change,
        unsure: branch.unsure,
        site: branch.site.clone(),
        folded: closed,
        children: !branch.children.is_empty(),
        at: at.to_vec(),
    }];
    if closed {
        return entries;
    }
    let last = branch.children.len().saturating_sub(1);
    for (index, child) in branch.children.iter().enumerate() {
        let (here, below) = if index == last { ("└─ ", "   ") } else { ("├─ ", "│  ") };
        entries.extend(entries_of(child, format!("{lead}{here}"), &format!("{lead}{below}"), &[at, &[index]].concat(), folded));
    }
    entries
}
