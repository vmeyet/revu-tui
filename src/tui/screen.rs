//! The terminal as revu holds it: raw mode on the alternate screen, the mouse wheel, and ⌘
//! reported where the terminal speaks the kitty keyboard protocol (Ghostty, Kitty, `WezTerm`,
//! iTerm2 with the option on), so ⌘K reaches the palette like ctrl-k does.
use crossterm::event::{KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
use crossterm::terminal::{EnterAlternateScreen, enable_raw_mode};
use ratatui::DefaultTerminal;
use std::io::Write;

/// Clicks and the wheel, in the SGR encoding. Not crossterm's `EnableMouseCapture`: it also
/// asks for every pointer move, which would wake the loop for nothing.
const MOUSE_ON: &[u8] = b"\x1b[?1000h\x1b[?1006h";
const MOUSE_OFF: &[u8] = b"\x1b[?1006l\x1b[?1000l";

/// What revu asked of the terminal, so it can take it back exactly.
pub struct Screen {
    modifiers: bool,
}

impl Screen {
    /// Takes the terminal. `ratatui::init` installs the panic hook that gives it back; the mouse
    /// and the modifier report are undone first on a panic too, or the shell would read escape codes.
    pub fn enter() -> (DefaultTerminal, Self) {
        let terminal = ratatui::init();
        let modifiers = crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false) && report_modifiers();
        let screen = Self { modifiers };
        screen.take_extras_back_on_panic();
        write(MOUSE_ON);
        (terminal, screen)
    }

    fn take_extras_back_on_panic(&self) {
        let modifiers = self.modifiers;
        let restore = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            give_extras_back(modifiers);
            restore(info);
        }));
    }

    /// Gives the terminal back to the shell for good.
    pub fn leave(self) {
        give_extras_back(self.modifiers);
        ratatui::restore();
    }

    /// Lends the terminal to another program while `run` lasts: the program gets a plain terminal,
    /// then revu takes it back and repaints. No new panic hook stacks up on the way.
    pub fn lend<T>(&self, terminal: &mut DefaultTerminal, run: impl FnOnce() -> T) -> T {
        give_extras_back(self.modifiers);
        ratatui::restore();
        let outcome = run();
        let _ = enable_raw_mode();
        let _ = crossterm::execute!(std::io::stdout(), EnterAlternateScreen);
        if self.modifiers {
            report_modifiers();
        }
        write(MOUSE_ON);
        let _ = terminal.clear();
        outcome
    }
}

/// What revu asked on top of ratatui: the mouse, and the modifier report when it was on.
fn give_extras_back(modifiers: bool) {
    write(MOUSE_OFF);
    if modifiers {
        stop_modifiers();
    }
}

fn write(sequence: &[u8]) {
    let mut out = std::io::stdout();
    let _ = out.write_all(sequence).and_then(|()| out.flush());
}

fn report_modifiers() -> bool {
    let flags = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES;
    crossterm::execute!(std::io::stdout(), PushKeyboardEnhancementFlags(flags)).is_ok()
}

fn stop_modifiers() {
    let _ = crossterm::execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
}
