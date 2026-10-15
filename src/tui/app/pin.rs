//! Pins: MRs the reader keeps at the top of the queue, in PINNED and out of their usual section,
//! until the forge says they are merged or closed.
use super::{Action, App, Focus};
use crate::forge::{MrKey, QueueMr};

impl App {
    /// `b`: pins the MR under the cursor, or unpins it.
    pub(super) fn toggle_pin(&mut self) -> Vec<Action> {
        let Some(key) = self.pin_target() else { return vec![] };
        let on = !self.pins.contains(&key);
        self.set_pin(&key, on)
    }

    /// `:pin`, `:unpin`.
    pub(super) fn pin_command(&mut self, on: bool) -> Vec<Action> {
        let Some(key) = self.pin_target() else { return vec![] };
        self.set_pin(&key, on)
    }

    /// The MR under the queue's cursor, or the open one outside the queue.
    fn pin_target(&mut self) -> Option<MrKey> {
        let target = match self.focus {
            Focus::Queue => self.selected_mr().map(QueueMr::key),
            Focus::Review | Focus::Side => self.open.as_ref().map(|open| open.key.clone()),
        };
        if target.is_none() {
            self.toast("no MR under the cursor");
        }
        target
    }

    fn set_pin(&mut self, key: &MrKey, on: bool) -> Vec<Action> {
        self.toast(format!("{} {}{}", if on { "pinned" } else { "unpinned" }, self.hosts.kind_of(key).sigil(), key.number));
        let changed = if on { self.pins.insert(key.clone()) } else { self.pins.remove(key) };
        if !changed {
            return vec![];
        }
        self.select_in_queue(key);
        self.queue_settle();
        vec![self.save_pin(key, on)]
    }

    /// The queue found these pins merged or closed: they go, and the cache forgets them.
    pub(super) fn drop_pins(&mut self, gone: &[MrKey]) -> Vec<Action> {
        let dropped: Vec<MrKey> = gone.iter().filter(|key| self.pins.remove(key)).cloned().collect();
        if dropped.is_empty() {
            return vec![];
        }
        self.queue_settle();
        dropped.iter().map(|key| self.save_pin(key, false)).collect()
    }

    /// Asked for every row on every draw: compared field by field, so no key is built.
    pub(super) fn is_pinned(&self, mr: &QueueMr) -> bool {
        self.pins.iter().any(|key| key.number == mr.number && key.project == mr.project && key.host == mr.host)
    }

    fn save_pin(&mut self, key: &MrKey, on: bool) -> Action {
        self.next_save += 1;
        Action::ChangePin { scope: self.scope(), key: key.clone(), on, order: self.next_save }
    }
}

#[cfg(test)]
mod tests {
    use crate::tui::app::test_support::*;

    fn pins(numbers: &[u64]) -> BTreeSet<MrKey> {
        numbers.iter().map(|n| MrKey::new("acme/widgets", *n)).collect()
    }

    /// The pin changes `actions` save, without their order.
    fn saved(actions: &[Action]) -> Vec<(u64, bool)> {
        actions
            .iter()
            .filter_map(|action| match action {
                Action::ChangePin { scope: None, key, on, .. } => Some((key.number, *on)),
                _ => None,
            })
            .collect()
    }

    /// The queue as section names and MR numbers, top to bottom.
    fn shown(app: &App) -> Vec<String> {
        app.queue_rows()
            .iter()
            .filter_map(|row| match row {
                QueueRow::Section { name, .. } => Some((*name).to_owned()),
                QueueRow::Mr(mr) => Some(mr.number.to_string()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn b_pins_the_mr_under_the_cursor_on_top_and_b_again_puts_it_back() {
        let mut app = with_queue();
        assert_eq!(shown(&app), ["TO REVIEW", "42", "MINE", "41", "WATCHING", "35", "DONE"]);
        press(&mut app, "jj");
        assert_eq!(saved(&press(&mut app, "b")), [(35, true)]);
        assert_eq!(app.live_toast().unwrap().text, "pinned !35");
        assert_eq!(shown(&app), ["PINNED", "35", "TO REVIEW", "42", "MINE", "41", "WATCHING", "DONE"], "it leaves Watching");
        assert_eq!(app.selected_mr().unwrap().number, 35, "the cursor follows it");
        assert_eq!(saved(&press(&mut app, "b")), [(35, false)]);
        assert_eq!(app.live_toast().unwrap().text, "unpinned !35");
        assert_eq!(shown(&app), ["TO REVIEW", "42", "MINE", "41", "WATCHING", "35", "DONE"], "an empty PINNED hides");
    }

    #[test]
    fn two_quick_pin_changes_are_counted_in_the_order_they_were_made() {
        let mut app = with_queue();
        press(&mut app, "jj");
        let actions = [press(&mut app, "b"), press(&mut app, "b")].concat();
        let orders: Vec<u64> = actions
            .iter()
            .filter_map(|action| match action {
                Action::ChangePin { order, .. } => Some(*order),
                _ => None,
            })
            .collect();
        let [pinned, unpinned] = orders.as_slice() else { panic!("{actions:?}") };
        assert!(pinned < unpinned, "{orders:?}");
    }

    #[test]
    fn colon_pin_twice_changes_nothing_the_second_time() {
        let mut app = with_queue();
        let mut run = |line: &str| {
            press(&mut app, line);
            app.handle_key(code(KeyCode::Enter))
        };
        assert_eq!(saved(&run(":pin")), [(42, true)]);
        assert_eq!(run(":pin"), vec![]);
        assert_eq!(saved(&run(":unpin")), [(42, false)]);
        assert_eq!(run(":unpin"), vec![]);
    }

    #[test]
    fn colon_pin_in_the_review_pins_the_open_mr() {
        let mut app = with_review();
        press(&mut app, ":pin");
        assert_eq!(saved(&app.handle_key(code(KeyCode::Enter))), [(42, true)]);
    }

    #[test]
    fn a_pinned_mr_no_list_holds_shows_only_while_pinned() {
        let mut app = app();
        let mut sections = sections();
        sections.pinned = vec![QueueMr { number: 7, ..sections.mine[0].clone() }];
        app.apply(Incoming::Queue { scope: None, me: "nina".into(), sections, opened: HashMap::new(), cached: false });
        assert!(!shown(&app).contains(&"7".to_owned()), "unpinned meanwhile: it shows no more");
        app.apply(Incoming::Pins { scope: None, pins: pins(&[7, 42]) });
        assert_eq!(shown(&app)[..4], ["PINNED", "42", "7", "TO REVIEW"]);
        assert_eq!(shown(&app).iter().filter(|row| *row == "42").count(), 1, "never twice");
        assert_eq!(app.zen_order()[..2], [MrKey::new("acme/widgets", 42), MrKey::new("acme/widgets", 7)], "]m starts with the pins");
    }

    #[test]
    fn pins_the_forge_says_are_gone_drop_once() {
        let mut app = with_queue();
        let gone = || vec![MrKey::new("acme/widgets", 42)];
        app.apply(Incoming::Pins { scope: None, pins: pins(&[41, 42]) });
        app.apply(Incoming::PinsGone { scope: Some("acme/other".into()), gone: gone() });
        assert_eq!(app.take_actions(), vec![], "another scope's answer");
        app.apply(Incoming::PinsGone { scope: None, gone: gone() });
        assert_eq!(saved(&app.take_actions()), [(42, false)], "only the gone pin is unpinned, 41 stays");
        app.apply(Incoming::PinsGone { scope: None, gone: gone() });
        assert_eq!(app.take_actions(), vec![], "an already dropped pin changes nothing");
    }

    #[test]
    fn snapshot_queue_with_a_pin() {
        let mut app = with_queue();
        app.apply(Incoming::Pins { scope: None, pins: [MrKey::new("acme/infra", 35)].into() });
        app.now = app.started;
        insta::assert_snapshot!("queue_pinned", render(&mut app, 100, 16));
    }
}
