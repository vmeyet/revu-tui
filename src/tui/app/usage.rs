//! The App's side of `[usage]`: it names what each key did and keeps the counts in memory.
//! Writing them is the runtime's job, so nothing here touches the disk.
use super::{App, Focus};
use crate::act::{Act, Pane};
use crate::tui::palette::Command;
use crate::usage::Counts;
use crossterm::event::KeyEvent;
use std::time::Duration;

/// What `[usage]` keeps while revu runs.
#[derive(Debug)]
pub(crate) struct Usage {
    /// What this session counted since the last flush; `None` when `[usage]` is off.
    pub(super) tally: Option<crate::usage::Tally>,
    /// Up to when screen time is counted, and what stops it.
    pub(super) clock: vtui::usage::Clock,
}

impl App {
    /// Names what the key in flight did, unless its handler already named something closer.
    pub(super) fn did(&mut self, act: Act) {
        self.key_did.get_or_insert(act);
    }

    /// What `key` does in `pane`, named as what the key in flight did until its handler says it did nothing.
    pub(super) fn act_here(&mut self, pane: Pane, key: KeyEvent) -> Option<Act> {
        let act = crate::act::act_in(pane, key)?;
        self.did(act);
        Some(act)
    }

    /// The key in flight did nothing after all: no action is counted for it.
    pub(super) fn did_nothing(&mut self) {
        self.key_did = None;
    }

    /// Counts what a key did, as its handler named it.
    pub(super) fn count_key(&mut self) {
        if let Some(name) = self.key_did.take().and_then(|act| act.row().name.counted()) {
            self.count(name);
        }
    }

    pub(super) fn count(&mut self, name: &'static str) {
        if let Some(tally) = &mut self.usage.tally {
            tally.act(name);
        }
    }

    pub(super) fn count_command(&mut self, command: &Command) {
        if let Some(name) = command.act().row().name.counted() {
            self.count(name);
        }
    }

    /// Adds the time read since the clock last counted to the screen on show; called by `tick`.
    pub(super) fn count_time(&mut self) {
        let read = self.usage.clock.count(self.now);
        self.spend(read);
    }

    /// A key, click or wheel: the time before it goes to the screen it was on, and reading goes on from now.
    pub(super) fn count_input(&mut self) {
        let read = self.usage.clock.input(self.now);
        self.spend(read);
    }

    /// The terminal came to the front or went to the background: the clock runs only while it is in front.
    pub(crate) fn terminal_focus(&mut self, focused: bool) {
        let read = if focused { self.usage.clock.to_front(self.now) } else { self.usage.clock.to_back(self.now) };
        self.spend(read);
    }

    fn spend(&mut self, read: Duration) {
        let screen = self.screen();
        if let Some(tally) = &mut self.usage.tally {
            tally.spend(screen, read);
        }
    }

    /// What the counts gathered since the last call add up to, for the runtime to write.
    pub(crate) fn take_usage(&mut self) -> Option<Counts> {
        self.usage.tally.as_mut()?.take()
    }

    fn screen(&self) -> &'static str {
        match () {
            () if self.help.is_some() => "help",
            () if self.palette.is_some() => "palette",
            () if self.brief.is_some() => "cover",
            () if self.zen => "zen",
            () => match self.focus {
                Focus::Queue => "queue",
                Focus::Review => "diff",
                Focus::Side => "pane",
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::tui::app::test_support::*;

    fn seconds_on(app: &mut App, screen: &str) -> Option<u64> {
        app.take_usage().and_then(|counts| counts.screens.get(screen).copied())
    }

    #[test]
    fn keys_are_counted_by_action_in_memory_only() {
        let mut app = counting();
        press(&mut app, "]n[nzajj");
        let counts = app.take_usage().unwrap();
        assert_eq!(counts.actions.get("next_thread"), Some(&1));
        assert_eq!(counts.actions.get("prev_thread"), Some(&1));
        assert_eq!(counts.actions.get("fold_toggle"), Some(&1));
        assert_eq!(counts.actions.get("move_down"), Some(&2));
        assert_eq!(app.take_usage(), None, "taken once, gone");
    }

    #[test]
    fn m_counts_as_viewed() {
        let mut app = counting();
        to_step(&mut app, 1);
        press(&mut app, "m");
        assert_eq!(app.take_usage().unwrap().actions.get("viewed"), Some(&1));
    }

    #[test]
    fn n_counts_as_a_thread_jump_only_with_no_search() {
        let mut app = counting();
        press(&mut app, "nN");
        let counts = app.take_usage().unwrap();
        assert_eq!((counts.actions.get("next_thread"), counts.actions.get("prev_thread")), (Some(&1), Some(&1)));
        press(&mut app, "/step");
        app.handle_key(code(KeyCode::Enter));
        app.take_usage();
        press(&mut app, "nN");
        let stepped = app.take_usage().map(|counts| counts.actions).unwrap_or_default();
        assert_eq!((stepped.get("next_thread"), stepped.get("prev_thread")), (None, None), "{stepped:?}");
    }

    #[test]
    fn typed_text_is_never_counted() {
        let mut app = counting();
        to_step(&mut app, 1);
        press(&mut app, "c");
        app.take_usage();
        press(&mut app, "secret words");
        assert_eq!(app.take_usage(), None, "keys in a text box name no action");
    }

    #[test]
    fn usage_off_counts_nothing() {
        let mut app = with_long_review();
        walk_to_any_thread(&mut app, Toward::Next);
        press(&mut app, "jjj");
        app.now += Duration::from_secs(5);
        app.tick();
        assert_eq!(app.take_usage(), None);
    }

    #[test]
    fn time_goes_to_the_screen_on_show() {
        let mut app = counting();
        app.now += Duration::from_secs(3);
        app.tick();
        press(&mut app, "zz");
        app.now += Duration::from_secs(2);
        app.tick();
        let counts = app.take_usage().unwrap();
        assert_eq!((counts.screens.get("diff"), counts.screens.get("zen")), (Some(&3), Some(&2)));
    }

    #[test]
    fn time_stops_a_minute_after_the_last_input_and_starts_again_on_the_next() {
        let mut app = counting();
        for _ in 0..600 {
            app.now += Duration::from_secs(1);
            app.tick();
        }
        assert_eq!(seconds_on(&mut app, "diff"), Some(60), "ten idle minutes count as one");
        press(&mut app, "j");
        app.now += Duration::from_secs(5);
        app.tick();
        assert_eq!(seconds_on(&mut app, "diff"), Some(5));
    }

    #[test]
    fn time_stops_while_the_terminal_is_in_the_background() {
        let mut app = counting();
        app.now += Duration::from_secs(5);
        app.terminal_focus(false);
        app.now += Duration::from_secs(30);
        app.tick();
        assert_eq!(seconds_on(&mut app, "diff"), Some(5), "the time in front only");
        app.terminal_focus(true);
        app.now += Duration::from_secs(50);
        app.tick();
        assert_eq!(seconds_on(&mut app, "diff"), Some(50), "back in front, the clock runs without a key");
    }

    #[test]
    fn q_and_esc_that_close_the_pane_count_as_closing_it() {
        let mut app = with_review();
        app.usage.tally = Some(crate::usage::Tally::default());
        walk_to_any_thread(&mut app, Toward::Next);
        app.handle_key(code(KeyCode::Enter));
        assert_eq!(app.focus, Focus::Side);
        press(&mut app, "q");
        assert!(!app.should_quit && app.focus == Focus::Review);
        press(&mut app, "l");
        app.handle_key(code(KeyCode::Esc));
        let counts = app.take_usage().unwrap();
        assert_eq!((counts.actions.get("close_pane"), counts.actions.get("quit")), (Some(&2), None));
        press(&mut app, "x");
        assert_eq!(app.take_usage(), None, "x with nothing to close does nothing");
    }

    /// What one key counted, on top of what was counted before.
    fn counted_by(app: &mut App, event: KeyEvent) -> Vec<String> {
        app.take_usage();
        app.handle_key(event);
        app.take_usage().map(|counts| counts.actions.into_keys().collect()).unwrap_or_default()
    }

    fn counted_by_keys(app: &mut App, keys: &str) -> Vec<String> {
        app.take_usage();
        press(app, keys);
        app.take_usage().map(|counts| counts.actions.into_keys().collect()).unwrap_or_default()
    }

    fn with_usage(mut app: App) -> App {
        app.usage.tally = Some(crate::usage::Tally::default());
        app
    }

    #[test]
    fn a_key_counts_as_what_it_did_where_it_landed() {
        let mut queue = with_usage(with_queue());
        assert_eq!(counted_by_keys(&mut queue, "s"), ["sort_queue"]);
        let mut diff = with_usage(with_review());
        assert_eq!(counted_by_keys(&mut diff, "]m"), ["next_mr"]);
        let mut diff = with_usage(with_review());
        assert_eq!(counted_by_keys(&mut diff, "]n"), ["next_thread"]);
        assert_eq!(counted_by_keys(&mut diff, "="), ["more_context"]);
        assert_eq!(counted_by(&mut diff, code(KeyCode::Right)), ["focus_right"]);
        assert_eq!(counted_by_keys(&mut diff, "z"), Vec::<String>::new(), "a prefix alone is no action");
        let mut suggest = with_usage(with_review());
        on_line(&mut suggest);
        assert_eq!(counted_by_keys(&mut suggest, "s"), ["suggest"]);
        let mut jump = with_usage(with_queue());
        assert_eq!(counted_by(&mut jump, ctrl('k')), ["jump"]);
    }

    #[test]
    fn enter_in_the_thread_pane_counts_as_what_it_did() {
        let mut app = with_usage(with_review());
        walk_to_any_thread(&mut app, Toward::Next);
        app.handle_key(code(KeyCode::Enter));
        assert_eq!(app.focus, Focus::Side);
        assert_eq!(counted_by(&mut app, code(KeyCode::Enter)), ["fold_toggle"], "it folds the resolved thread");
        assert_eq!(counted_by_keys(&mut app, "S"), ["apply_suggestion"]);
        press(&mut app, "T");
        assert_eq!(counted_by(&mut app, code(KeyCode::Enter)), ["open"], "in every thread it goes to the thread");
    }

    #[test]
    fn a_ctrl_letter_counts_only_what_it_did() {
        let mut app = with_usage(with_queue());
        let before = app.queue_selected;
        assert_eq!(counted_by(&mut app, ctrl('j')), Vec::<String>::new());
        assert_eq!(app.queue_selected, before, "`^j` is no move");
        assert_eq!(counted_by(&mut app, ctrl('d')), ["page_down"]);
        assert_eq!(counted_by(&mut app, code(KeyCode::Esc)), Vec::<String>::new(), "esc with no filter to clear does nothing");
    }

    #[test]
    fn enter_on_a_file_row_counts_as_folding_it() {
        let mut app = counting();
        assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::File { .. })));
        app.handle_key(code(KeyCode::Enter));
        assert_eq!(app.take_usage().unwrap().actions.get("fold_toggle"), Some(&1));
    }

    #[test]
    fn the_search_and_commands_are_counted() {
        let mut app = counting();
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
        press(&mut app, "42");
        app.handle_key(code(KeyCode::Enter));
        assert_eq!(app.take_usage().unwrap().actions.get("jump"), Some(&1));
        press(&mut app, ":news");
        app.handle_key(code(KeyCode::Enter));
        assert_eq!(app.take_usage().unwrap().actions.get(":news"), Some(&1));
    }
}
