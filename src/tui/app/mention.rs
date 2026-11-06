//! Mentions: where someone wrote to me on an MR. Its row carries `@`, MENTIONS lists the MRs no other
//! section does, `enter` lands on the note, and `x`, a reply or a resolve marks them done.
use super::{Action, App};
use crate::forge::{Mention, MentionId, MrKey, MrState, QueueMr, ThreadId};

/// The note of a mention `enter` opened an MR on, for the cursor once the MR is painted.
#[derive(Debug)]
pub(crate) struct MentionAim {
    key: MrKey,
    note: u64,
}

impl App {
    fn mentions(&self) -> &[Mention] {
        self.sections.as_ref().map_or(&[], |sections| &sections.mentions)
    }

    /// Whether the queue keeps a column for the `@` mark: only while some mention is pending.
    pub(crate) fn shows_mentions(&self) -> bool {
        !self.mentions().is_empty()
    }

    pub(crate) fn mentioned(&self, mr: &QueueMr) -> bool {
        self.mentions().iter().any(|mention| mention.is_on(mr))
    }

    /// `merged` or `closed`, for a row only its mention brings: no other section lists a finished MR.
    pub(crate) fn finished(&self, mr: &QueueMr) -> Option<MrState> {
        self.mentions().iter().find(|mention| mention.is_on(mr)).map(|mention| mention.state).filter(|state| *state != MrState::Open)
    }

    /// `x`: every mention of the MR under the cursor is done; a MENTIONS row leaves at once.
    pub(super) fn mention_done(&mut self) -> Vec<Action> {
        let Some(key) = self.selected_mr().map(QueueMr::key) else {
            self.did_nothing();
            return vec![];
        };
        let ids = self.take_mentions(&key, |_| true);
        if ids.is_empty() {
            self.toast("no mention on this MR");
            return vec![];
        }
        self.toast(match ids.len() {
            1 => "mention done".to_owned(),
            many => format!("{many} mentions done"),
        });
        self.queue_settle();
        vec![Action::MarkMentionsDone { key, ids }]
    }

    /// A reply or a resolve on `thread` answers the mentions in it.
    pub(super) fn answer_mentions(&mut self, key: &MrKey, thread: &ThreadId) {
        let Some(open) = self.open.as_ref().filter(|open| &open.key == key) else { return };
        let Some(found) = open.review.thread(thread) else { return };
        let notes: Vec<u64> = found.notes.iter().map(|note| note.id).collect();
        let ids = self.take_mentions(key, |mention| mention.note.is_some_and(|note| notes.contains(&note)));
        if !ids.is_empty() {
            self.composed.push(Action::MarkMentionsDone { key: key.clone(), ids });
        }
    }

    /// Before `enter` opens `key`: the note of its newest mention, for the cursor once the MR is painted.
    pub(super) fn aim_at_mention(&mut self, key: &MrKey) {
        let note = self.mentions().iter().find(|mention| mention.mr.key() == *key).and_then(|mention| mention.note);
        self.mention_note = note.map(|note| MentionAim { key: key.clone(), note });
    }

    /// The MR `enter` aimed at is painted: the cursor goes to the mentioned thread. A review painted
    /// from the cache may not hold the note yet; the fresh one holds it, or never will.
    pub(super) fn land_on_mention(&mut self) -> Vec<Action> {
        let Some(aim) = &self.mention_note else { return vec![] };
        let Some(open) = self.open.as_ref().filter(|open| open.key == aim.key) else { return vec![] };
        let thread =
            open.review.threads.iter().find(|thread| thread.notes.iter().any(|n| n.id == aim.note)).map(|thread| thread.id.clone());
        if thread.is_none() && open.cached.is_some() {
            return vec![];
        }
        self.mention_note = None;
        thread.map(|id| self.show_thread(&id)).unwrap_or_default()
    }

    /// Takes the mentions of `key` that `which` keeps out of the queue, and gives back their ids.
    fn take_mentions(&mut self, key: &MrKey, which: impl Fn(&Mention) -> bool) -> Vec<MentionId> {
        let Some(sections) = self.sections.as_mut() else { return vec![] };
        sections.mentions.extract_if(.., |mention| mention.mr.key() == *key && which(mention)).map(|mention| mention.id).collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::forge::{Mention, MentionId, MrState};
    use crate::tui::app::test_support::*;

    const THREAD: &str = "9f2c0aa1d4e5b6c7";

    /// A mention on the fixture's MR `number`, or on an MR no list holds when it has none.
    fn mention(id: u64, number: u64, state: MrState, note: Option<u64>) -> Mention {
        let listed = sections().all().find(|mr| mr.number == number).cloned();
        let mr = listed.unwrap_or_else(|| QueueMr {
            number,
            title: "fix: round the totals".into(),
            draft: false,
            size: None,
            ..sections().mine[0].clone()
        });
        Mention { id: MentionId(id), mr, state, note }
    }

    fn with_mentions(mentions: Vec<Mention>) -> App {
        let mut app = app();
        let sections = Sections { mentions, ..sections() };
        app.apply(Incoming::Queue { scope: None, me: "nina".into(), sections, opened: HashMap::new(), cached: false });
        app
    }

    fn row(app: &App, number: u64) -> QueueMr {
        app.queue_rows()
            .iter()
            .find_map(|row| match row {
                QueueRow::Mr(mr) if mr.number == number => Some((*mr).clone()),
                _ => None,
            })
            .unwrap()
    }

    fn marked_done(actions: &[Action]) -> Vec<u64> {
        actions
            .iter()
            .filter_map(|action| match action {
                Action::MarkMentionsDone { ids, .. } => Some(ids.iter().map(|id| id.0)),
                _ => None,
            })
            .flatten()
            .collect()
    }

    /// The MR 42 opened from the queue with `enter`, its review painted.
    fn opened_on_mention() -> App {
        let mut app = with_mentions(vec![mention(11, 42, MrState::Open, Some(502))]);
        app.queue_first();
        assert_eq!(app.handle_key(code(KeyCode::Enter)), vec![Action::Open(mr_key())]);
        app.apply(Incoming::Review { key: mr_key(), review: Box::new(review()), cached: None });
        app
    }

    #[test]
    fn mentions_holds_the_mentioned_mrs_no_other_section_lists() {
        let app = with_mentions(vec![mention(11, 42, MrState::Open, Some(502)), mention(12, 77, MrState::Merged, None)]);
        assert_eq!(shown(&app), ["TO REVIEW", "42", "MINE", "41", "MENTIONS", "77", "WATCHING", "35", "DONE"]);
        assert!(app.mentioned(&row(&app, 42)) && !app.mentioned(&row(&app, 41)), "a listed MR keeps its section, marked");
        assert_eq!(app.finished(&row(&app, 77)), Some(MrState::Merged));
        assert_eq!(app.finished(&row(&app, 42)), None);
    }

    #[test]
    fn without_a_mention_the_queue_has_no_mentions_section_nor_column() {
        let app = with_mentions(vec![]);
        assert!(!shown(&app).contains(&"MENTIONS".to_owned()));
        assert!(!app.shows_mentions());
    }

    #[test]
    fn x_marks_the_mentions_of_the_mr_done_and_a_second_x_changes_nothing() {
        let mut app = with_mentions(vec![mention(11, 42, MrState::Open, Some(502)), mention(12, 42, MrState::Open, None)]);
        app.queue_first();
        assert_eq!(marked_done(&press(&mut app, "x")), [11, 12]);
        assert_eq!(app.live_toast().unwrap().text, "2 mentions done");
        assert!(!app.shows_mentions());
        assert_eq!(press(&mut app, "x"), vec![]);
        assert_eq!(app.live_toast().unwrap().text, "no mention on this MR");
    }

    #[test]
    fn x_on_a_mentions_row_takes_it_out_of_the_queue() {
        let mut app = with_mentions(vec![mention(12, 77, MrState::Closed, None)]);
        app.queue_selected = app.queue_rows().iter().position(|row| matches!(row, QueueRow::Mr(mr) if mr.number == 77)).unwrap();
        assert_eq!(marked_done(&press(&mut app, "x")), [12]);
        assert!(!shown(&app).contains(&"77".to_owned()));
    }

    #[test]
    fn enter_on_a_mentioned_mr_lands_on_the_thread_of_the_note() {
        let app = opened_on_mention();
        assert_eq!(app.open.as_ref().unwrap().focused_thread(), Some(THREAD.into()));
        assert!(app.mention_note.is_none(), "it lands once");
    }

    #[test]
    fn a_cached_review_without_the_note_waits_for_the_fresh_one() {
        let mut app = with_mentions(vec![mention(11, 42, MrState::Open, Some(502))]);
        app.queue_first();
        app.handle_key(code(KeyCode::Enter));
        let bare = Review::new(mr(), &diffs(), vec![], &[]);
        app.apply(Incoming::Review { key: mr_key(), review: Box::new(bare), cached: Some(Duration::from_secs(60)) });
        assert!(app.mention_note.is_some());
        app.apply(Incoming::Review { key: mr_key(), review: Box::new(review()), cached: None });
        assert_eq!(app.open.as_ref().unwrap().focused_thread(), Some(THREAD.into()));
    }

    #[test]
    fn a_reply_posted_on_the_mentioned_thread_marks_the_mention_done() {
        let mut app = opened_on_mention();
        app.take_actions();
        app.apply(Incoming::Posted { key: mr_key(), to: Post::Reply("6a9c1750b2d6e4f0".into()) });
        assert_eq!(marked_done(&app.take_actions()), [] as [u64; 0], "another thread answers nothing");
        app.apply(Incoming::Posted { key: mr_key(), to: Post::Reply(THREAD.into()) });
        assert_eq!(marked_done(&app.take_actions()), [11]);
    }

    #[test]
    fn resolving_the_mentioned_thread_marks_the_mention_done() {
        let mut app = opened_on_mention();
        app.take_actions();
        app.apply(Incoming::Resolved { key: mr_key(), thread: THREAD.into(), resolved: false });
        assert_eq!(marked_done(&app.take_actions()), [] as [u64; 0], "opening a thread again answers nothing");
        app.apply(Incoming::Resolved { key: mr_key(), thread: THREAD.into(), resolved: true });
        assert_eq!(marked_done(&app.take_actions()), [11]);
    }

    #[test]
    fn snapshot_queue_with_mentions() {
        let mut app = with_mentions(vec![mention(11, 42, MrState::Open, Some(502)), mention(12, 77, MrState::Merged, None)]);
        app.now = app.started;
        insta::assert_snapshot!("queue_mentions", render(&mut app, 100, 18));
    }
}
