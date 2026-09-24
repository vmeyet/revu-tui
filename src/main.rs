//! The `revu` binary: parses the command line and runs the matching command.
use anyhow::Result;
use clap::{CommandFactory, Parser};
use revu::cli::{Cli, Command};
use revu::commands;
use revu::ctx::Ctx;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    revu::adopt_old_name();
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
        Some(Command::Docs(args)) => return commands::docs::run(&args),
        Some(Command::Ai(args)) if !args.needs_forge() => return commands::ai::run(args, cli.json).await,
        Some(Command::Completions { shell }) => {
            clap_complete::generate(shell, &mut Cli::command(), "revu", &mut std::io::stdout());
            return Ok(());
        }
        _ => {}
    }
    let ctx = Ctx::open(cli.host.as_deref(), cli.json)?.everywhere(cli.all);
    match cli.command {
        Some(Command::Whoami) => commands::whoami::run(&ctx).await,
        Some(Command::List(args)) => commands::list::run(&ctx, args).await,
        Some(Command::Show(args)) => commands::show::run(&ctx, args).await,
        Some(Command::Diff(args)) => commands::diff::run(&ctx, args).await,
        Some(Command::Comment(args)) => commands::comment::run(&ctx, args).await,
        Some(Command::Approve(args)) => commands::approve::run(&ctx, args).await,
        Some(Command::Merge(args)) => commands::merge::run(&ctx, args).await,
        Some(Command::Publish(args)) => commands::publish::run(&ctx, args).await,
        Some(Command::Share(args)) => commands::share::run(&ctx, args).await,
        Some(Command::Ai(args)) => commands::ai::ask(&ctx, args).await,
        Some(Command::Tui) | None => revu::tui::run(ctx).await,
        Some(Command::Login(_) | Command::Logout { .. } | Command::Completions { .. } | Command::Update(_) | Command::Docs(_)) => {
            unreachable!("handled above")
        }
    }
}
