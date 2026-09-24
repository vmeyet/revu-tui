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
/// `revu publish`.
pub mod publish;
/// `revu share`.
pub mod share;
/// `revu show`.
pub mod show;
mod target;
/// `revu update`.
pub mod update;
/// `revu whoami`.
pub mod whoami;
