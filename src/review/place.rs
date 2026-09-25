//! Where conversations live: the anchor column's marks per line, and what the right pane lists
//! for one place (a line, the MR itself, a file's outdated threads, or all of them).
use super::{Anchor, Draft, Review, Row, Side, Thread};
use std::collections::BTreeMap;

/// The most pressing thing on a line, in rising order: an unresolved thread outranks my draft,
/// which outranks threads that are all resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mark {
    Resolved,
    Draft,
    Unresolved,
}

/// What the anchor column shows for one line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Marker {
    pub mark: Mark,
    /// Threads and new drafts on the line; replies in a thread count with their thread.
    pub count: usize,
    /// One of my drafts on the line has not reached the forge yet.
    pub unsaved: bool,
}

impl Marker {
    fn merge(self, other: Self) -> Self {
        Self { mark: self.mark.max(other.mark), count: self.count + other.count, unsaved: self.unsaved || other.unsaved }
    }
}

/// Markers by `(path, side, line)`, built once per review.
pub type Markers = BTreeMap<(String, Side, u32), Marker>;

/// Where the pane looks: a line of a file (both numbers of a context line), the MR, a file's outdated
/// threads, or every conversation of the MR.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    Line { file: usize, new: Option<u32>, old: Option<u32> },
    Mr,
    Outdated { file: usize },
    All,
}

/// Where one conversation hangs: on the MR, on a line of the diff, or on a line the diff no longer has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spot<'a> {
    Mr,
    Line(&'a Anchor),
    Outdated(&'a Anchor),
}

/// One entry of the pane: a thread with my replies to it, or a draft that starts a new thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conversation {
    pub thread: Option<String>,
    /// Indexes into `Review::drafts`: my replies to `thread`, or the one new draft.
    pub drafts: Vec<usize>,
}

impl Review {
    /// Every line carrying a thread or one of my drafts, with what its anchor column shows.
    pub fn markers(&self) -> Markers {
        let mut markers = Markers::new();
        let mut add = |key: (String, Side, u32), marker: Marker| {
            let merged = markers.get(&key).map_or(marker, |old| old.merge(marker));
            markers.insert(key, merged);
        };
        for thread in self.threads.iter().filter(|t| !t.outdated) {
            let Some(anchor) = &thread.anchor else { continue };
            add((anchor.path.clone(), anchor.side, anchor.line), self.marker_of_thread(thread));
        }
        for draft in self.drafts.iter().filter(|d| d.reply_to.is_none()) {
            let Some(anchor) = &draft.anchor else { continue };
            add((anchor.path.clone(), anchor.side, anchor.line), Marker { mark: Mark::Draft, count: 1, unsaved: draft.id.is_none() });
        }
        markers
    }

    fn marker_of_thread(&self, thread: &Thread) -> Marker {
        let mark = if thread.resolved { Mark::Resolved } else { Mark::Unresolved };
        let unsaved = self.replies_to(&thread.id).any(|(_, d)| d.id.is_none());
        let replied = self.replies_to(&thread.id).next().is_some();
        Marker { mark: if replied { mark.max(Mark::Draft) } else { mark }, count: 1, unsaved }
    }

    /// The most pressing mark among the conversations on the MR itself, by the anchor column's rule.
    pub fn mr_mark(&self) -> Option<Mark> {
        let threads = self.threads_in(&Place::Mr).into_iter().map(|t| self.marker_of_thread(t).mark);
        let drafts = self.new_drafts_in(&Place::Mr).into_iter().map(|_| Mark::Draft);
        threads.chain(drafts).max()
    }

    /// The marker a row shows: both sides of a context line, both lines of an inline pair.
    pub fn marker_of(&self, markers: &Markers, row: &Row) -> Option<Marker> {
        let Place::Line { file, new, old } = self.place_of(row)? else { return None };
        let file = &self.files[file];
        let new = new.and_then(|n| markers.get(&(file.new_path.clone(), Side::New, n)).copied());
        let old = old.and_then(|n| markers.get(&(file.old_path.clone(), Side::Old, n)).copied());
        match (new, old) {
            (Some(a), Some(b)) => Some(a.merge(b)),
            (a, b) => a.or(b),
        }
    }

    /// The place a row stands for in the pane; the header is the MR itself.
    pub fn place_of(&self, row: &Row) -> Option<Place> {
        match row {
            Row::Line { file, hunk, index } => {
                let line = &self.files[*file].hunks[*hunk].lines[*index];
                Some(Place::Line { file: *file, new: line.new, old: line.old })
            }
            Row::Pair { file, hunk, removed, added } => {
                let lines = &self.files[*file].hunks[*hunk].lines;
                Some(Place::Line { file: *file, new: lines[*added].new, old: lines[*removed].old })
            }
            Row::Context { file, old, new, .. } => Some(Place::Line { file: *file, new: Some(*new), old: Some(*old) }),
            Row::Header => Some(Place::Mr),
            Row::File { .. } | Row::Hunk { .. } | Row::Gap => None,
        }
    }

    /// What the pane lists at `place`: unresolved threads, then my new drafts, then resolved
    /// threads; oldest first inside each group, or in diff order when the pane lists them all.
    pub fn conversations(&self, place: &Place) -> Vec<Conversation> {
        let threads = self.threads_in(place);
        let (resolved, open): (Vec<&Thread>, Vec<&Thread>) = threads.into_iter().partition(|t| t.resolved);
        let with_replies =
            |t: &Thread| Conversation { thread: Some(t.id.clone()), drafts: self.replies_to(&t.id).map(|(i, _)| i).collect() };
        let new_drafts = self.new_drafts_in(place).into_iter().map(|index| Conversation { thread: None, drafts: vec![index] }).collect();
        let groups = [open.into_iter().map(with_replies).collect(), new_drafts, resolved.into_iter().map(with_replies).collect()];
        groups.into_iter().flat_map(|group| if *place == Place::All { self.in_diff_order(group) } else { group }).collect()
    }

    /// The MR's own conversations first, then files as the diff lists them, lines in order, a
    /// file's outdated threads after its lines.
    fn in_diff_order(&self, group: Vec<Conversation>) -> Vec<Conversation> {
        let rank = |anchor: &Anchor| self.file_of(anchor).map_or(usize::MAX, |file| file + 1);
        let mut sorted = group;
        sorted.sort_by_cached_key(|conversation| match self.spot(conversation) {
            Spot::Mr => (0, false, 0),
            Spot::Line(anchor) => (rank(anchor), false, anchor.line),
            Spot::Outdated(anchor) => (rank(anchor), true, anchor.line),
        });
        sorted
    }

    /// Whether `user` wrote a note of the conversation; a draft counts too, since every draft is mine.
    pub fn takes_part(&self, conversation: &Conversation, user: &str) -> bool {
        let thread = conversation.thread.as_deref().and_then(|id| self.thread(id));
        !conversation.drafts.is_empty() || thread.is_some_and(|t| t.notes.iter().any(|note| note.author.username == user))
    }

    /// Where a conversation hangs: its thread's anchor, or its new draft's.
    pub fn spot(&self, conversation: &Conversation) -> Spot<'_> {
        let thread = conversation.thread.as_deref().and_then(|id| self.thread(id));
        let anchor = match thread {
            Some(thread) => thread.anchor.as_ref(),
            None => conversation.drafts.first().and_then(|&index| self.drafts.get(index)?.anchor.as_ref()),
        };
        match anchor {
            None => Spot::Mr,
            Some(anchor) if thread.is_some_and(|t| t.outdated) => Spot::Outdated(anchor),
            Some(anchor) => Spot::Line(anchor),
        }
    }

    /// The file whose side of the change the anchor names.
    pub fn file_of(&self, anchor: &Anchor) -> Option<usize> {
        self.files.iter().position(|file| match anchor.side {
            Side::New => file.new_path == anchor.path,
            Side::Old => file.old_path == anchor.path,
        })
    }

    /// The code on the anchor's line, when the diff shows it.
    pub fn line_text(&self, anchor: &Anchor) -> Option<&str> {
        self.files.iter().find_map(|file| file.line_at(anchor)).map(|line| line.text.as_str())
    }

    /// Whether the row shows the anchor's line.
    pub fn row_holds(&self, row: &Row, anchor: &Anchor) -> bool {
        let Some(Place::Line { file, new, old }) = self.place_of(row) else { return false };
        let number = match anchor.side {
            Side::New => new,
            Side::Old => old,
        };
        self.file_of(anchor) == Some(file) && number == Some(anchor.line)
    }

    /// Threads anchored in the diff that `place` does not show: the footer's "n more in this file".
    pub fn others_in_file(&self, place: &Place) -> usize {
        let Place::Line { file, .. } = place else { return 0 };
        let here: Vec<&str> = self.threads_in(place).iter().map(|t| t.id.as_str()).collect();
        let file = &self.files[*file];
        self.threads
            .iter()
            .filter(|t| !t.outdated && !here.contains(&t.id.as_str()))
            .filter(|t| t.anchor.as_ref().is_some_and(|a| a.path == file.new_path || a.path == file.old_path))
            .count()
    }

    /// The text of the lines a note covers, first to last, on the side it hangs on:
    /// what a suggestion in that note replaces.
    pub fn text_at(&self, position: &crate::forge::Position) -> Vec<String> {
        let Some(file) = self.files.iter().find(|f| f.new_path == position.new_path || f.old_path == position.old_path) else {
            return vec![];
        };
        let side = position.line.side();
        let number = |line: crate::forge::LineRef| match side {
            Side::New => line.new,
            Side::Old => line.old,
        };
        let Some(last) = number(position.line) else { return vec![] };
        let first = position.start.and_then(number).unwrap_or(last);
        file.hunks
            .iter()
            .flat_map(|h| &h.lines)
            .filter(|l| match side {
                Side::New => l.new.is_some_and(|n| (first..=last).contains(&n)),
                Side::Old => l.old.is_some_and(|n| (first..=last).contains(&n)),
            })
            .map(|l| l.text.clone())
            .collect()
    }

    fn threads_in(&self, place: &Place) -> Vec<&Thread> {
        match place {
            Place::Line { file, new, old } => {
                let file = &self.files[*file];
                let new = new.map(|n| self.threads_at(&file.new_path, Side::New, n)).unwrap_or_default();
                let old = old.map(|n| self.threads_at(&file.old_path, Side::Old, n)).unwrap_or_default();
                new.into_iter().chain(old).collect()
            }
            Place::Mr => self.threads.iter().filter(|t| t.anchor.is_none()).collect(),
            Place::Outdated { file } => self.outdated(&self.files[*file].new_path),
            Place::All => self.threads.iter().collect(),
        }
    }

    fn new_drafts_in(&self, place: &Place) -> Vec<usize> {
        match place {
            Place::Line { file, new, old } => {
                let file = &self.files[*file];
                let new = new.map(|n| self.drafts_at(&file.new_path, Side::New, n)).unwrap_or_default();
                let old = old.map(|n| self.drafts_at(&file.old_path, Side::Old, n)).unwrap_or_default();
                new.into_iter().chain(old).map(|(index, _)| index).collect()
            }
            Place::Mr => {
                self.drafts.iter().enumerate().filter(|(_, d)| d.anchor.is_none() && d.reply_to.is_none()).map(|(i, _)| i).collect()
            }
            Place::All => self.drafts.iter().enumerate().filter(|(_, d)| d.reply_to.is_none()).map(|(i, _)| i).collect(),
            Place::Outdated { .. } => vec![],
        }
    }

    fn replies_to<'a>(&'a self, thread: &'a str) -> impl Iterator<Item = (usize, &'a Draft)> + 'a {
        self.drafts.iter().enumerate().filter(move |(_, d)| d.reply_to.as_deref() == Some(thread))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::super::tests::review;
    use super::*;
    use crate::review::Anchor;

    fn old_13() -> Anchor {
        Anchor { path: "src/pay/charge.rs".into(), side: Side::Old, line: 13 }
    }

    /// The removed line 13 of charge.rs, where the fixture's live thread sits.
    fn removed_line() -> Row {
        Row::Line { file: 0, hunk: 0, index: 1 }
    }

    #[test]
    fn a_line_shows_its_most_pressing_mark_and_counts_threads_and_new_drafts() {
        let review = review().with_resolved("c0ffee00c0ffee00", false);
        let markers = review.markers();
        assert_eq!(review.marker_of(&markers, &removed_line()), Some(Marker { mark: Mark::Unresolved, count: 1, unsaved: false }));
        assert_eq!(review.marker_of(&markers, &Row::Line { file: 0, hunk: 0, index: 0 }), None);
        let with_draft = review.with_drafts(vec![Draft::new(Some(old_13()), "nit")]);
        let markers = with_draft.markers();
        let marker = with_draft.marker_of(&markers, &removed_line()).unwrap();
        assert_eq!((marker.mark, marker.count, marker.unsaved), (Mark::Unresolved, 2, true), "the unsaved draft shows");
    }

    #[test]
    fn a_draft_outranks_resolved_threads_and_a_reply_marks_its_threads_line() {
        let review = review().with_resolved("c0ffee00c0ffee00", true);
        let markers = review.markers();
        assert_eq!(review.marker_of(&markers, &removed_line()).unwrap().mark, Mark::Resolved);
        let replied = review.with_drafts(vec![Draft::reply("c0ffee00c0ffee00", "agreed").with_id(3)]);
        let markers = replied.markers();
        let marker = replied.marker_of(&markers, &removed_line()).unwrap();
        assert_eq!((marker.mark, marker.count, marker.unsaved), (Mark::Draft, 1, false), "a reply counts with its thread");
    }

    #[test]
    fn outdated_and_mr_level_threads_never_mark_a_line() {
        let review = review();
        let markers = review.markers();
        assert_eq!(markers.len(), 1, "only the live thread on old line 13: {markers:?}");
        assert_eq!(review.conversations(&Place::Outdated { file: 0 }).len(), 1);
        assert_eq!(review.conversations(&Place::Mr).len(), 1);
    }

    #[test]
    fn the_mr_takes_the_most_pressing_mark_of_its_conversations() {
        let review = review();
        assert_eq!(review.mr_mark(), Some(Mark::Unresolved), "a plain comment is never resolved");
        let resolved = review.with_resolved("6a9c1750b2d6e4f0", true);
        assert_eq!(resolved.mr_mark(), Some(Mark::Resolved));
        assert_eq!(resolved.with_drafts(vec![Draft::new(None, "one more thing")]).mr_mark(), Some(Mark::Draft));
        let anchored = review.threads.iter().filter(|t| t.anchor.is_some()).cloned().collect();
        assert_eq!(Review { threads: anchored, ..review }.mr_mark(), None, "nothing on the MR");
    }

    #[test]
    fn the_pane_lists_unresolved_then_new_drafts_then_resolved() {
        let base = review().with_resolved("c0ffee00c0ffee00", false);
        let new_draft = Draft::new(Some(old_13()), "why drop the default client?");
        let review = base.with_drafts(vec![Draft::reply("c0ffee00c0ffee00", "agreed"), new_draft]);
        let place = review.place_of(&removed_line()).unwrap();
        assert_eq!(place, Place::Line { file: 0, new: None, old: Some(13) });
        let listed = review.conversations(&place);
        assert_eq!(
            listed,
            [Conversation { thread: Some("c0ffee00c0ffee00".into()), drafts: vec![0] }, Conversation { thread: None, drafts: vec![1] },]
        );
        let resolved = review.with_resolved("c0ffee00c0ffee00", true);
        assert_eq!(resolved.conversations(&place)[0].thread, None, "a resolved thread goes last");
    }

    #[test]
    fn every_conversation_lists_unresolved_then_new_drafts_then_resolved_each_in_diff_order() {
        let lock = Anchor { path: "Cargo.lock".into(), side: Side::New, line: 1 };
        let drafts = vec![
            Draft::new(Some(lock), "why a lock bump?"),
            Draft::reply("c0ffee00c0ffee00", "agreed"),
            Draft::new(Some(old_13()), "why drop the default client?"),
            Draft::new(None, "one more thing"),
        ];
        let review = review().with_resolved("c0ffee00c0ffee00", false).with_drafts(drafts);
        let listed = review.conversations(&Place::All);
        let thread = |id: &str, drafts: Vec<usize>| Conversation { thread: Some(id.into()), drafts };
        let draft = |index: usize| Conversation { thread: None, drafts: vec![index] };
        let expected = [
            thread("6a9c1750b2d6e4f0", vec![]),
            thread("c0ffee00c0ffee00", vec![1]),
            thread("9f2c0aa1d4e5b6c7", vec![]),
            draft(3),
            draft(2),
            draft(0),
        ];
        assert_eq!(listed, expected, "the MR, then charge.rs's line, then its outdated thread; the lock file last");
        let resolved = review.with_resolved("c0ffee00c0ffee00", true).conversations(&Place::All);
        assert_eq!(resolved.last(), Some(&thread("c0ffee00c0ffee00", vec![1])), "a resolved thread goes last");
    }

    #[test]
    fn i_take_part_in_a_thread_i_wrote_in_or_answered_and_in_my_new_draft() {
        let review = review().with_drafts(vec![Draft::reply("9f2c0aa1d4e5b6c7", "agreed"), Draft::new(None, "one more thing")]);
        let mine: Vec<Conversation> = review.conversations(&Place::All).into_iter().filter(|c| review.takes_part(c, "nina")).collect();
        let thread = |id: &str, drafts: Vec<usize>| Conversation { thread: Some(id.into()), drafts };
        let expected =
            [thread("9f2c0aa1d4e5b6c7", vec![0]), Conversation { thread: None, drafts: vec![1] }, thread("c0ffee00c0ffee00", vec![])];
        assert_eq!(mine, expected, "my reply, my new draft, the thread I started; not lea's comment on the MR");
        assert!(review.takes_part(&thread("6a9c1750b2d6e4f0", vec![]), "lea"));
        assert!(!review.takes_part(&thread("c0ffee00c0ffee00", vec![]), "omar"));
    }

    #[test]
    fn a_conversation_hangs_on_the_mr_a_line_or_an_outdated_line() {
        let review = review().with_drafts(vec![Draft::new(Some(old_13()), "nit")]);
        let on = |id: &str| review.spot(&Conversation { thread: Some(id.into()), drafts: vec![] });
        assert_eq!(on("6a9c1750b2d6e4f0"), Spot::Mr);
        assert_eq!(on("c0ffee00c0ffee00"), Spot::Line(&old_13()));
        assert!(matches!(on("9f2c0aa1d4e5b6c7"), Spot::Outdated(anchor) if anchor.line == 57));
        assert_eq!(review.spot(&Conversation { thread: None, drafts: vec![0] }), Spot::Line(&old_13()), "a new draft hangs on its line");
        assert_eq!(review.line_text(&old_13()), Some("    let client = Client::new();"));
        assert!(review.row_holds(&removed_line(), &old_13()));
        assert!(!review.row_holds(&Row::Line { file: 0, hunk: 0, index: 0 }, &old_13()));
    }

    #[test]
    fn an_inline_pair_is_one_place_with_both_lines() {
        let review = review();
        let pair = Row::Pair { file: 0, hunk: 0, removed: 1, added: 2 };
        assert_eq!(review.place_of(&pair), Some(Place::Line { file: 0, new: Some(13), old: Some(13) }));
        assert_eq!(review.marker_of(&review.markers(), &pair).unwrap().mark, Mark::Resolved, "the fixture's thread is resolved");
        assert_eq!(review.place_of(&Row::Header), Some(Place::Mr));
        assert_eq!(review.place_of(&Row::File { index: 0, open: true }), None);
    }

    #[test]
    fn a_note_covers_the_text_of_its_lines_on_its_side() {
        let review = review();
        let refs = review.mr.refs.clone();
        let at = |line: crate::forge::LineRef, start: Option<crate::forge::LineRef>| crate::forge::Position {
            refs: refs.clone(),
            old_path: "src/pay/charge.rs".into(),
            new_path: "src/pay/charge.rs".into(),
            line,
            start,
        };
        let new = |n| crate::forge::LineRef { old: None, new: Some(n) };
        assert_eq!(review.text_at(&at(new(13), None)), ["    let client = Client::with_key(idempotency_key(card));"]);
        assert_eq!(review.text_at(&at(new(14), Some(new(13)))).len(), 2);
        let old = crate::forge::LineRef { old: Some(13), new: None };
        assert_eq!(review.text_at(&at(old, None)), ["    let client = Client::new();"]);
    }
}
