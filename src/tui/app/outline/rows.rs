//! What the outline pane draws, built once whenever its state changes, so a frame only slices it.
use crate::diff::words::{Segment, text_segments};
use crate::outline::{Change, Reading, State};

/// The columns before a change's signature: under its name, past the cursor bar and its sign.
const SIGNATURE_INDENT: usize = 4;

/// Every row of the pane.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Rows {
    pub(crate) lines: Vec<PaneLine>,
    /// The line each change is on.
    pub(crate) line_of: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PaneLine {
    /// Breaking, added and renamed changes.
    Counts([usize; 3]),
    Blank,
    /// The file the next changes are in.
    Path(String),
    /// A change, by its index in the reading; the cursor stops here.
    Change(usize),
    /// The old signature turned into the new one, under a `~` change, `indent` columns in.
    Signature {
        indent: usize,
        segments: Vec<Segment>,
    },
}

impl Rows {
    pub(super) fn build(reading: &Reading) -> Self {
        let mut lines = if reading.changes.is_empty() { vec![] } else { vec![PaneLine::Counts(counts(reading)), PaneLine::Blank] };
        let mut line_of = vec![];
        let mut path = None;
        for (index, change) in reading.changes.iter().enumerate() {
            if path != Some(&change.path) {
                if path.is_some() {
                    lines.push(PaneLine::Blank);
                }
                path = Some(&change.path);
                lines.push(PaneLine::Path(change.path.clone()));
            }
            line_of.push(lines.len());
            lines.push(PaneLine::Change(index));
            lines.extend(signature(change));
        }
        Self { lines, line_of }
    }
}

fn counts(reading: &Reading) -> [usize; 3] {
    let count = |wanted: fn(&Change) -> bool| reading.changes.iter().filter(|c| wanted(c)).count();
    [count(Change::breaking), count(|c| c.state == State::Added), count(|c| c.state == State::Renamed)]
}

fn signature(change: &Change) -> Option<PaneLine> {
    let before = change.before.as_ref().filter(|_| change.state == State::Signature)?;
    Some(PaneLine::Signature { indent: SIGNATURE_INDENT, segments: text_segments(&before.signature, &change.symbol.signature) })
}
