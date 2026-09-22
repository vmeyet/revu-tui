//! One module per subcommand, each with a `run` the binary calls.
/// `mr approve`.
pub mod approve;
/// `mr comment`.
pub mod comment;
/// `mr diff`.
pub mod diff;
/// `mr list`.
pub mod list;
/// `mr login` and `mr logout`.
pub mod login;
/// `mr publish`.
pub mod publish;
/// `mr show`.
pub mod show;
mod target;
/// `mr update`.
pub mod update;
/// `mr whoami`.
pub mod whoami;
