//! Merge request review in your terminal, on GitLab and GitHub: the library behind the `revu` binary.
mod auth;
mod cache;
pub mod cli;
pub mod commands;
mod config;
pub mod ctx;
mod diff;
mod forge;
mod fuzzy;
mod legacy;
mod mrref;
mod open;
mod render;
mod review;
mod syntax;
pub mod tui;
mod update;
mod version;

/// Moves what the tool kept under its old name, `gitlabmr`, to `revu`; runs before anything reads the config.
pub fn adopt_old_name() {
    legacy::move_dirs();
}
