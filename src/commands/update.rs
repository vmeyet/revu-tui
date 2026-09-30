use crate::cli::UpdateArgs;
use crate::render::{Style, Theme};
use crate::update::{self, Decision, FORMULA, REPO, decide};
use crate::version;
use anyhow::Result;

/// Rebuilds and installs `revu` when the repo moved past the running commit, or hands over to brew when brew installed it.
pub fn run(args: &UpdateArgs) -> Result<()> {
    let theme = Theme::detect();
    if update::installed_by_brew() {
        return upgrade_with_brew(theme, args.force);
    }
    let latest = if args.force { None } else { latest(theme) };
    match decide(version::COMMIT, latest.as_deref(), args.force) {
        Decision::UpToDate => println!("{} already up to date ({})", theme.paint("✓", Style::Ok), version::label()),
        Decision::Install => {
            println!("{} installing the latest revu from {REPO}…", theme.paint("→", Style::Accent));
            update::install()?;
            println!("{} updated, run `revu --version` to see it", theme.paint("✓", Style::Ok));
        }
    }
    Ok(())
}

/// brew ships tagged releases only, so the latest commit says nothing about what brew would install.
fn upgrade_with_brew(theme: Theme, force: bool) -> Result<()> {
    println!("{} upgrading revu with brew ({FORMULA})…", theme.paint("→", Style::Accent));
    update::brew(force)?;
    println!("{} done, run `revu --version` to see it", theme.paint("✓", Style::Ok));
    Ok(())
}

fn latest(theme: Theme) -> Option<String> {
    update::latest().inspect_err(|err| eprintln!("{} could not check the latest version: {err}", theme.paint("!", Style::Warn))).ok()
}
