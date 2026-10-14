//! One module per subcommand, each with a `run` the binary calls.
/// `revu ai`.
pub mod ai;
/// `revu approve`.
pub mod approve;
/// `revu comment`.
pub mod comment;
/// `revu diff`.
pub mod diff;
/// `revu docs`: the reference pages, written from the code.
pub mod docs;
/// `revu list`.
pub mod list;
/// `revu login` and `revu logout`.
pub mod login;
/// `revu merge`.
pub mod merge;
/// `revu news`.
pub mod news;
/// `revu publish`.
pub mod publish;
/// `revu ready`.
pub mod ready;
/// `revu share`.
pub mod share;
/// `revu show`.
pub mod show;
mod target;
/// `revu tui` and `revu <ref>`.
pub mod tui;
/// `revu update`.
pub mod update;
/// `revu usage`.
pub mod usage;
/// `revu whoami`.
pub mod whoami;

use crate::render::{Style, Theme};

/// Each key of the config this revu ignores, on stderr; the TUI toasts them instead.
pub fn warn_of_ignored_settings() {
    let warnings = crate::config::warnings();
    if warnings.is_empty() {
        return;
    }
    let theme = Theme::detect();
    for warning in &warnings {
        eprintln!("{} {warning}", theme.paint("!", Style::Warn));
    }
    eprintln!("  {}", theme.paint(&format!("in {}", crate::config::Config::path().display()), Style::Dim));
}
