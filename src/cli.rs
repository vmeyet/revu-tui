//! The command line `revu` parses, with the help text each flag shows.
use crate::version;
use clap::{Args, Parser, Subcommand};

/// The command line: global flags, then one subcommand.
#[derive(Parser, Debug)]
#[command(name = "revu", version = version::label(), about = "Review GitLab merge requests and GitHub pull requests from your terminal.", propagate_version = true)]
pub struct Cli {
    /// The forge host, GitLab or GitHub. Defaults to the checkout's, then the config, then gitlab.com.
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

/// Every subcommand `revu` knows.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Store a token in the keychain: GitLab scope `api`, GitHub scope `repo`.
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
    /// Post a merge request through a `[share]` command: a chat channel, a webhook, a script.
    Share(ShareArgs),
    /// The AI providers: store a key, forget it, see which one is on.
    Ai(AiArgs),
    /// Interactive review client.
    Tui,
    /// Generate shell completions.
    Completions {
        /// The shell to write the script for.
        shell: clap_complete::Shell,
    },
    /// Rebuild and install the latest `revu` with cargo.
    Update(UpdateArgs),
    /// Write the reference pages under `docs/reference/` from the code.
    #[command(hide = true)]
    Docs(DocsArgs),
}

/// Flags of `revu docs`.
#[derive(Args, Debug)]
pub struct DocsArgs {
    /// Where the pages go.
    #[arg(long, default_value = "docs/reference")]
    pub dir: std::path::PathBuf,
    /// Write nothing; fail when a page on disk differs from the code.
    #[arg(long)]
    pub check: bool,
}

/// Flags of `revu update`.
#[derive(Args, Debug)]
pub struct UpdateArgs {
    /// Install even when the running binary is already the latest commit.
    #[arg(short, long)]
    pub force: bool,
}

/// Flags of `revu login`.
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

/// Flags of `revu list`.
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

/// Arguments of `revu comment`.
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

/// Arguments of `revu share`.
#[derive(Args, Debug)]
pub struct ShareArgs {
    /// `group/project!42`, `!42`, an MR URL, or nothing for the current branch.
    pub mr: Option<String>,
    /// Which `[share.targets.<name>]` to post to; needed when there are several.
    #[arg(long)]
    pub target: Option<String>,
    /// A line of context under the link.
    #[arg(long)]
    pub note: Option<String>,
    /// Send without asking.
    #[arg(long)]
    pub yes: bool,
    /// Print the message and send nothing.
    #[arg(long)]
    pub dry_run: bool,
}

/// Arguments of `revu approve`.
#[derive(Args, Debug)]
pub struct ApproveArgs {
    /// `group/project!42`, `!42`, an MR URL, or nothing for the current branch.
    pub mr: Option<String>,
    /// Remove your approval instead.
    #[arg(long)]
    pub undo: bool,
}

/// `revu ai`: one subcommand.
#[derive(Args, Debug)]
pub struct AiArgs {
    /// What to do with the AI keys, or a question.
    #[command(subcommand)]
    pub command: AiCommand,
}

impl AiArgs {
    /// `ask` reads an MR, so it needs the forge; the key commands do not.
    pub fn needs_forge(&self) -> bool {
        matches!(self.command, AiCommand::Ask(_))
    }
}

/// The `revu ai` subcommands.
#[derive(Subcommand, Debug)]
pub enum AiCommand {
    /// Store a provider's key in the keychain; the config still has to switch it on.
    Login(AiLoginArgs),
    /// Forget a provider's key.
    Logout {
        /// Which provider.
        provider: crate::ai::Provider,
    },
    /// Which provider is on and where its key comes from (never the key).
    Status,
    /// Ask Claude about a merge request; the answer streams to stdout.
    Ask(AiAskArgs),
}

/// Arguments of `revu ai ask`.
#[derive(Args, Debug)]
pub struct AiAskArgs {
    /// `group/project!42`, `!42`, an MR URL, or `-` for the current branch.
    pub mr: String,
    /// Ask about one file instead of the whole MR.
    #[arg(long, value_name = "PATH")]
    pub file: Option<String>,
    /// With `--file`: the new-side lines asked about, `13` or `13-20`.
    #[arg(long, value_name = "LINES", requires = "file")]
    pub lines: Option<String>,
    /// The question; a summary of the MR (or an explanation of the file) without one.
    #[arg(trailing_var_arg = true)]
    pub question: Vec<String>,
}

/// Flags of `revu ai login`.
#[derive(Args, Debug)]
pub struct AiLoginArgs {
    /// Which provider.
    pub provider: crate::ai::Provider,
    /// Read the key from stdin (`-`) instead of prompting.
    #[arg(long, value_name = "-")]
    pub token: Option<String>,
    /// Reuse the TypeSafe key slack-tui keeps in the keychain.
    #[arg(long, conflicts_with = "token")]
    pub from_slack_tui: bool,
}
