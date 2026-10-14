use crate::cache::Cache;
use crate::cli::UsageArgs;
use crate::config::Config;
use crate::render::{Style, Theme};
use crate::usage::{self, Report};
use anyhow::Result;
use std::fmt::Write as _;

/// Prints what the `[usage]` counts say about the window asked for.
pub fn run(args: &UsageArgs, json: bool) -> Result<()> {
    let window = usage::since(&args.since)?;
    let enabled = Config::load()?.usage.enabled;
    let report = usage::report(&usage::read(&Cache::shared()), chrono::Local::now().date_naive(), window);
    if json {
        return crate::ctx::emit(&report);
    }
    print!("{}", text(&report, &args.since, enabled, Theme::detect()));
    Ok(())
}

/// The report as plain terminal text, section by section; empty sections are left out.
fn text(report: &Report, since: &str, enabled: bool, theme: Theme) -> String {
    let mut out = String::new();
    let head = if since == "all" { "every recorded day".to_owned() } else { format!("the last {since}") };
    let _ = writeln!(out, "{}", theme.paint(&format!("revu usage · {head} · {} days counted", report.days), Style::Bold));
    if !enabled {
        let _ =
            writeln!(out, "{}", theme.paint("counting is off: set `[usage] enabled = true` in ~/.config/revu/config.toml", Style::Warn));
    }
    if report.days == 0 {
        return out;
    }
    if report.old_days > 0 {
        let old =
            format!("{} of these days were counted before revu stopped counting idle time: TIME and HINTS leave them out", report.old_days);
        let _ = writeln!(out, "{}", theme.paint(&old, Style::Dim));
    }
    section(&mut out, theme, "NEVER USED");
    for (group, names) in &report.never {
        let _ = writeln!(out, "  {}  {}", theme.paint(&format!("{group:<18}"), Style::Dim), names.join("  "));
    }
    if !report.rarely.is_empty() {
        section(&mut out, theme, "RARELY USED (2 times or less)");
        let rarely: Vec<String> = report.rarely.iter().map(|(name, n)| format!("{name} {n}")).collect();
        let _ = writeln!(out, "  {}", rarely.join(" · "));
    }
    if !report.most.is_empty() {
        section(&mut out, theme, "MOST USED");
        for (name, n) in &report.most {
            let _ = writeln!(out, "  {name:<22}{}", theme.paint(&format!("{n:>6}"), Style::Accent));
        }
    }
    if !report.screens.is_empty() {
        section(&mut out, theme, "TIME");
        for (screen, seconds) in &report.screens {
            let _ = writeln!(out, "  {screen:<22}{:>6}", usage::duration(*seconds));
        }
    }
    if !report.hints.is_empty() {
        section(&mut out, theme, "HINTS");
        for (_, n, hint) in &report.hints {
            let _ = writeln!(out, "  {} {hint}", theme.paint(&format!("{n}×"), Style::Warn));
        }
    }
    out
}

fn section(out: &mut String, theme: Theme, title: &str) {
    let _ = writeln!(out, "\n{}", theme.paint(title, Style::Bold));
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn report() -> Report {
        let days = usage::parse(
            "{\"date\":\"2026-09-23\",\"actions\":{\"next_thread\":1},\"screens\":{\"diff\":36000}}\n\
             {\"date\":\"2026-09-24\",\"version\":2,\"actions\":{\"next_thread\":40,\"zen\":2},\"screens\":{\"diff\":4320},\"hints\":{\"long_walk\":3}}\n",
        );
        usage::report(&days, "2026-09-24".parse().unwrap(), Some(30))
    }

    #[test]
    fn the_report_reads_section_by_section() {
        let text = text(&report(), "30d", true, Theme::plain());
        assert!(text.starts_with("revu usage · the last 30d · 2 days counted\n"), "{text}");
        assert!(text.contains("1 of these days were counted before revu stopped counting idle time"), "{text}");
        let order: Vec<usize> =
            ["NEVER USED", "RARELY USED", "MOST USED", "TIME", "HINTS"].iter().map(|s| text.find(&format!("\n{s}")).expect(s)).collect();
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{text}");
        assert!(text.contains("zen 2") && text.contains(&format!("{:<22}{:>6}", "next_thread", 41)) && text.contains("1h 12m"), "{text}");
        assert!(!text.contains("11h"), "the old day's ten hours are left out: {text}");
        assert!(text.contains("3× walked 15 lines"), "{text}");
        assert!(!text.contains("counting is off"));
    }

    #[test]
    fn an_empty_log_says_how_to_switch_it_on() {
        let text = text(&usage::report(&[], "2026-09-24".parse().unwrap(), Some(30)), "30d", false, Theme::plain());
        assert!(text.contains("0 days counted") && text.contains("[usage] enabled = true"), "{text}");
        assert!(!text.contains("NEVER USED"));
    }
}
