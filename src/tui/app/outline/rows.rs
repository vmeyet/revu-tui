//! What the outline pane draws, built once whenever its state changes, so a frame only slices it.
use crate::outline::Reading;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PaneLine {
    Blank,
    /// The file the next changes are in.
    Path(String),
    /// A change, by its index in the reading.
    Change(usize),
}

/// Every row of the pane: each file's path, then its changes.
pub(super) fn build(reading: &Reading) -> Vec<PaneLine> {
    let mut lines = vec![];
    let mut path = None;
    for (index, change) in reading.changes.iter().enumerate() {
        if path != Some(&change.path) {
            if path.is_some() {
                lines.push(PaneLine::Blank);
            }
            path = Some(&change.path);
            lines.push(PaneLine::Path(change.path.clone()));
        }
        lines.push(PaneLine::Change(index));
    }
    lines
}
