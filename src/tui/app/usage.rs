//! The App's side of `[usage]`: it names what each key did and keeps the counts in memory.
//! Writing them is the runtime's job, so nothing here touches the disk.
use super::{App, Focus};
use crate::tui::palette::Command;
use crate::usage::{Counts, Place};
use crossterm::event::KeyEvent;
use std::time::Duration;

/// A walk longer than this, where a jump existed, is worth a hint.
const LONG_WALK: u32 = 15;
/// Longer gaps between two ticks are a sleeping laptop or an editor hand-off, not reading.
const MAX_GAP: Duration = Duration::from_secs(60);

impl App {
    /// The action a key stands for, read before the key changes anything; `None` when off.
    pub(super) fn usage_name(&self, key: KeyEvent) -> Option<(&'static str, Place)> {
        self.usage.as_ref()?;
        let place = self.usage_place();
        crate::usage::action(self.pending, key, place).map(|name| (name, place))
    }

    /// Counts a key's action, and the habits a faster key would replace.
    pub(super) fn count_key(&mut self, named: Option<(&'static str, Place)>, was_zen: bool) {
        let Some((name, place)) = named else { return };
        let walking = matches!(name, "move_down" | "move_up") && place == Place::Diff && self.jumps_exist();
        self.walk = if walking { self.walk + 1 } else { 0 };
        if self.walk == LONG_WALK {
            self.count_hint("long_walk");
        }
        if matches!(name, "open" | "focus_right") && place == Place::Queue && self.zen_seen && !was_zen {
            self.count_hint("queue_after_zen");
        }
        self.count(name);
    }

    pub(super) fn count(&mut self, name: &'static str) {
        if let Some(tally) = &mut self.usage {
            tally.act(name);
        }
    }

    pub(super) fn count_hint(&mut self, name: &'static str) {
        if let Some(tally) = &mut self.usage {
            tally.hint(name);
        }
    }

    pub(super) fn count_command(&mut self, command: &Command) {
        self.count(match command {
            Command::Go(_) => ":go",
            Command::Open => ":open",
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
            Command::Help => ":help",
            Command::Quit => ":quit",
        });
    }

    /// Adds the time since the last tick to the screen on show; called by `tick`.
    pub(super) fn count_time(&mut self) {
        let spent = self.now.saturating_duration_since(self.usage_at).min(MAX_GAP);
        self.usage_at = self.now;
        self.zen_seen |= self.zen;
        let screen = self.screen();
        if let Some(tally) = &mut self.usage {
            tally.spend(screen, spent);
        }
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

    /// The open diff has somewhere to jump to: another hunk, or a thread.
    fn jumps_exist(&self) -> bool {
        self.open.as_ref().is_some_and(|open| {
            let hunks: usize = open.review.files.iter().map(|file| file.hunks.len()).sum();
            hunks > 1 || !open.review.threads.is_empty()
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crate::tui::app::test_support::*;

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
        app.now += std::time::Duration::from_secs(5);
        app.tick();
        assert_eq!(app.take_usage(), None);
    }

    #[test]
    fn time_goes_to_the_screen_on_show() {
        let mut app = counting();
        app.now += std::time::Duration::from_secs(3);
        app.tick();
        press(&mut app, "zz");
        app.now += std::time::Duration::from_secs(2);
        app.tick();
        let counts = app.take_usage().unwrap();
        assert_eq!((counts.screens.get("diff"), counts.screens.get("zen")), (Some(&3), Some(&2)));
    }

    #[test]
    fn a_long_walk_where_a_jump_existed_is_a_hint_once() {
        let mut app = counting();
        press(&mut app, &"j".repeat(30));
        assert_eq!(app.take_usage().unwrap().hints.get("long_walk"), Some(&1), "one hint per walk, not per step");
        press(&mut app, &"j".repeat(10));
        press(&mut app, "zc");
        press(&mut app, &"j".repeat(10));
        assert!(app.take_usage().unwrap().hints.is_empty(), "another action ends the walk");
    }

    #[test]
    fn changing_mr_from_the_queue_after_zen_is_a_hint() {
        let mut app = counting();
        press(&mut app, "zz");
        app.tick();
        press(&mut app, "zz");
        app.focus = Focus::Queue;
        app.handle_key(code(KeyCode::Enter));
        assert_eq!(app.take_usage().unwrap().hints.get("queue_after_zen"), Some(&1));
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
