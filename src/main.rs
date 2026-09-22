use anyhow::Result;
use clap::{CommandFactory, Parser};
use gitlabmr::cli::{Cli, Command};
use gitlabmr::commands;
use gitlabmr::ctx::Ctx;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
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
        Some(Command::Logout { host }) => return commands::login::logout(host.or(cli.host)),
        Some(Command::Completions { shell }) => {
            clap_complete::generate(shell, &mut Cli::command(), "mr", &mut std::io::stdout());
            return Ok(());
        }
        _ => {}
    }
    let ctx = Ctx::open(cli.host.as_deref(), cli.json)?;
    match cli.command {
        Some(Command::Whoami) => commands::whoami::run(&ctx).await,
        Some(Command::List(args)) => commands::list::run(&ctx, args).await,
        Some(Command::Show(args)) => commands::show::run(&ctx, args).await,
        Some(Command::Diff(args)) => commands::diff::run(&ctx, args).await,
        Some(Command::Tui) | None => gitlabmr::tui::run(ctx).await,
        Some(Command::Login(_) | Command::Logout { .. } | Command::Completions { .. }) => unreachable!("handled above"),
    }
}
