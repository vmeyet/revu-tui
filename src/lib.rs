//! Merge request review in your terminal, on GitLab and GitHub: the library behind the `revu` binary.
mod ai;
mod auth;
mod cache;
pub mod cli;
pub mod commands;
mod config;
pub mod ctx;
mod diff;
mod docs;
mod forge;
mod fuzzy;
mod keymap;
mod legacy;
mod mrref;
mod open;
mod program;
mod query;
mod ready;
mod render;
mod review;
mod share;
mod syntax;
pub mod tui;
mod update;
mod usage;
mod version;

/// Moves what the tool kept under its old name, `gitlabmr`, to `revu`; runs before anything reads the config.
pub fn adopt_old_name() {
    legacy::move_dirs();
}
