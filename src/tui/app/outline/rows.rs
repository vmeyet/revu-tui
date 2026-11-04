//! What the outline pane draws, built once whenever its state changes, so a frame only slices it.
use crate::diff::words::{Segment, text_segments};
use crate::outline::{Branch, Change, Item, Reading, Site, State};

/// Every row of the pane, and the entries among them the cursor walks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Rows {
    pub(crate) lines: Vec<PaneLine>,
    pub(crate) entries: Vec<Entry>,
    /// The line each entry is on.
    pub(crate) line_of: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PaneLine {
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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) lines: String,
    /// The branch's item; none in the flat list.
    pub(crate) item: Option<Item>,
    /// The change the row shows, by its index in the reading.
    pub(crate) change: Option<usize>,
    pub(crate) unsure: bool,
    pub(crate) site: Option<Site>,
}

impl Entry {
    /// A change of the list, or one shown in full in the tree, not a reference to it.
    pub(crate) fn full(&self) -> bool {
        self.change.is_some() && self.item.as_ref().is_none_or(|item| matches!(item, Item::Changed(_)))
    }
}

impl Rows {
    pub(super) fn build(reading: &Reading, flat_list: bool) -> Self {
        let entries = if flat_list { flat(reading) } else { tree(&reading.tree()) };
        let mut lines = if entries.is_empty() { vec![] } else { vec![PaneLine::Counts(counts(reading)), PaneLine::Blank] };
        let mut line_of = vec![];
        let mut path = None;
        for (index, entry) in entries.iter().enumerate() {
            let change = entry.change.map(|c| &reading.changes[c]);
            if let Some(change) = change.filter(|c| entry.item.is_none() && path != Some(&c.path)) {
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

fn counts(reading: &Reading) -> [usize; 3] {
    let count = |wanted: fn(&Change) -> bool| reading.changes.iter().filter(|c| wanted(c)).count();
    [count(Change::breaking), count(|c| c.state == State::Added), count(|c| c.state == State::Renamed)]
}

fn signature(change: &Change, indent: usize) -> Option<PaneLine> {
    let before = change.before.as_ref().filter(|_| change.state == State::Signature)?;
    Some(PaneLine::Signature { indent, segments: text_segments(&before.signature, &change.symbol.signature) })
}

fn flat(reading: &Reading) -> Vec<Entry> {
    (0..reading.changes.len()).map(|change| Entry { change: Some(change), ..Entry::default() }).collect()
}

fn tree(roots: &[Branch]) -> Vec<Entry> {
    roots.iter().flat_map(|root| entries_of(root, String::new(), "")).collect()
}

/// `branch` after `lines`, then its children, `lead` drawn before theirs.
fn entries_of(branch: &Branch, lines: String, lead: &str) -> Vec<Entry> {
    let change = match branch.item {
        Item::Changed(index) | Item::Seen(index) => Some(index),
        Item::Cycle(_) => None,
    };
    let mut entries = vec![Entry { lines, item: Some(branch.item.clone()), change, unsure: branch.unsure, site: branch.site.clone() }];
    let last = branch.children.len().saturating_sub(1);
    for (index, child) in branch.children.iter().enumerate() {
        let (here, below) = if index == last { ("└─ ", "   ") } else { ("├─ ", "│  ") };
        entries.extend(entries_of(child, format!("{lead}{here}"), &format!("{lead}{below}")));
    }
    entries
}
