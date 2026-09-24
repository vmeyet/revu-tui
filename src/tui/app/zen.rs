//! Zen, `zz`: the diff alone in a calm centred column. `←` `→` walk the queue's MRs in the order
//! it shows them without leaving zen; notifications wait until zen ends.
use super::queue::QueueRow;
use super::{Action, App, Focus, MrKey};
use std::time::Instant;

impl App {
    /// `zz`: into zen on an open MR, or back out of it.
    pub(super) fn toggle_zen(&mut self) -> Vec<Action> {
        if self.zen {
            return self.leave_zen();
        }
        if self.open.is_none() {
            self.toast("open an MR first");
            return vec![];
        }
        self.zen = true;
        self.focus = Focus::Review;
        vec![]
    }

    /// Out of zen: the queue and the frames come back, and what arrived meanwhile is sent.
    pub(super) fn leave_zen(&mut self) -> Vec<Action> {
        self.zen = false;
        self.zen_switch = None;
        std::mem::take(&mut self.quiet_notices)
    }

    /// Zen with the diff focused: where `←` `→` walk the MRs.
    pub(super) fn in_zen_diff(&self) -> bool {
        self.zen && self.focus == Focus::Review
    }

    /// A notification, sent now or kept until zen ends.
    pub(super) fn notice(&mut self, action: Option<Action>) {
        match (action, self.zen) {
            (Some(action), true) => self.quiet_notices.push(action),
            (action, false) => self.composed.extend(action),
            (None, true) => {}
        }
    }

    /// `→` `←` in zen: the next or previous MR the queue shows, opened without leaving zen.
    pub(super) fn zen_step(&mut self, forward: bool) -> Vec<Action> {
        let order = self.zen_order();
        let current = self.opening.clone().or_else(|| self.open.as_ref().map(|o| o.key.clone()));
        let at = current.and_then(|key| order.iter().position(|k| *k == key));
        let next = match (at, forward) {
            (None, _) => 0,
            (Some(i), true) => i + 1,
            (Some(i), false) => i.wrapping_sub(1),
        };
        let Some(key) = order.get(next).cloned() else {
            self.toast(if forward { "the last MR of the queue" } else { "the first MR of the queue" });
            return vec![];
        };
        self.select_in_queue(&key);
        let label =
            self.queue_mr(&key).map_or_else(String::new, |mr| format!("{}{} {}", self.hosts.kind_of(&key).sigil(), mr.number, mr.title));
        self.zen_switch = Some((self.now, format!("‹  {label}  ·  {}/{}  ›", next + 1, order.len())));
        let actions = self.open_key(key);
        self.focus = Focus::Review;
        actions
    }

    /// The MRs the queue shows, top to bottom: a folded stack counts as its MRs, a folded
    /// section as none, each MR once.
    pub(super) fn zen_order(&self) -> Vec<MrKey> {
        let mut keys: Vec<MrKey> = vec![];
        for row in self.queue_rows() {
            let mrs = match row {
                QueueRow::Mr(mr) | QueueRow::Stacked(mr) => vec![mr],
                QueueRow::Stack { mrs, open: false, .. } => mrs,
                QueueRow::Stack { open: true, .. } | QueueRow::Section { .. } | QueueRow::Author { .. } => vec![],
            };
            for mr in mrs {
                let key = mr.key();
                if !keys.contains(&key) {
                    keys.push(key);
                }
            }
        }
        keys
    }

    /// The zen switch line, while it is fresh.
    pub fn zen_banner(&self, now: Instant) -> Option<&str> {
        let (since, text) = self.zen_switch.as_ref()?;
        (now.duration_since(*since) < ZEN_BANNER).then_some(text.as_str())
    }

    /// The queue cursor on `key`'s row, or on the folded stack holding it, so leaving zen shows it.
    fn select_in_queue(&mut self, key: &MrKey) {
        let at = self.queue_rows().iter().position(|row| match row {
            QueueRow::Mr(mr) | QueueRow::Stacked(mr) => mr.key() == *key,
            QueueRow::Stack { mrs, open: false, .. } => mrs.iter().any(|mr| mr.key() == *key),
            _ => false,
        });
        if let Some(at) = at {
            self.queue_selected = at;
        }
    }

    fn queue_mr(&self, key: &MrKey) -> Option<&crate::forge::QueueMr> {
        self.sections.as_ref()?.all().find(|mr| mr.key() == *key)
    }
}

/// How long the zen switch line stays on top.
const ZEN_BANNER: std::time::Duration = std::time::Duration::from_millis(1_500);
