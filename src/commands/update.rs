use crate::cli::UpdateArgs;
use crate::render::{Style, Theme};
use crate::update::{Decision, Source, decide};
use crate::version;
use anyhow::Result;

pub fn run(args: &UpdateArgs) -> Result<()> {
    let theme = Theme::detect();
    let source = Source::find()?;
    let latest = if args.force { None } else { latest(&source, &theme) };
    match decide(version::COMMIT, latest.as_deref(), args.force) {
        Decision::UpToDate => println!("{} already up to date ({})", theme.paint("✓", Style::Ok), version::label()),
        Decision::Install => install(&source, &theme)?,
    }
    Ok(())
}

fn latest(source: &Source, theme: &Theme) -> Option<String> {
    source.latest().inspect_err(|err| eprintln!("{} could not check the latest version: {err}", theme.paint("!", Style::Warn))).ok()
}

fn install(source: &Source, theme: &Theme) -> Result<()> {
    if source.dirty() {
        eprintln!("{} {source} has uncommitted changes; they go into this build", theme.paint("!", Style::Warn));
    }
    println!("{} installing mr from {source}…", theme.paint("→", Style::Accent));
    source.install()?;
    println!("{} updated, run `mr --version` to see it", theme.paint("✓", Style::Ok));
    Ok(())
}
