//! Zen, `zz`: the diff alone in a calm centred column. `[m` `]m` walk the queue's MRs in the order
//! it shows them, in zen or not; notifications wait until zen ends.
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

    /// Out of zen: the queue and the frames come back, its cursor on the open MR, and what arrived
    /// meanwhile is sent.
    pub(super) fn leave_zen(&mut self) -> Vec<Action> {
        if let Some(key) = self.open.as_ref().map(|o| o.key.clone()) {
            self.select_in_queue(&key);
        }
        self.zen = false;
        self.zen_switch = None;
        std::mem::take(&mut self.quiet_notices)
    }

    /// A notification, sent now or kept until zen ends.
    pub(super) fn notice(&mut self, action: Option<Action>) {
        match (action, self.zen) {
            (Some(action), true) => self.quiet_notices.push(action),
            (action, false) => self.composed.extend(action),
            (None, true) => {}
        }
    }

    /// `]m` `[m`: the next or previous MR the queue shows, opened without leaving zen.
    pub(super) fn step_mr(&mut self, forward: bool) -> Vec<Action> {
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
        if self.zen {
            self.zen_switch = Some((self.now, self.zen_banner_text(&key, next, order.len())));
        }
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

    /// `‹  !42 title  ·  3/12  ›`: where zen just went, with `at` counted from 0.
    fn zen_banner_text(&self, key: &MrKey, at: usize, of: usize) -> String {
        let label =
            self.queue_mr(key).map_or_else(String::new, |mr| format!("{}{} {}", self.hosts.kind_of(key).sigil(), mr.number, mr.title));
        format!("‹  {label}  ·  {}/{of}  ›", at + 1)
    }

    fn queue_mr(&self, key: &MrKey) -> Option<&crate::forge::QueueMr> {
        self.sections.as_ref()?.all().find(|mr| mr.key() == *key)
    }
}

/// How long the zen switch line stays on top.
const ZEN_BANNER: std::time::Duration = std::time::Duration::from_millis(1_500);

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crate::tui::app::test_support::*;

    #[test]
    fn zen_gives_side_by_side_the_whole_screen_whatever_its_width_setting() {
        let mut app = with_sum_review();
        app.zen_width = Some(90);
        press(&mut app, "zzD");
        let screen = render(&mut app, 138, 18);
        assert!(app.open.as_ref().unwrap().review.shows_side_by_side(), "{screen}");
        assert!(screen.lines().any(|l| l.contains("-    let b = 2;") && l.contains("+    let b = 20;")), "{screen}");
    }

    #[test]
    fn zz_reads_the_diff_alone_and_h_brings_the_queue_back() {
        let mut app = with_queue();
        press(&mut app, "zz");
        assert!(!app.zen, "nothing to read before an MR is open");
        let mut app = with_review();
        press(&mut app, "zz");
        assert!(app.zen);
        let screen = render(&mut app, 160, 20);
        assert!(!screen.contains("Queue"), "{screen}");
        press(&mut app, "h");
        assert!(!app.zen);
        assert_eq!(app.focus, Focus::Queue);
    }

    #[test]
    fn zz_zen_order_is_the_queue_as_shown() {
        let mut app = with_review();
        press(&mut app, "zz");
        let order = app.zen_order();
        let shown: Vec<MrKey> = app
            .queue_rows()
            .iter()
            .filter_map(|row| match row {
                QueueRow::Mr(mr) | QueueRow::Stacked(mr) => Some(mr.key()),
                _ => None,
            })
            .collect();
        assert_eq!(order, shown, "every MR row, top to bottom, nothing from the folded sections");
        let done = app.sections.clone().unwrap().done;
        assert!(done.iter().all(|mr| !order.contains(&mr.key())), "Done is folded: its MRs are skipped");
    }

    #[test]
    fn brackets_m_in_zen_open_the_next_and_previous_mr_and_stay_in_zen() {
        let mut app = with_review();
        press(&mut app, "zz");
        let order = app.zen_order();
        assert!(order.len() > 1, "the fixture queue holds more than one MR");
        assert_eq!(order[0], mr_key());
        assert_eq!(press(&mut app, "]m"), vec![Action::Open(order[1].clone())]);
        assert!(app.zen, "the MR changes, zen stays");
        let banner = app.zen_banner(app.now).unwrap().to_owned();
        assert!(banner.contains(&format!("2/{}", order.len())), "{banner}");
        assert_eq!(app.selected_mr().map(crate::forge::QueueMr::key), Some(order[1].clone()), "leaving zen shows the queue on it");
        app.apply(Incoming::Review { key: order[1].clone(), review: Box::new(review()), cached: None });
        assert_eq!(press(&mut app, "[m"), vec![Action::Open(order[0].clone())]);
        app.apply(Incoming::Review { key: order[0].clone(), review: Box::new(review()), cached: None });
        assert_eq!(press(&mut app, "[m"), [] as [Action; 0]);
        assert!(app.live_toast().unwrap().text.contains("first MR"));
    }

    #[test]
    fn brackets_m_outside_zen_open_the_next_mr_without_a_banner() {
        let mut app = with_review();
        let order = app.zen_order();
        assert_eq!(press(&mut app, "]m"), vec![Action::Open(order[1].clone())]);
        assert!(!app.zen && app.zen_banner(app.now).is_none());
        assert_eq!(app.focus, Focus::Review);
        app.apply(Incoming::Review { key: order[1].clone(), review: Box::new(review()), cached: None });
        app.focus = Focus::Queue;
        assert_eq!(press(&mut app, "[m"), vec![Action::Open(order[0].clone())], "the queue reads it too");
    }

    #[test]
    fn arrows_in_zen_move_focus_as_outside_it() {
        let mut app = with_review();
        press(&mut app, "zz]N");
        assert!(app.handle_key(code(KeyCode::Right)).is_empty(), "→ never changes MR");
        assert!(app.open.as_ref().unwrap().pane.is_some(), "→ on a marked line opens the pane");
        assert_eq!(app.focus, Focus::Side);
        app.handle_key(code(KeyCode::Left));
        assert_eq!(app.focus, Focus::Review);
        assert!(app.zen);
        assert_eq!(app.handle_key(code(KeyCode::Left)), [] as [Action; 0]);
        assert!(!app.zen, "← from the diff leaves zen, as h does");
        assert_eq!(app.focus, Focus::Queue);
    }

    #[test]
    fn esc_and_h_leave_zen() {
        let mut app = with_review();
        press(&mut app, "zz");
        app.handle_key(code(KeyCode::Esc));
        assert!(!app.zen);
        assert_eq!(app.focus, Focus::Review, "esc leaves zen and stays on the diff");
        press(&mut app, "zz");
        press(&mut app, "h");
        assert!(!app.zen);
        assert_eq!(app.focus, Focus::Queue);
    }

    #[test]
    fn starting_on_an_mr_opens_it_in_zen_with_the_queue_loading_behind() {
        let target = with_queue().zen_order()[1].clone();
        let mut app = app();
        assert_eq!(app.start_on(target.clone()), vec![Action::LoadQueue { scope: None, from_cache: true }, Action::Open(target.clone())]);
        assert!(app.zen);
        assert_eq!(app.focus, Focus::Review);
        app.apply(Incoming::Queue { scope: None, me: "nina".into(), sections: sections(), opened: HashMap::new(), cached: false });
        app.apply(Incoming::Review { key: target.clone(), review: Box::new(review()), cached: None });
        assert_eq!(app.open.as_ref().map(|o| o.key.clone()), Some(target.clone()), "the queue arriving leaves the MR open");
        app.handle_key(code(KeyCode::Esc));
        assert!(!app.zen);
        assert_eq!(app.focus, Focus::Review, "esc leaves zen onto the diff");
        assert_eq!(app.selected_mr().map(crate::forge::QueueMr::key), Some(target), "the queue shows it selected, as if opened from it");
    }

    #[test]
    fn starting_on_an_mr_that_fails_to_open_leaves_zen_for_the_queue() {
        let mut app = app();
        app.start_on(mr_key());
        app.apply(Incoming::Failed { what: Failure::Open, message: "404 Not Found".into() });
        assert!(!app.zen, "zen hides the queue, the only place left to go");
        assert_eq!(app.focus, Focus::Queue);
    }

    #[test]
    fn notifications_wait_for_zen_to_end() {
        let mut app = with_review();
        let full = sections();
        let before = crate::forge::Sections { to_review: vec![], ..full.clone() };
        app.apply(Incoming::Queue { scope: None, me: "nina".into(), sections: before, opened: HashMap::new(), cached: false });
        app.take_actions();
        press(&mut app, "zz");
        app.apply(Incoming::Queue { scope: None, me: "nina".into(), sections: full, opened: HashMap::new(), cached: false });
        assert!(app.take_actions().iter().all(|a| !matches!(a, Action::Notify { .. })), "zen is quiet");
        let released = press(&mut app, "zz");
        assert!(matches!(released.as_slice(), [Action::Notify { .. }]), "{released:?}");
    }

    #[test]
    fn in_zen_h_and_l_move_between_the_diff_and_the_pane_under_it() {
        let mut app = zen_on_a_thread();
        render(&mut app, 138, 40);
        press(&mut app, "h");
        assert_eq!((app.focus, app.zen), (Focus::Review, true), "h from the pane stays in zen");
        assert!(render(&mut app, 138, 40).contains("─ charge.rs:-13 · 1 thread"), "the pane stays under the diff");
        press(&mut app, "l");
        assert_eq!(app.focus, Focus::Side);
        app.handle_key(code(KeyCode::Left));
        assert_eq!(app.focus, Focus::Review);
        app.handle_key(code(KeyCode::Right));
        assert_eq!(app.focus, Focus::Side);
    }

    #[test]
    fn in_zen_a_new_thread_box_keeps_a_row_to_type_in() {
        let mut app = with_review();
        let file = DiffFile {
            diff: "@@ -1,2 +1,3 @@\n a\n-b\n+c\n+d\n".into(),
            old_path: "src/quiet.rs".into(),
            new_path: "src/quiet.rs".into(),
            ..DiffFile::default()
        };
        app.apply(Incoming::Review { key: mr_key(), review: Box::new(Review::new(mr(), &[file], vec![], &[])), cached: None });
        app.review_jump_to(|row| matches!(row, Row::Line { .. }));
        press(&mut app, "zzc");
        for (width, height) in [(138, 40), (120, 25)] {
            let screen = render(&mut app, width, height);
            let lines: Vec<&str> = screen.lines().collect();
            let top = lines.iter().position(|l| l.contains("╭ new thread")).unwrap_or_else(|| panic!("a box:\n{screen}"));
            assert!(lines[top + 1].contains('│'), "{width}x{height}: a row to type in under the box's top:\n{screen}");
        }
    }

    #[test]
    fn in_zen_the_wheel_and_a_drag_stay_in_the_part_under_the_pointer() {
        let mut app = with_review();
        press(&mut app, "zzT");
        render(&mut app, 138, 40);
        let (diff, pane) = (app.areas.review, app.areas.side);
        let selected = app.open.as_ref().unwrap().selected;
        wheel(&mut app, true, pane.x + 5, pane.y + 2);
        assert_eq!(app.open.as_ref().unwrap().selected, selected, "the diff stays");
        assert_ne!(focused_thread(&app).as_deref(), Some("6a9c1750b2d6e4f0"), "the list moved");
        wheel(&mut app, true, diff.x + 5, diff.y + 5);
        assert_ne!(app.open.as_ref().unwrap().selected, selected, "the diff moved");
        assert_eq!(app.focus, Focus::Side);
        let code = spot(&mut app, "let client = Client::new()", 138, 40);
        let actions = drag(&mut app, code, (code.0, pane.y + 3), 138, 40);
        let Some(Action::Copy { text, .. }) = actions.first() else { panic!("a copy: {actions:?}") };
        assert!(text.starts_with("let client = Client::new();") && !text.contains("Why drop"), "{text}");
    }

    #[test]
    fn the_zen_header_counts_unresolved_threads() {
        let mut app = with_review();
        press(&mut app, "zz");
        let header = |app: &mut App| render(app, 160, 45).lines().find(|l| !l.trim().is_empty()).unwrap().trim_end().to_owned();
        assert!(header(&mut app).ends_with("omar · +5 −3 ✓ · ◆1"), "{}", header(&mut app));
        resolve_the_diff_note(&mut app);
        assert!(header(&mut app).ends_with("omar · +5 −3 ✓"), "no count once all are resolved");
    }

    #[test]
    fn a_toast_in_zen_shows_briefly_on_the_bottom_row() {
        let mut app = with_review();
        press(&mut app, "zz]m");
        let _ = render(&mut app, 160, 45);
        press(&mut app, "w");
        let screen = render(&mut app, 160, 45);
        assert!(screen.lines().last().unwrap().contains("long lines wrap"), "{screen}");
        app.now += std::time::Duration::from_secs(3);
        let screen = render(&mut app, 160, 45);
        assert!(!screen.contains("long lines wrap"), "gone after two seconds");
    }
}
