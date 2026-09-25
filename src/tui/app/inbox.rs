//! Review mode: the MRs that need me, walked like an inbox. `]r` and `[r` move through them, a
//! publish offers the next one, the cursor comes back where the reader left each MR, and the queue
//! shows how far each review went.
use super::{Action, App, MrKey, Open};
use crate::review::{Progress, Review, Row};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// How long the cursor rests before its place is saved: moving through a file writes nothing.
const SETTLE: Duration = Duration::from_secs(1);

/// A place in a diff that survives a new commit: the file, and the line on each side it had.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spot {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new: Option<u32>,
}

/// Where the row at `index` sits, when it is a file or a line.
pub fn spot_at(review: &Review, row: &Row) -> Option<Spot> {
    let at = |file: usize, old: Option<u32>, new: Option<u32>| Some(Spot { path: review.files.get(file)?.new_path.clone(), old, new });
    let line = |file: usize, hunk: usize, index: usize| review.files.get(file)?.hunks.get(hunk)?.lines.get(index);
    match *row {
        Row::File { index, .. } | Row::Hunk { file: index, .. } => at(index, None, None),
        Row::Line { file, hunk, index } => line(file, hunk, index).and_then(|l| at(file, l.old, l.new)),
        Row::Pair { file, hunk, added, .. } => line(file, hunk, added).and_then(|l| at(file, l.old, l.new)),
        Row::Context { file, old, new, .. } => at(file, Some(old), Some(new)),
        _ => None,
    }
}

/// The row showing `spot`, else its file's row, else nothing: a file the new commit removed is gone.
fn row_of(open: &Open, spot: &Spot) -> Option<usize> {
    let on = |wanted: &Spot| open.rows.iter().position(|row| spot_at(&open.review, row).as_ref() == Some(wanted));
    on(spot).or_else(|| on(&Spot { path: spot.path.clone(), old: None, new: None }))
}

impl App {
    /// The MRs that need me, in the order review mode walks them: READY, then TO REVIEW, sorted
    /// and filtered as the queue shows them, folded sections and stacks included; what I already
    /// approved is skipped.
    pub fn review_order(&self) -> Vec<MrKey> {
        let Some(sections) = &self.sections else { return vec![] };
        let mut keys = vec![];
        for (name, mrs) in [("READY", &sections.ready), ("TO REVIEW", &sections.to_review)] {
            let wanted = mrs.iter().filter(|mr| self.matches_filter(mr) && !mr.approved_by.iter().any(|who| who == &self.me)).collect();
            for item in super::stack::group(&self.in_order(name, wanted)) {
                match item {
                    super::stack::Item::Single(mr) => keys.push(mr.key()),
                    super::stack::Item::Stack(stack) => keys.extend(stack.mrs.iter().map(|mr| mr.key())),
                }
            }
        }
        keys
    }

    /// The MR after (or before) the one being read, in review order: from the queue, the one
    /// after the selected row; outside the list, the first (or last) one.
    fn review_neighbour(&self, forward: bool) -> Option<MrKey> {
        let order = self.review_order();
        let here = self.open.as_ref().map(|o| o.key.clone()).or_else(|| self.selected_mr().map(crate::forge::QueueMr::key));
        match here.and_then(|key| order.iter().position(|k| *k == key)) {
            Some(at) if forward => order.get(at + 1).cloned(),
            Some(at) => at.checked_sub(1).and_then(|before| order.get(before)).cloned(),
            None if forward => order.first().cloned(),
            None => order.last().cloned(),
        }
    }

    /// `]r` / `[r`: the next or previous MR that needs me, opened in place.
    pub(super) fn walk_reviews(&mut self, forward: bool) -> Vec<Action> {
        match self.review_neighbour(forward) {
            Some(key) => self.switch_to(key),
            None if self.review_order().is_empty() => {
                self.toast("nothing needs you now");
                vec![]
            }
            None => {
                self.toast(if forward { "that was the last MR that needs you" } else { "that was the first MR that needs you" });
                vec![]
            }
        }
    }

    /// Opens `key` after saving where the reader stood in the MR they leave.
    pub(super) fn switch_to(&mut self, key: MrKey) -> Vec<Action> {
        let mut actions: Vec<Action> = self.save_spot_now().into_iter().collect();
        actions.extend(self.open_key(key));
        actions
    }

    /// After a publish: the status line offers the next MR that needs me, when there is one.
    pub(super) fn offer_next(&mut self) {
        self.offer = self.review_neighbour(true);
    }

    /// While the next MR is offered, `enter` opens it and any other key declines, then does what it does.
    pub(super) fn answer_offer(&mut self, key: crossterm::event::KeyEvent) -> Option<Vec<Action>> {
        let next = self.offer.take()?;
        match key.code {
            crossterm::event::KeyCode::Enter => Some(self.switch_to(next)),
            crossterm::event::KeyCode::Esc => Some(vec![]),
            _ => None,
        }
    }

    /// The place of the cursor in the open MR, when it is on a file or a line.
    pub fn spot(&self) -> Option<Spot> {
        let open = self.open.as_ref()?;
        spot_at(&open.review, open.row()?)
    }

    /// Saves the cursor's place once it rested for a second and differs from what is saved.
    pub(super) fn spot_tick(&mut self) -> Option<Action> {
        let (key, spot) = (self.open.as_ref()?.key.clone(), self.spot()?);
        match &self.spot_pending {
            Some((waiting, since)) if *waiting == spot => {
                let rested = self.now.duration_since(*since) >= SETTLE;
                let saved = self.spot_saved.as_ref() == Some(&(key, spot.clone()));
                (rested && !saved).then(|| self.save_spot_now()).flatten()
            }
            _ => {
                self.spot_pending = Some((spot, self.now));
                None
            }
        }
    }

    /// The open MR's state with the cursor's place, unless that place is already saved.
    pub(super) fn save_spot_now(&mut self) -> Option<Action> {
        let open = self.open.as_ref()?;
        let spot = self.spot()?;
        let saved = (open.key.clone(), spot.clone());
        if self.spot_saved.as_ref() == Some(&saved) {
            return None;
        }
        let action = Action::SaveState {
            key: open.key.clone(),
            fold: open.review.fold.clone(),
            viewed: open.review.viewed_fingerprints(),
            auto_folded: open.review.auto_folded.clone(),
            split: open.review.split,
            spot: Some(spot),
        };
        self.spot_saved = Some(saved);
        Some(action)
    }

    /// Puts the cursor back where the reader left this MR, unless they moved since it opened.
    pub(super) fn resume(&mut self, key: &MrKey, spot: &Spot) {
        let Some(open) = self.open.as_ref().filter(|o| &o.key == key) else { return };
        if open.selected != super::review::first_selectable(&open.rows) {
            return;
        }
        if let Some(row) = row_of(open, spot) {
            let mut next = open.clone();
            next.selected = row;
            self.spot_saved = Some((key.clone(), spot.clone()));
            self.open = Some(next);
        }
    }

    /// How far I went in the MR `key`, when I started it.
    pub fn progress_of(&self, key: &MrKey) -> Option<Progress> {
        self.progress.get(key).copied().filter(|p| p.viewed > 0)
    }

    /// Keeps the queue's count for the open MR in step with its viewed files.
    pub(super) fn count_viewed(&mut self, key: &MrKey, review: &Review) {
        self.progress.insert(key.clone(), review.progress());
    }
}

/// `viewed 7/12` with a thin bar of `width` cells, for the review header.
pub fn progress_bar(viewed: usize, files: usize, width: usize) -> (String, String) {
    let done = if files == 0 { 0 } else { (viewed * width).div_ceil(files).min(width) };
    ("━".repeat(done), "─".repeat(width - done))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn the_bar_fills_with_the_share_viewed() {
        assert_eq!(progress_bar(0, 12, 10), (String::new(), "─".repeat(10)));
        assert_eq!(progress_bar(6, 12, 10), ("━".repeat(5), "─".repeat(5)));
        assert_eq!(progress_bar(12, 12, 10), ("━".repeat(10), String::new()));
        assert_eq!(progress_bar(1, 12, 10).0.chars().count(), 1, "one viewed file already shows");
        assert_eq!(progress_bar(3, 0, 10).0, "", "no file, no progress");
    }
}
