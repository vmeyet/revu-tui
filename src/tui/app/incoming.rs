use super::{App, Failure, Incoming, Open};
use crate::forge::{ApprovalChange, Discussion};
use crate::review::{Draft, Review, draft};

impl App {
    pub(crate) fn apply(&mut self, incoming: Incoming) {
        match incoming {
            Incoming::Queue { scope, .. }
            | Incoming::QueueView { scope, .. }
            | Incoming::Pins { scope, .. }
            | Incoming::PinsGone { scope, .. }
                if scope != self.scope() => {}
            Incoming::QueueView { view, .. } => {
                self.queue_view = view;
                self.queue_settle();
            }
            Incoming::Pins { pins, .. } => {
                self.pins = pins;
                self.queue_settle();
            }
            Incoming::PinsGone { gone, .. } => {
                let follow = self.drop_pins(&gone);
                self.composed.extend(follow);
            }
            Incoming::Queue { ref me, .. } if self.me.is_empty() && !me.is_empty() => {
                self.me.clone_from(me);
                self.apply(incoming);
            }
            Incoming::Queue { sections, opened, cached: true, .. } => {
                if self.sections.is_none() {
                    self.sections = Some(sections);
                    self.opened = opened;
                    self.queue_settle();
                }
            }
            Incoming::Queue { sections, opened, .. } => {
                self.sections = Some(sections);
                self.opened = opened;
                self.queue_loading = false;
                self.queue_failed = false;
                self.offline = None;
                self.queue_settle();
                self.schedule_queue();
                self.ask_triage();
                let notice = self.notify_new();
                self.notice(notice);
                self.prefetch();
            }
            Incoming::Review { key, review, cached } => self.apply_review(key, *review, cached),
            Incoming::Threads { key, mr, discussions, drafts } => self.apply_threads(key, *mr, discussions, drafts),
            Incoming::Mr { key, mr } => self.apply_mr(&key, *mr),
            Incoming::Resume { key, spot } => self.resume(&key, &spot),
            Incoming::Progress(counts) => {
                let open = self.open.as_ref().map(|o| (o.key.clone(), o.review.progress()));
                self.progress = counts;
                self.progress.extend(open);
            }
            Incoming::Discussions { key, discussions } => {
                if let Some(open) = self.open.take_if(|o| o.key == key) {
                    let fresh = open.review.clone().with_discussions(discussions);
                    let moved = !std::sync::Arc::ptr_eq(&fresh.threads, &open.review.threads);
                    self.news = news(&open.review, &fresh).or(self.news.take());
                    self.open = Some(if moved { open.with_review(fresh) } else { open });
                    self.offline = None;
                    self.schedule_discussions();
                }
            }
            Incoming::Done(text) => self.toast(text),
            Incoming::File { key, path, sha, text } => self.apply_file(&key, path, &sha, &text),
            Incoming::DraftSaved { key, draft, id, body } => {
                let follow = self.apply_draft_saved(&key, &draft, id, &body);
                self.composed.extend(follow);
            }
            Incoming::DraftChanged { key } => self.draft_write_answered(&key),
            Incoming::Published { key, approval, count } => self.apply_published(&key, &approval, count),
            Incoming::Posted { key, to } => self.apply_posted(&key, &to),
            Incoming::Resolved { key, thread, resolved } => self.apply_resolved(&key, &thread, resolved),
            Incoming::Checks { key, checks } => self.apply_checks(&key, checks),
            Incoming::Outline { key, reading } => self.apply_outline(&key, reading),
            Incoming::Prose { key, sides, versions } => self.apply_prose(&key, &sides, versions),
            Incoming::PastAnswers { key, answers } => self.apply_past_answers(&key, answers),
            Incoming::Deployments { key, deployments } => {
                self.update_open_of(&key, |open| Open { deployments: Some(deployments), ..open });
            }
            Incoming::Image { url, image } => self.thumbs.arrived(&url, image),
            Incoming::Applied { key, branch } => {
                if self.open.as_ref().is_some_and(|o| o.key == key) {
                    let follow = self.applied(&branch);
                    self.composed.extend(follow);
                }
            }
            Incoming::Merged { key } => {
                let follow = self.merged(&key);
                self.composed.extend(follow);
            }
            Incoming::ReadinessSet { key, readiness } => {
                let follow = self.readiness_set(&key, readiness);
                self.composed.extend(follow);
            }
            Incoming::Approved { key, change } => {
                self.set_approved(&key, change);
                self.toast(match change {
                    ApprovalChange::Approve => "approved",
                    ApprovalChange::Unapprove => "approval removed",
                });
                let follow = self.reread(&key);
                self.composed.extend(follow);
            }
            Incoming::Composed { input, edited } => self.apply_composed(input, edited),
            Incoming::ViewReady { key, view } => self.apply_view_ready(&key, view),
            Incoming::Viewed { view, outcome } => self.apply_viewed(&view, outcome),
            Incoming::SessionPick { mr, sessions } => self.offer_sessions(*mr, sessions),
            Incoming::SessionReady(handoff) => self.handoff = Some(handoff),
            Incoming::SessionLeft { said, outcome } => self.apply_session_left(&said, outcome),
            Incoming::Answer { key, id, part } => self.apply_answer(&key, id, part),
            Incoming::Triaged { key, verdict } => self.apply_triaged(key, verdict),
            Incoming::Read { key, head, reading } => self.apply_read(key, head, reading),
            Incoming::Failed { what, message } => self.apply_failure(what, message),
        }
    }

    /// Every answer that is waiting applied in one go, so the loop draws once per batch and not once per piece of a stream.
    pub(crate) fn apply_all(&mut self, waiting: impl IntoIterator<Item = Incoming>) -> Vec<super::Action> {
        waiting
            .into_iter()
            .flat_map(|incoming| {
                self.apply(incoming);
                self.take_actions()
            })
            .collect()
    }

    /// Actions an `Incoming` produced, taken by the loop right after `apply`.
    pub(crate) fn take_actions(&mut self) -> Vec<super::Action> {
        let actions = std::mem::take(&mut self.composed);
        self.sending(actions)
    }

    fn apply_review(&mut self, key: super::MrKey, review: Review, cached: Option<std::time::Duration>) {
        if self.opening.as_ref() != Some(&key) && self.open.as_ref().is_none_or(|o| o.key != key) {
            return;
        }
        if cached.is_none() {
            self.news = self.open.as_ref().filter(|o| o.key == key).and_then(|o| news(&o.review, &review)).or(self.news.take());
            self.drafts_read(&key);
        }
        let current = self.open.take_if(|o| o.key == key);
        let fresh_open = current.is_none();
        let pushed = current.as_ref().is_some_and(|o| o.review.mr.refs.head != review.mr.refs.head);
        let next = match current {
            Some(open) => {
                let carried = carry_over(&open.review, review);
                open.with_review(carried)
            }
            None => Open::new(key.clone(), review),
        };
        self.count_viewed(&key, &next.review);
        self.open = Some(Open { cached: cached.map(|age| (self.now, age)), ..next });
        if fresh_open {
            self.spot_saved = self.spot().map(|spot| (key.clone(), spot));
        }
        if cached.is_none() && (pushed || self.open.as_ref().is_some_and(|o| o.deployments.is_none())) {
            self.ask_deployments();
        }
        if cached.is_none() {
            self.opening = None;
            self.offline = None;
            self.opened.insert(key, self.today);
            self.schedule_review();
            self.ask_reading();
            if self.prefetch_due {
                self.prefetch();
            }
        }
    }

    /// The painted review with the fresh MR, threads and drafts, arriving as a fresh review would: the files stay the
    /// same shared slice, so their colours and every layout built on them are kept.
    fn apply_threads(&mut self, key: super::MrKey, mr: crate::forge::Mr, discussions: Vec<Discussion>, drafts: Vec<Draft>) {
        let Some(open) = self.open.as_ref().filter(|o| o.key == key && o.review.mr.refs.head == mr.refs.head) else { return };
        let review = open.review.clone().with_mr(mr).with_discussions(discussions).with_drafts(drafts);
        self.apply_review(key, review, None);
    }

    /// A poll found the head unchanged: the MR's own fields change, the diff and the reader's place stay.
    fn apply_mr(&mut self, key: &super::MrKey, mr: crate::forge::Mr) {
        let Some(open) = self.open.take_if(|o| o.key == *key) else { return };
        let same_head = open.review.mr.refs.head == mr.refs.head;
        self.open = Some(if same_head { Open { review: open.review.with_mr(mr), cached: None, ..open } } else { open });
        self.offline = None;
        self.schedule_review();
    }

    fn apply_failure(&mut self, what: Failure, message: String) {
        match what {
            Failure::Open { key } if self.opening.as_ref() != Some(&key) => {}
            Failure::Poll { key } if self.open.as_ref().is_none_or(|o| o.key != key) => {}
            Failure::Queue => {
                self.queue_loading = false;
                self.queue_failed = true;
                self.warn(format!("{message} · r to retry"));
                self.back_off();
            }
            Failure::Open { .. } => {
                self.opening = None;
                if self.open.is_some() {
                    self.offline = self.offline.or(Some(self.now));
                } else {
                    let held = self.leave_zen();
                    self.composed.extend(held);
                    self.focus = super::Focus::Queue;
                    self.warn(format!("{message} · enter to retry"));
                }
                self.back_off();
            }
            Failure::Poll { .. } => {
                self.offline = self.offline.or(Some(self.now));
                self.back_off();
            }
            Failure::Local => self.warn(message),
            Failure::Host => self.toast(message),
            Failure::Triage => self.triage_failed(&message),
            other => self.apply_write_failure(other, message),
        }
    }
}

/// What a poll brought to the open MR, in the words of the status line; nothing when nothing moved.
fn news(old: &Review, fresh: &Review) -> Option<String> {
    if old.mr.refs.head != fresh.mr.refs.head {
        return Some("● new commits".into());
    }
    let new_notes = fresh.note_count().saturating_sub(old.note_count());
    (new_notes > 0).then(|| format!("● {new_notes} new note{}", if new_notes == 1 { "" } else { "s" }))
}

/// Fresh data keeps the folds of every file that did not change, so a poll never unfolds what was read,
/// and the drafts the forge does not hold yet, so a poll never loses one.
fn carry_over(old: &Review, fresh: Review) -> Review {
    let mut fold = fresh.fold.clone();
    for file in old.files.iter() {
        let unchanged = fresh.files.iter().any(|f| f.new_path == file.new_path && f.hunks == file.hunks);
        if !unchanged {
            continue;
        }
        if let Some(state) = old.fold.files.get(&file.new_path) {
            fold.files.insert(file.new_path.clone(), *state);
        }
        if let Some(hunks) = old.fold.hunks.get(&file.new_path) {
            fold.hunks.insert(file.new_path.clone(), hunks.clone());
        }
    }
    let viewed = fresh.still_viewed(&old.viewed_fingerprints());
    let drafts = draft::carry(&fresh.drafts, &old.drafts);
    fresh.with_fold(fold).with_viewed(viewed).with_drafts(drafts).with_colours_kept(&old.files)
}

#[cfg(test)]
mod tests {
    use crate::tui::app::test_support::*;

    #[test]
    fn enter_opens_the_mr_and_the_review_arrives() {
        let app = with_review();
        assert_eq!(app.opening, None);
        assert_eq!(app.focus, Focus::Review);
        let open = app.open.as_ref().unwrap();
        assert_eq!(open.key, mr_key());
        assert_eq!(open.row(), Some(&Row::File { index: 0, open: true }), "the cursor starts on the first file");
        assert_eq!(app.opened.get(&mr_key()), Some(&today()));
        assert!(app.poll.mr_due.is_some() && app.poll.discussions_due.is_some());
    }

    #[test]
    fn a_poll_that_brings_nothing_new_keeps_the_threads_so_nothing_is_laid_out_again() {
        let mut app = with_review();
        let threads = |app: &App| Arc::clone(&app.open.as_ref().unwrap().review.threads);
        let before = threads(&app);
        app.apply(Incoming::Discussions { key: mr_key(), discussions: discussions() });
        assert!(Arc::ptr_eq(&threads(&app), &before), "the same threads, kept by identity");
        assert!(app.poll.discussions_due.is_some(), "the next poll is set");
        let more: Vec<Discussion> = discussions().into_iter().chain([thread_on_line_13("new", 1)]).collect();
        app.apply(Incoming::Discussions { key: mr_key(), discussions: more });
        assert!(!Arc::ptr_eq(&threads(&app), &before), "a new thread replaces them");
        assert_eq!(app.open.as_ref().unwrap().review.threads.len(), before.len() + 1);
    }

    #[test]
    fn opening_over_a_current_cache_keeps_the_painted_diff_and_takes_the_fresh_threads() {
        let mut app = with_queue();
        app.handle_key(code(KeyCode::Enter));
        app.apply(Incoming::Review { key: mr_key(), review: Box::new(review()), cached: Some(Duration::from_secs(120)) });
        let painted = Arc::clone(&app.open.as_ref().unwrap().review.files);
        let more: Vec<Discussion> = discussions().into_iter().chain([thread_on_line_13("new", 1)]).collect();
        app.apply(Incoming::Threads { key: mr_key(), mr: Box::new(mr()), discussions: more, drafts: vec![] });
        let open = app.open.as_ref().unwrap();
        assert!(Arc::ptr_eq(&open.review.files, &painted), "the diff is the one painted");
        assert_eq!(open.review.threads.len(), review().threads.len() + 1);
        assert_eq!(open.staleness(app.now), None);
        assert_eq!(app.opening, None);
        assert!(app.poll.mr_due.is_some() && app.poll.discussions_due.is_some());
    }

    #[test]
    fn a_cached_review_paints_first_and_the_fresh_one_clears_the_age() {
        let mut app = with_queue();
        app.handle_key(code(KeyCode::Enter));
        app.apply(Incoming::Review { key: mr_key(), review: Box::new(review()), cached: Some(Duration::from_secs(120)) });
        assert!(app.opening.is_some(), "still fetching");
        assert_eq!(app.open.as_ref().unwrap().staleness(app.now), Some(Duration::from_secs(120)));
        app.apply(Incoming::Failed { what: Failure::Open { key: mr_key() }, message: "offline".into() });
        assert!(app.offline.is_some() && app.open.is_some(), "the cached view stays");
        app.apply(Incoming::Review { key: mr_key(), review: Box::new(review()), cached: None });
        assert_eq!(app.open.as_ref().unwrap().staleness(app.now), None);
        assert_eq!(app.offline, None);
    }

    #[test]
    fn a_late_open_failure_of_the_mr_left_behind_spares_the_one_opening() {
        let mut app = with_queue();
        let left = app.selected_mr().unwrap().key();
        app.handle_key(code(KeyCode::Enter));
        app.handle_key(code(KeyCode::Esc));
        press(&mut app, "j");
        app.handle_key(code(KeyCode::Enter));
        let wanted = app.selected_mr().unwrap().key();
        assert_eq!(app.opening.as_ref(), Some(&wanted), "the keys leave a pending open for another");
        app.apply(Incoming::Failed { what: Failure::Open { key: left }, message: "HTTP 502".into() });
        assert_eq!((app.opening.as_ref(), app.focus, app.offline), (Some(&wanted), Focus::Review, None));
        assert!(app.live_toast().is_none());
        app.apply(Incoming::Review { key: wanted.clone(), review: Box::new(review()), cached: None });
        assert_eq!(app.open.as_ref().map(|open| &open.key), Some(&wanted));
    }

    #[test]
    fn a_late_poll_failure_of_another_mr_leaves_the_open_one_online() {
        let mut app = with_review();
        let poll = app.poll.clone();
        app.apply(Incoming::Failed { what: Failure::Poll { key: MrKey::new("acme/widgets", 99) }, message: "offline".into() });
        assert_eq!((app.offline, &app.poll), (None, &poll));
        app.apply(Incoming::Failed { what: Failure::Poll { key: mr_key() }, message: "offline".into() });
        assert!(app.offline.is_some());
    }

    #[test]
    fn a_review_for_another_mr_is_ignored() {
        let mut app = with_review();
        app.apply(Incoming::Review { key: MrKey::new("acme/widgets", 99), review: Box::new(review()), cached: None });
        assert_eq!(app.open.as_ref().unwrap().key, mr_key());
    }

    #[test]
    fn answers_for_another_mr_leave_the_open_one_as_it_was() {
        let mut app = with_review();
        let before = app.open.clone();
        let other = MrKey::new("acme/widgets", 99);
        let head = before.as_ref().unwrap().review.mr.refs.head.clone();
        app.apply(Incoming::Deployments { key: other.clone(), deployments: vec![] });
        app.apply(Incoming::File { key: other.clone(), path: "src/pay/charge.rs".into(), sha: head, text: "fn main() {}".into() });
        app.apply(Incoming::Checks { key: other.clone(), checks: None });
        app.apply(Incoming::Resolved { key: other.clone(), thread: "c0ffee00c0ffee00".into(), resolved: true });
        app.apply(Incoming::Approved { key: other.clone(), change: ApprovalChange::Approve });
        app.apply(Incoming::Published { key: other, approval: Approval::NotAsked, count: 1 });
        assert_eq!(app.open, before);
    }

    #[test]
    fn fresh_discussions_replace_the_threads_and_reschedule() {
        let mut app = with_review();
        app.poll.discussions_due = None;
        app.apply(Incoming::Discussions {
            key: mr_key(),
            discussions: vec![fixture::discussion(include_str!("../../forge/gitlab/fixtures/diff_note.json"))],
        });
        assert_eq!(app.open.as_ref().unwrap().review.threads.len(), 1);
        assert!(app.poll.discussions_due.is_some());
    }

    #[test]
    fn a_fresh_review_keeps_the_folds_of_unchanged_files() {
        let mut app = with_review();
        press(&mut app, "za");
        app.apply(Incoming::Review { key: mr_key(), review: Box::new(review()), cached: None });
        assert!(!app.open.as_ref().unwrap().review.fold.file_is_open("src/pay/charge.rs"));
    }

    #[test]
    fn a_poll_at_the_same_head_changes_the_mr_and_keeps_the_diff_and_the_cursor() {
        let mut app = with_review();
        press(&mut app, "]cj");
        let before = app.open.clone().unwrap();
        let renamed = Mr { title: "feat: charge cards twice".into(), ..mr() };
        app.apply(Incoming::Mr { key: mr_key(), mr: Box::new(renamed) });
        let open = app.open.clone().unwrap();
        assert_eq!(open.review.mr.title, "feat: charge cards twice");
        assert_eq!((open.selected, open.rows), (before.selected, before.rows));
        let pushed = Mr { title: "pushed".into(), refs: crate::forge::Refs { head: "cccc".into(), ..mr().refs }, ..mr() };
        app.apply(Incoming::Mr { key: mr_key(), mr: Box::new(pushed) });
        assert_eq!(app.open.as_ref().unwrap().review.mr.title, "feat: charge cards twice", "a moved head waits for the whole review");
        assert!(app.poll.mr_due.is_some(), "polling goes on either way");
    }

    #[test]
    fn queue_failures_toast_and_stop_the_spinner() {
        let mut app = app();
        app.apply(Incoming::Failed { what: Failure::Queue, message: "HTTP 401".into() });
        assert!(!app.queue_loading);
        let toast = app.live_toast().unwrap();
        assert!(toast.danger && toast.text.contains("r to retry"));
        app.now += Duration::from_secs(5);
        assert!(app.live_toast().is_none(), "toasts age out");
    }

    #[test]
    fn a_queue_failure_keeps_the_queue_already_shown() {
        let mut app = with_queue();
        app.apply(Incoming::Failed { what: Failure::Queue, message: "HTTP 502".into() });
        assert!(!render(&mut app, 100, 16).contains("did not load"));
    }

    #[test]
    fn a_poll_that_brings_notes_says_so_until_the_next_key() {
        let mut app = with_review();
        let mut more = discussions();
        let extra = more[1].notes[0].clone();
        more[1].notes.push(crate::forge::Note { id: 999, ..extra });
        app.apply(Incoming::Discussions { key: mr_key(), discussions: more });
        assert_eq!(app.news.as_deref(), Some("● 1 new note"));
        assert!(render(&mut app, 120, 20).contains("● 1 new note"));
        press(&mut app, "j");
        assert_eq!(app.news, None);
    }
}
