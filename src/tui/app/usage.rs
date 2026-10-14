//! The App's side of `[usage]`: it names what each key did and keeps the counts in memory.
//! Writing them is the runtime's job, so nothing here touches the disk.
use super::{App, Focus};
use crate::review::Row;
use crate::tui::palette::Command;
use crate::usage::{Counts, Place};
use crossterm::event::KeyEvent;
use std::time::{Duration, Instant};

/// A walk this long that reached a hunk or a thread is worth a hint: a jump would have got there.
const LONG_WALK: u32 = 15;
/// Reading goes on this long after the last key, click or wheel; past it the screen is left alone, not read.
const IDLE_AFTER: Duration = Duration::from_secs(60);
/// Opening another MR from the queue this soon after leaving zen is changing MR, which `[m` `]m` do in zen.
const ZEN_TO_QUEUE: Duration = Duration::from_secs(30);

/// Up to when screen time was counted, the last input, and whether the terminal is in front.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    counted_to: Instant,
    last_input: Instant,
    focused: bool,
}

impl Clock {
    pub fn starting(now: Instant) -> Self {
        Self { counted_to: now, last_input: now, focused: true }
    }

    /// The screen time since the clock last counted, and the clock counted up to `now`.
    fn count(self, now: Instant) -> (Duration, Self) {
        let read_until = now.min(self.last_input + IDLE_AFTER);
        let spent = if self.focused { read_until.saturating_duration_since(self.counted_to) } else { Duration::ZERO };
        (spent, Self { counted_to: now, ..self })
    }
}

/// The j or k in a row moving through the diff.
#[derive(Clone, Copy, Debug, Default)]
pub struct Walk {
    steps: u32,
    hinted: bool,
}

impl Walk {
    /// One more step, and whether it calls for the hint: once per walk, on a step that reached a hunk or a thread.
    fn step(self, reached_a_jump_stop: bool) -> (Self, bool) {
        let steps = self.steps + 1;
        let is_long = reached_a_jump_stop && steps >= LONG_WALK && !self.hinted;
        (Self { steps, hinted: self.hinted || is_long }, is_long)
    }
}

/// What counting a key needs to know from before the key changed anything.
pub(super) struct Before {
    name: Option<&'static str>,
    place: Place,
    row: Option<usize>,
    zen: bool,
}

impl App {
    /// What the key stands for and where it lands, read before the key changes anything; `None` when off.
    pub(super) fn usage_before(&self, key: KeyEvent) -> Option<Before> {
        self.usage.as_ref()?;
        let place = self.usage_place();
        let on_thread = self.pending.is_none() && key.modifiers.is_empty() && self.on_thread_row();
        let thread_action = if on_thread { crate::usage::thread_key(key.code) } else { None };
        let name = thread_action.or_else(|| crate::usage::action(self.pending, key, place));
        Some(Before { name, place, row: self.open.as_ref().map(|open| open.selected), zen: self.zen })
    }

    /// Counts what a key did, named by its handler or else by the key, and the habits a faster key would replace.
    pub(super) fn count_key(&mut self, before: Option<Before>) {
        let did = self.key_did.take();
        let Some(before) = before else { return };
        let Some(name) = did.or(before.name) else { return };
        self.count_walk(name, &before);
        self.count_zen_return(name, &before);
        self.count(name);
    }

    fn count_walk(&mut self, name: &'static str, before: &Before) {
        let is_step = matches!(name, "move_down" | "move_up") && before.place == Place::Diff;
        if !is_step {
            self.walk = Walk::default();
            return;
        }
        let (walk, is_long) = self.walk.step(self.reached_a_jump_stop(before.row));
        self.walk = walk;
        if is_long {
            self.count_hint("long_walk");
        }
    }

    /// The cursor went onto or past a row `]c` or `]n` stops on since `from`.
    fn reached_a_jump_stop(&self, from: Option<usize>) -> bool {
        let (Some(open), Some(from)) = (&self.open, from) else { return false };
        let to = open.selected;
        let passed = if to > from { from + 1..to + 1 } else { to..from };
        let stops = |row: &Row| matches!(row, Row::Hunk { .. }) || open.is_marked(row);
        open.rows.get(passed).is_some_and(|rows| rows.iter().any(stops))
    }

    fn count_zen_return(&mut self, name: &'static str, before: &Before) {
        if before.zen && !self.zen {
            self.left_zen = self.open.as_ref().map(|open| (self.now, open.key.clone()));
            return;
        }
        let is_queue_open = matches!(name, "open" | "focus_right") && before.place == Place::Queue;
        if !is_queue_open {
            return;
        }
        let Some((left_at, left_mr)) = self.left_zen.take() else { return };
        let is_soon = self.now.saturating_duration_since(left_at) <= ZEN_TO_QUEUE;
        let is_another_mr = self.selected_mr().is_some_and(|mr| mr.key() != left_mr);
        if is_soon && is_another_mr {
            self.count_hint("queue_after_zen");
        }
    }

    pub(super) fn count(&mut self, name: &'static str) {
        if let Some(tally) = &mut self.usage {
            tally.act(name);
        }
    }

    /// Counts a hint, and shows it as a toast its first few times.
    pub(super) fn count_hint(&mut self, name: &'static str) {
        let is_shown = self.usage.as_mut().is_some_and(|tally| tally.hint(name));
        if is_shown && let Some(tip) = self.tip(name) {
            self.toast(tip);
        }
    }

    fn tip(&self, hint: &str) -> Option<String> {
        let label = |keys| self.keymap.label(keys);
        Some(match hint {
            "long_walk" => format!("tip: {} jumps to the next hunk, {} to the next thread", label("]c"), label("]n")),
            "queue_after_zen" => format!("tip: {} and {} change MR without leaving zen", label("[m"), label("]m")),
            "palette_number" => "tip: !42 finds MR 42 directly, #42 on GitHub".to_owned(),
            _ => return None,
        })
    }

    pub(super) fn count_command(&mut self, command: &Command) {
        self.count(match command {
            Command::Go(_) => ":go",
            Command::Open => ":open",
            Command::Copy => ":copy",
            Command::Merge => ":merge",
            Command::Ready => ":ready",
            Command::Approve => ":approve",
            Command::Publish => ":publish",
            Command::Threads => ":threads",
            Command::All => ":all",
            Command::Pin => ":pin",
            Command::Unpin => ":unpin",
            Command::Set { .. } => ":set",
            Command::View(_) => ":view",
            Command::Outline => ":outline",
            Command::AiOff | Command::AiOn => ":ai",
            Command::Ask(_) => ":ask",
            Command::Share(_) => ":share",
            Command::Session => ":session",
            Command::News => ":news",
            Command::Help => ":help",
            Command::Quit => ":quit",
        });
    }

    /// Adds the time read since the clock last counted to the screen on show; called by `tick`.
    pub(super) fn count_time(&mut self) {
        let (spent, clock) = self.usage_clock.count(self.now);
        self.usage_clock = clock;
        let screen = self.screen();
        if let Some(tally) = &mut self.usage {
            tally.spend(screen, spent);
        }
    }

    /// A key, click or wheel: the time before it goes to the screen it was on, and reading goes on from now.
    pub(super) fn count_input(&mut self) {
        self.count_time();
        self.usage_clock = Clock { last_input: self.now, ..self.usage_clock };
    }

    /// The terminal came to the front or went to the background: the clock runs only while it is in front.
    pub fn terminal_focus(&mut self, focused: bool) {
        self.count_time();
        self.usage_clock = Clock { focused, last_input: self.now, ..self.usage_clock };
    }

    /// What the counts gathered since the last call add up to, for the runtime to write.
    pub fn take_usage(&mut self) -> Option<Counts> {
        self.usage.as_mut()?.take()
    }

    fn usage_place(&self) -> Place {
        match self.focus {
            Focus::Queue => Place::Queue,
            Focus::Review => Place::Diff,
            Focus::Side => Place::Pane,
        }
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
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crate::tui::app::test_support::*;

    fn seconds_on(app: &mut App, screen: &str) -> Option<u64> {
        app.take_usage().and_then(|counts| counts.screens.get(screen).copied())
    }

    fn hints(app: &mut App, hint: &str) -> Option<u64> {
        app.take_usage().and_then(|counts| counts.hints.get(hint).copied())
    }

    fn walk(app: &mut App, from_step: usize, steps: usize) {
        to_step(app, from_step);
        press(app, &"j".repeat(steps));
    }

    #[test]
    fn keys_are_counted_by_action_in_memory_only() {
        let mut app = counting();
        press(&mut app, "]n[Nzajj");
        let counts = app.take_usage().unwrap();
        assert_eq!(counts.actions.get("next_thread"), Some(&1));
        assert_eq!(counts.actions.get("prev_any_thread"), Some(&1));
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
        press(&mut app, "]Njjj");
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
    fn a_long_walk_onto_a_hunk_is_a_hint_once_and_shows_as_a_toast() {
        let mut app = counting();
        walk(&mut app, 0, 40);
        assert_eq!(hints(&mut app, "long_walk"), Some(1), "one hint per walk, not per step");
        assert!(app.live_toast().is_some_and(|toast| toast.text.contains("]c jumps to the next hunk")));
    }

    #[test]
    fn a_walk_inside_one_hunk_or_past_a_hunk_soon_after_it_starts_is_no_hint() {
        let mut inside = counting();
        walk(&mut inside, 0, 25);
        assert_eq!(hints(&mut inside, "long_walk"), None, "no jump stop on the way");
        let mut early = counting();
        walk(&mut early, 25, 25);
        assert_eq!(hints(&mut early, "long_walk"), None, "the hunk came after five steps, a jump saved little");
    }

    #[test]
    fn another_action_starts_the_walk_again() {
        let mut app = counting();
        walk(&mut app, 10, 10);
        press(&mut app, "w");
        press(&mut app, &"j".repeat(15));
        assert_eq!(hints(&mut app, "long_walk"), None, "ten steps to the hunk since `w`");
    }

    #[test]
    fn a_hint_shows_as_a_toast_until_it_was_shown_three_times() {
        let mut app = counting();
        let shown_twice = crate::usage::parse("{\"date\":\"2026-09-24\",\"version\":2,\"shown\":{\"long_walk\":2}}");
        app.usage = Some(crate::usage::Tally::after(&shown_twice));
        walk(&mut app, 0, 40);
        assert!(app.live_toast().is_some(), "the third time");
        press(&mut app, "w");
        app.toast = None;
        walk(&mut app, 0, 40);
        assert!(app.live_toast().is_none(), "the fourth time is only counted");
        assert_eq!(hints(&mut app, "long_walk"), Some(2));
    }

    fn zen_left_from(app: &mut App) {
        press(app, "zz");
        press(app, "h");
        assert_eq!(app.focus, Focus::Queue);
    }

    #[test]
    fn opening_another_mr_from_the_queue_soon_after_leaving_zen_is_a_hint() {
        let mut app = counting();
        zen_left_from(&mut app);
        app.now += Duration::from_secs(10);
        press(&mut app, "j");
        app.handle_key(code(KeyCode::Enter));
        assert_eq!(hints(&mut app, "queue_after_zen"), Some(1));
    }

    #[test]
    fn opening_from_the_queue_is_no_hint_on_the_same_mr_long_after_zen_or_without_it() {
        let mut same = counting();
        zen_left_from(&mut same);
        same.handle_key(code(KeyCode::Enter));
        assert_eq!(hints(&mut same, "queue_after_zen"), None, "back to the MR zen showed");
        let mut later = counting();
        zen_left_from(&mut later);
        later.now += Duration::from_secs(31);
        press(&mut later, "jl");
        assert_eq!(hints(&mut later, "queue_after_zen"), None, "half a minute after zen is a new choice");
        press(&mut later, "hjl");
        assert_eq!(hints(&mut later, "queue_after_zen"), None, "zen was used once, this open is not after it");
    }

    #[test]
    fn q_and_esc_that_close_the_pane_count_as_closing_it() {
        let mut app = with_review();
        app.usage = Some(crate::usage::Tally::default());
        press(&mut app, "]N");
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

    #[test]
    fn enter_on_a_file_row_counts_as_folding_it() {
        let mut app = counting();
        assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::File { .. })));
        app.handle_key(code(KeyCode::Enter));
        assert_eq!(app.take_usage().unwrap().actions.get("fold_toggle"), Some(&1));
    }

    #[test]
    fn a_bare_number_in_the_search_is_a_hint_and_commands_are_counted() {
        let mut app = counting();
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
        press(&mut app, "42");
        app.handle_key(code(KeyCode::Enter));
        let counts = app.take_usage().unwrap();
        assert_eq!(counts.actions.get("jump"), Some(&1));
        assert_eq!(counts.hints.get("palette_number"), Some(&1));
        press(&mut app, ":help");
        app.handle_key(code(KeyCode::Enter));
        assert_eq!(app.take_usage().unwrap().actions.get(":help"), Some(&1));
    }
}
