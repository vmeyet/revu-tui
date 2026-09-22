use super::{App, Failure, Incoming, Open};
use crate::review::Review;

impl App {
    pub fn apply(&mut self, incoming: Incoming) {
        match incoming {
            Incoming::Queue { scope, .. } if scope != self.scope() => {}
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
            }
            Incoming::Review { key, review, cached } => self.apply_review(key, *review, cached),
            Incoming::Discussions { key, discussions } => {
                if let Some(open) = self.open.as_ref().filter(|o| o.key == key) {
                    self.open = Some(open.with_review(open.review.with_discussions(discussions)));
                    self.offline = None;
                    self.schedule_discussions();
                }
            }
            Incoming::Done(text) => self.toast(text),
            Incoming::DraftSaved { key, index, id } => self.apply_draft_saved(key, index, id),
            Incoming::Published { key, approved, count } => self.apply_published(key, approved, count),
            Incoming::Resolved { key, thread, resolved } => self.apply_resolved(key, thread, resolved),
            Incoming::Approved { key, approve } => {
                self.set_approved(key, approve);
                self.toast(if approve { "approved" } else { "approval removed" });
            }
            Incoming::Composed { input, text } => {
                if let Some(text) = text {
                    self.composed = self.submit(input, text);
                }
            }
            Incoming::Failed { what, message } => self.apply_failure(what, message),
        }
    }

    /// Actions an `Incoming` produced, taken by the loop right after `apply`.
    pub fn take_actions(&mut self) -> Vec<super::Action> {
        std::mem::take(&mut self.composed)
    }

    fn apply_review(&mut self, key: super::MrKey, review: Review, cached: Option<std::time::Duration>) {
        if self.opening != Some(key) && self.open.as_ref().is_none_or(|o| o.key != key) {
            return;
        }
        let next = match self.open.as_ref().filter(|o| o.key == key) {
            Some(open) => open.with_review(carry_folds(&open.review, review)),
            None => Open::new(key, review),
        };
        self.open = Some(Open { cached: cached.map(|age| (self.now, age)), ..next });
        if cached.is_none() {
            self.opening = None;
            self.offline = None;
            self.opened.insert(key, self.today);
            self.schedule_review();
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
            other => self.apply_write_failure(other, message),
        }
    }
}

/// Fresh data keeps the folds of every file that did not change, so a poll never unfolds what was read.
fn carry_folds(old: &Review, fresh: Review) -> Review {
    let mut fold = fresh.fold.clone();
    for file in &old.files {
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
    fresh.with_fold(fold).with_viewed(old.viewed.clone())
}
