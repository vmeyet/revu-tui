//! The `revu` binary: parses the command line and runs the matching command.
use anyhow::Result;
use clap::{CommandFactory, Parser};
use revu::cli::{Cli, Command};
use revu::commands;
use revu::ctx::Ctx;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    revu::log::start();
    if cli.command.is_some() {
        commands::warn_of_ignored_settings();
    }
    if let Err(err) = run(cli).await {
        eprintln!("\x1b[31m✗\x1b[0m {err}");
        for cause in err.chain().skip(1) {
            eprintln!("  \x1b[2m{cause}\x1b[0m");
        }
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Some(Command::Login(args)) => return commands::login::run(args, cli.host.as_deref(), cli.json).await,
        Some(Command::Logout { host }) => return commands::login::logout(host.or(cli.host).as_deref()),
        Some(Command::Update(args)) => return commands::update::run(&args),
        Some(Command::Usage(args)) => return commands::usage::run(&args, cli.json),
        Some(Command::News(args)) => return commands::news::run(&args, cli.json),
        Some(Command::Ai(args)) => return commands::ai::run(args, cli.json),
        Some(Command::Local(args)) => return commands::local::run(args).await,
        Some(Command::Completions { shell }) => {
            clap_complete::generate(shell, &mut Cli::command(), "revu", &mut std::io::stdout());
            return Ok(());
        }
        _ => {}
    }
    let ctx = Ctx::open(cli.host.as_deref(), cli.json)?.everywhere(cli.all);
    let Some(command) = cli.command else { return commands::tui::run(ctx, cli.mr).await };
    run_on_forge(&ctx, command).await.map_err(|err| ctx.with_host_hint(err))
}

/// The commands that ask the forge of `ctx`.
async fn run_on_forge(ctx: &Ctx, command: Command) -> Result<()> {
    match command {
        Command::Whoami => commands::whoami::run(ctx).await,
        Command::List(args) => commands::list::run(ctx, args).await,
        Command::Show(args) => commands::show::run(ctx, args).await,
        Command::Diff(args) => commands::diff::run(ctx, args).await,
        Command::Comment(args) => commands::comment::run(ctx, args).await,
        Command::Approve(args) => commands::approve::run(ctx, args).await,
        Command::Merge(args) => commands::merge::run(ctx, args).await,
        Command::Ready(args) => commands::ready::run(ctx, args).await,
        Command::Publish(args) => commands::publish::run(ctx, args).await,
        Command::Share(args) => commands::share::run(ctx, args).await,
        Command::Ai(_)
        | Command::Local(_)
        | Command::Login(_)
        | Command::Logout { .. }
        | Command::Completions { .. }
        | Command::Update(_)
        | Command::Usage(_)
        | Command::News(_) => {
            unreachable!("handled above")
        }
    }
}
