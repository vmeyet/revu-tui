//! The command line `mr` parses, with the help text each flag shows.
use crate::version;
use clap::{Args, Parser, Subcommand};

/// The command line: global flags, then one subcommand.
#[derive(Parser, Debug)]
#[command(name = "mr", version = version::label(), about = "GitLab merge requests from your terminal.", propagate_version = true)]
pub struct Cli {
    /// GitLab host. Defaults to the config, then gitlab.com.
    #[arg(long, global = true, env = "GITLAB_HOST")]
    pub host: Option<String>,
    /// Print machine-readable JSON instead of the pretty output.
    #[arg(long, global = true)]
    pub json: bool,
    /// Queue every project, not only the one of the checkout you are in.
    #[arg(long, global = true)]
    pub all: bool,
    /// What to run; the TUI when omitted.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Every subcommand `mr` knows.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Store a personal access token (scope `api`) in the keychain.
    Login(LoginArgs),
    /// Forget a host: keychain entry, config and cache.
    Logout {
        /// Forge host, the configured one by default.
        host: Option<String>,
    },
    /// Show who you are logged in as.
    Whoami,
    /// The merge requests waiting on you, yours, and the ones you watch.
    #[command(visible_alias = "ls")]
    List(ListArgs),
    /// One merge request: header, files, unresolved threads.
    Show(RefArgs),
    /// The coloured diff of a merge request, through your pager.
    Diff(RefArgs),
    /// Post a public comment on a merge request, on a line with `--at path:line`.
    Comment(CommentArgs),
    /// Approve a merge request (`--undo` takes it back).
    Approve(ApproveArgs),
    /// Publish every draft comment you hold on a merge request as one review.
    Publish(RefArgs),
    /// Interactive review client.
    Tui,
    /// Generate shell completions.
    Completions {
        /// The shell to write the script for.
        shell: clap_complete::Shell,
    },
    /// Rebuild and install the latest `mr` with cargo.
    Update(UpdateArgs),
}

/// Flags of `mr update`.
#[derive(Args, Debug)]
pub struct UpdateArgs {
    /// Install even when the running binary is already the latest commit.
    #[arg(short, long)]
    pub force: bool,
}

/// Flags of `mr login`.
#[derive(Args, Debug)]
pub struct LoginArgs {
    /// The forge host: `gitlab.com`, `github.com`, or your own. By default the checkout's, then the configured one.
    pub host: Option<String>,
    /// Read the token `glab` already holds instead of prompting (GitLab).
    #[arg(long, conflicts_with = "from_gh")]
    pub from_glab: bool,
    /// Read the token `gh` already holds instead of prompting (GitHub).
    #[arg(long)]
    pub from_gh: bool,
    /// Read the token from stdin (`-`) instead of prompting.
    #[arg(long, value_name = "-")]
    pub token: Option<String>,
}

/// Flags of `mr list`.
#[derive(Args, Debug)]
pub struct ListArgs {
    /// Print the last fetched queue without touching the network.
    #[arg(long)]
    pub cached: bool,
}

/// The merge request a read command works on.
#[derive(Args, Debug)]
pub struct RefArgs {
    /// `group/project!42`, `!42` (project from the origin remote), an MR URL, or nothing for the current branch.
    pub mr: Option<String>,
}

/// Arguments of `mr comment`.
#[derive(Args, Debug)]
pub struct CommentArgs {
    /// `group/project!42`, `!42`, or an MR URL.
    pub mr: String,
    /// Anchor the comment on a line of the new file: `src/a.rs:13`.
    #[arg(long, value_name = "PATH:LINE")]
    pub at: Option<String>,
    /// The comment, markdown.
    #[arg(required = true, trailing_var_arg = true)]
    pub text: Vec<String>,
}

/// Arguments of `mr approve`.
#[derive(Args, Debug)]
pub struct ApproveArgs {
    /// `group/project!42`, `!42`, an MR URL, or nothing for the current branch.
    pub mr: Option<String>,
    /// Remove your approval instead.
    #[arg(long)]
    pub undo: bool,
}
