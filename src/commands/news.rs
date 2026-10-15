#![expect(clippy::let_underscore_must_use, reason = "`writeln!` into a `String` cannot fail")]
use crate::changelog::{Changelog, Entry, Position, Section};
use crate::cli::NewsArgs;
use crate::render::{Style, Theme};
use anyhow::Result;
use std::fmt::Write as _;

const AGAIN: &str = "Read it again with `revu news`, or with `:news` inside revu.";

/// Prints what changed in revu, newest version first; with `--since`, only what came after that version.
pub fn run(args: &NewsArgs, json: bool) -> Result<()> {
    let log = Changelog::embedded()?;
    let sections = log.since(args.since.unwrap_or(Position::START));
    if json {
        return crate::ctx::emit(&sections);
    }
    let theme = Theme::detect();
    let page = match args.since {
        Some(seen) if args.after_update => after_update(&sections, seen, theme),
        Some(seen) if sections.is_empty() => format!("Nothing new since {}.\n", seen.version),
        _ => text(&sections, theme),
    };
    print!("{page}");
    Ok(())
}

/// What `revu update` ends with: nothing when nothing is new, else the sections and how to read them again.
fn after_update(sections: &[&Section], seen: Position, theme: Theme) -> String {
    if sections.is_empty() {
        return String::new();
    }
    let title = theme.paint(&format!("New since {}", seen.version), Style::Bold);
    format!("\n{title}\n\n{}\n{}\n", text(sections, theme), code(AGAIN, Style::Dim, theme))
}

/// The sections as terminal text: a header per version, New then Fixed, three short lines per entry.
fn text(sections: &[&Section], theme: Theme) -> String {
    sections.iter().map(|section| section_text(section, theme)).collect::<Vec<_>>().join("\n")
}

fn section_text(section: &Section, theme: Theme) -> String {
    let date = section.date.as_ref().map(|date| format!(" · {date}")).unwrap_or_default();
    let mut out = format!("{}{}\n", theme.paint(&section.title(), Style::Accent), theme.paint(&date, Style::Dim));
    for (name, entries) in section.groups() {
        let _ = writeln!(out, "  {}", theme.paint(name, Style::Dim));
        out.extend(entries.iter().map(|entry| entry_text(entry, theme)));
    }
    out
}

fn entry_text(entry: &Entry, theme: Theme) -> String {
    let rest = [Some(&entry.place), entry.how.as_ref()].into_iter().flatten();
    let rest: String = rest.map(|line| format!("    {}\n", code(line, Style::Plain, theme))).collect();
    format!("  • {}\n{rest}", code(&entry.what, Style::Bold, theme))
}

/// A sentence in `style` with what its backticks hold in the accent colour; without colours the backticks stay.
fn code(sentence: &str, style: Style, theme: Theme) -> String {
    if !theme.color {
        return sentence.to_owned();
    }
    let paint = |(i, part): (usize, &str)| theme.paint(part, if i % 2 == 1 { Style::Accent } else { style });
    sentence.split('`').enumerate().map(paint).collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::changelog::tests::sample;

    fn since(version: &str) -> Position {
        version.parse().unwrap()
    }

    #[test]
    fn a_version_reads_as_its_header_then_new_then_fixed() {
        let log = sample();
        let page = text(&log.since(since("0.5.0")), Theme::plain());
        let expected = "\
Unreleased
  New
  • Pick a row with the mouse.
    In the queue.
    Click a row to pick it.

0.6.0 · 2026-10-06
  New
  • Read a Markdown file as a page.
    In the diff, on a Markdown file.
    `v` opens the page, and `v` again goes back.
  • Move a full page at a time.
    In the queue and the diff.
    `ctrl-f` goes down and `ctrl-b` goes up.
  Fixed
  • A moved signature no longer shows as changed.
    In the outline that `O` opens.
";
        assert_eq!(page, expected);
    }

    #[test]
    fn the_update_tail_names_the_version_left_and_says_how_to_read_it_again() {
        let log = sample();
        let tail = after_update(&log.since(since("0.5.0+3")), since("0.5.0+3"), Theme::plain());
        assert!(tail.starts_with("\nNew since 0.5.0\n\nUnreleased\n"), "{tail}");
        assert!(tail.ends_with("\nRead it again with `revu news`, or with `:news` inside revu.\n"), "{tail}");
        assert_eq!(tail.matches("0.6.0 · 2026-10-06").count(), 1);
    }

    #[test]
    fn the_update_tail_is_silent_when_nothing_is_new() {
        let log = sample();
        assert_eq!(after_update(&log.since(log.position()), log.position(), Theme::plain()), "");
    }

    #[test]
    fn colours_replace_the_backticks_around_a_key() {
        let painted = code("`v` opens the page.", Style::Plain, Theme { color: true });
        assert!(painted.contains("\x1b[36mv\x1b[39m") && !painted.contains('`'), "{painted:?}");
        assert_eq!(code("`v` opens the page.", Style::Bold, Theme::plain()), "`v` opens the page.");
    }
}
