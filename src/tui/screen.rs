//! The terminal as revu holds it: raw mode on the alternate screen, and ⌘ reported where the
//! terminal speaks the kitty keyboard protocol (Ghostty, Kitty, `WezTerm`, iTerm2 with the option
//! on), so ⌘K reaches the palette like ctrl-k does.
use crossterm::event::{KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
use crossterm::terminal::{EnterAlternateScreen, enable_raw_mode};
use ratatui::DefaultTerminal;

/// What revu asked of the terminal, so it can take it back exactly.
pub struct Screen {
    modifiers: bool,
}

impl Screen {
    /// Takes the terminal. `ratatui::init` installs the panic hook that gives it back; the
    /// modifier report is undone first on a panic too, or the shell would read escape codes.
    pub fn enter() -> (DefaultTerminal, Self) {
        let terminal = ratatui::init();
        let modifiers = crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false) && report_modifiers();
        if modifiers {
            let restore = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                stop_modifiers();
                restore(info);
            }));
        }
        (terminal, Self { modifiers })
    }

    /// Gives the terminal back to the shell for good.
    pub fn leave(self) {
        if self.modifiers {
            stop_modifiers();
        }
        ratatui::restore();
    }

    /// Lends the terminal to another program while `run` lasts: the program gets a plain terminal,
    /// then revu takes it back and repaints. No new panic hook stacks up on the way.
    pub fn lend<T>(&self, terminal: &mut DefaultTerminal, run: impl FnOnce() -> T) -> T {
        if self.modifiers {
            stop_modifiers();
        }
        ratatui::restore();
        let outcome = run();
        let _ = enable_raw_mode();
        let _ = crossterm::execute!(std::io::stdout(), EnterAlternateScreen);
        if self.modifiers {
            report_modifiers();
        }
        let _ = terminal.clear();
        outcome
    }
}

fn report_modifiers() -> bool {
    let flags = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES;
    crossterm::execute!(std::io::stdout(), PushKeyboardEnhancementFlags(flags)).is_ok()
}

fn stop_modifiers() {
    let _ = crossterm::execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
}
