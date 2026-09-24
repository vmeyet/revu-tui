use super::{App, Failure, Incoming, Open};
use crate::review::Review;

impl App {
    pub fn apply(&mut self, incoming: Incoming) {
        match incoming {
            Incoming::Queue { scope, .. } | Incoming::QueueView { scope, .. } if scope != self.scope() => {}
            Incoming::QueueView { view, .. } => {
                self.queue_view = view;
                self.queue_settle();
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
                self.offline = None;
                self.queue_settle();
                self.schedule_queue();
                self.ask_triage();
                let notice = self.notify_new();
                self.notice(notice);
                self.prefetch();
            }
            Incoming::Review { key, review, cached } => self.apply_review(key, *review, cached),
            Incoming::Resume { key, spot } => self.resume(&key, &spot),
            Incoming::Progress(counts) => {
                let open = self.open.as_ref().map(|o| (o.key.clone(), o.review.viewed.len()));
                self.viewed_counts = counts;
                self.viewed_counts.extend(open);
            }
            Incoming::Discussions { key, discussions } => {
                if let Some(open) = self.open.as_ref().filter(|o| o.key == key) {
                    let fresh = open.review.with_discussions(discussions);
                    self.news = news(&open.review, &fresh).or(self.news.take());
                    self.open = Some(open.with_review(fresh));
                    self.offline = None;
                    self.schedule_discussions();
                }
            }
            Incoming::Done(text) => self.toast(text),
            Incoming::File { key, path, text } => self.apply_file(&key, path, &text),
            Incoming::DraftSaved { key, index, id } => self.apply_draft_saved(&key, index, id),
            Incoming::Published { key, approved, count } => self.apply_published(&key, approved, count),
            Incoming::Posted { key, to } => self.apply_posted(&key, &to),
            Incoming::Resolved { key, thread, resolved } => self.apply_resolved(&key, &thread, resolved),
            Incoming::Checks { key, checks } => self.apply_checks(&key, checks),
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
            Incoming::DraftSet { key, draft } => {
                let follow = self.draft_set(&key, draft);
                self.composed.extend(follow);
            }
            Incoming::Approved { key, approve } => {
                self.set_approved(&key, approve);
                self.toast(if approve { "approved" } else { "approval removed" });
            }
            Incoming::Composed { input, text } => {
                if let Some(text) = text {
                    self.composed = self.submit(input, text);
                }
            }
            Incoming::ViewReady { key, view } => self.apply_view_ready(&key, view),
            Incoming::Viewed { view, outcome } => self.apply_viewed(&view, outcome),
            Incoming::Answer { key, id, part } => self.apply_answer(&key, id, part),
            Incoming::Triaged { key, verdict } => self.apply_triaged(key, verdict),
            Incoming::Read { key, head, reading } => self.apply_read(key, head, reading),
            Incoming::Failed { what, message } => self.apply_failure(what, message),
        }
    }

    /// Actions an `Incoming` produced, taken by the loop right after `apply`.
    pub fn take_actions(&mut self) -> Vec<super::Action> {
        std::mem::take(&mut self.composed)
    }

    fn apply_review(&mut self, key: super::MrKey, review: Review, cached: Option<std::time::Duration>) {
        if self.opening.as_ref() != Some(&key) && self.open.as_ref().is_none_or(|o| o.key != key) {
            return;
        }
        if cached.is_none() {
            self.news = self.open.as_ref().filter(|o| o.key == key).and_then(|o| news(&o.review, &review)).or(self.news.take());
        }
        let next = match self.open.as_ref().filter(|o| o.key == key) {
            Some(open) => open.with_review(carry_folds(&open.review, &review)),
            None => Open::new(key.clone(), review),
        };
        self.count_viewed(&key, &next.review);
        let fresh_open = self.open.as_ref().is_none_or(|o| o.key != key);
        self.open = Some(Open { cached: cached.map(|age| (self.now, age)), ..next });
        if fresh_open {
            self.spot_saved = self.spot().map(|spot| (key.clone(), spot));
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

    fn apply_failure(&mut self, what: Failure, message: String) {
        match what {
            Failure::Queue => {
                self.queue_loading = false;
                self.warn(format!("{message} · r to retry"));
                self.back_off();
            }
            Failure::Open => {
                self.opening = None;
                if self.open.is_some() {
                    self.offline = self.offline.or(Some(self.now));
                } else {
                    self.focus = super::Focus::Queue;
                    self.warn(format!("{message} · enter to retry"));
                }
                self.back_off();
            }
            Failure::Poll => {
                self.offline = self.offline.or(Some(self.now));
                self.back_off();
            }
            Failure::Local => self.warn(message),
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

/// Fresh data keeps the folds of every file that did not change, so a poll never unfolds what was read.
fn carry_folds(old: &Review, fresh: &Review) -> Review {
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
    fresh.with_fold(fold).with_viewed(fresh.still_viewed(&old.viewed_fingerprints()))
}
