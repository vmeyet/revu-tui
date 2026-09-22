//! GitLab merge request review in your terminal: the library behind the `mr` binary.
mod auth;
mod cache;
pub mod cli;
pub mod commands;
mod config;
pub mod ctx;
mod diff;
mod forge;
mod mrref;
mod render;
mod review;
pub mod tui;
mod update;
mod version;
