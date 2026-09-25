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
            Command::Set { .. } => ":set",
            Command::View(_) => ":view",
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
