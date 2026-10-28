#![expect(clippy::let_underscore_must_use, reason = "`writeln!` into a `String` cannot fail")]
use crate::cache::Cache;
use crate::cli::UsageArgs;
use crate::config::Config;
use crate::render::{Style, Theme};
use crate::usage;
use anyhow::{Result, anyhow};
use std::fmt::Write as _;
use vtui::usage::{RARELY, Report, Window};

/// Prints what the `[usage]` counts say about the window asked for.
pub fn run(args: &UsageArgs, json: bool) -> Result<()> {
    let window: Window = args.since.parse().map_err(|err| anyhow!("--since {err}"))?;
    let enabled = Config::load()?.usage.enabled;
    let report = usage::rules().report(&usage::read(&Cache::shared()), chrono::Local::now().date_naive(), window);
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
    for unused in &report.never {
        let _ = writeln!(out, "  {}  {}", theme.paint(&format!("{:<18}", unused.group), Style::Dim), unused.actions.join("  "));
    }
    if !report.rarely.is_empty() {
        section(&mut out, theme, &format!("RARELY USED ({RARELY} times or less)"));
        let rarely: Vec<String> = report.rarely.iter().map(|count| format!("{} {}", count.action, count.count)).collect();
        let _ = writeln!(out, "  {}", rarely.join(" · "));
    }
    if !report.most.is_empty() {
        section(&mut out, theme, "MOST USED");
        for count in &report.most {
            let _ = writeln!(out, "  {:<22}{}", count.action, theme.paint(&format!("{:>6}", count.count), Style::Accent));
        }
    }
    if !report.screens.is_empty() {
        section(&mut out, theme, "TIME");
        for time in &report.screens {
            let _ = writeln!(out, "  {:<22}{:>6}", time.screen, vtui::usage::duration(time.seconds));
        }
    }
    if !report.hints.is_empty() {
        section(&mut out, theme, "HINTS");
        for hinted in &report.hints {
            let _ = writeln!(out, "  {} {}", theme.paint(&format!("{}×", hinted.count), Style::Warn), hinted.text);
        }
    }
    out
}

fn section(out: &mut String, theme: Theme, title: &str) {
    let _ = writeln!(out, "\n{}", theme.paint(title, Style::Bold));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> Report {
        let days = usage::rules().parse(
            "{\"date\":\"2026-09-23\",\"actions\":{\"next_thread\":1},\"screens\":{\"diff\":36000}}\n\
             {\"date\":\"2026-09-24\",\"version\":2,\"actions\":{\"next_thread\":40,\"zen\":2},\"screens\":{\"diff\":4320},\"hints\":{\"long_walk\":3}}\n",
        );
        usage::rules().report(&days, "2026-09-24".parse().unwrap(), Window::Days(30))
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
        let text = text(&usage::rules().report(&[], "2026-09-24".parse().unwrap(), Window::Days(30)), "30d", false, Theme::plain());
        assert!(text.contains("0 days counted") && text.contains("[usage] enabled = true"), "{text}");
        assert!(!text.contains("NEVER USED"));
    }
}
