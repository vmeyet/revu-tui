//! What changed for the people who use revu: `CHANGELOG.md` is built into the binary, read into
//! one section per version, and cut at the point a reader last saw.
use anyhow::{Context, Result, bail, ensure};
use serde::{Serialize, Serializer};
use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

const TEXT: &str = include_str!("../CHANGELOG.md");
const UNRELEASED: &str = "Unreleased";
const SECTION: &str = "## ";
const GROUP: &str = "### ";
const ENTRY: &str = "- ";
const INDENT: &str = "  ";
const DATE: &str = "%Y-%m-%d";

/// A release number, `0.6.0`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(u64, u64, u64);

impl FromStr for Version {
    type Err = anyhow::Error;

    fn from_str(text: &str) -> Result<Self> {
        let numbers: Vec<u64> = text.trim_start_matches('v').split('.').map(str::parse).collect::<Result<_, _>>().unwrap_or_default();
        match numbers.as_slice() {
            [major, minor, patch] => Ok(Self(*major, *minor, *patch)),
            _ => bail!("`{text}` is not a version, write it like 0.6.0"),
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// How far a binary or a reader got: a version, and how many `Unreleased` entries past it, written `0.6.0+2`.
/// A build from `main` keeps the number of the last release, so only that count tells two of them apart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position {
    pub version: Version,
    pub unreleased: usize,
}

impl Position {
    /// Before every release: everything is new past it.
    pub const START: Self = Self { version: Version(0, 0, 0), unreleased: 0 };
}

impl FromStr for Position {
    type Err = anyhow::Error;

    fn from_str(text: &str) -> Result<Self> {
        let (version, unreleased) = text.split_once('+').unwrap_or((text, "0"));
        let unreleased = unreleased.parse().with_context(|| format!("`{text}` is not a version, write it like 0.6.0"))?;
        Ok(Self { version: version.parse()?, unreleased })
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.unreleased {
            0 => write!(f, "{}", self.version),
            count => write!(f, "{}+{count}", self.version),
        }
    }
}

/// One change: what it is, where it applies, and how to use it. A fix may have nothing to press.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub what: String,
    #[serde(rename = "where")]
    pub place: String,
    pub how: Option<String>,
}

/// What one version brought; `version` is `None` for what waits on `main` for the next release.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Section {
    #[serde(serialize_with = "title_of")]
    pub version: Option<Version>,
    pub date: Option<String>,
    pub new: Vec<Entry>,
    pub fixed: Vec<Entry>,
    pub removed: Vec<Entry>,
}

impl Section {
    pub fn title(&self) -> String {
        title(self.version)
    }

    pub fn count(&self) -> usize {
        self.new.len() + self.fixed.len() + self.removed.len()
    }

    /// The groups that hold something, in the order they read: New, Fixed, then Removed.
    pub fn groups(&self) -> impl Iterator<Item = (&'static str, &[Entry])> {
        let groups = [("New", self.new.as_slice()), ("Fixed", self.fixed.as_slice()), ("Removed", self.removed.as_slice())];
        groups.into_iter().filter(|(_, entries)| !entries.is_empty())
    }
}

fn title(version: Option<Version>) -> String {
    version.map_or_else(|| UNRELEASED.to_owned(), |version| version.to_string())
}

#[expect(clippy::ref_option, reason = "serde hands the field over by reference")]
fn title_of<S: Serializer>(version: &Option<Version>, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&title(*version))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Group {
    New,
    Fixed,
    Removed,
}

/// Every section, newest first, as the binary running `running` carries them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changelog {
    pub running: Version,
    pub sections: Vec<Section>,
}

impl Changelog {
    /// The changelog this binary was built with.
    pub fn embedded() -> Result<Self> {
        Self::parse(TEXT, env!("CARGO_PKG_VERSION").parse()?)
    }

    pub fn parse(text: &str, running: Version) -> Result<Self> {
        let lines: Vec<&str> = text.lines().collect();
        let sections = parts(&lines, SECTION).iter().skip(1).map(|part| section(part)).collect::<Result<Vec<_>>>()?;
        check_order(&sections, running)?;
        Ok(Self { running, sections })
    }

    /// Where this binary stands.
    pub fn position(&self) -> Position {
        Position { version: self.running, unreleased: self.unreleased().map_or(0, Section::count) }
    }

    /// Where a reader nothing is remembered about is taken to stand: on the release before this one.
    pub fn previous(&self) -> Position {
        let version = self.sections.iter().filter_map(|s| s.version).find(|v| *v < self.running);
        version.map_or(Position::START, |version| Position { version, unreleased: 0 })
    }

    /// The sections that came after `seen`, newest first; `Unreleased` only while it holds something.
    pub fn since(&self, seen: Position) -> Vec<&Section> {
        let unreleased = self.unreleased().filter(|_| self.unreleased_since(seen) > 0);
        let released = self.sections.iter().filter(|s| s.version.is_some_and(|version| version > seen.version));
        unreleased.into_iter().chain(released).collect()
    }

    /// How many entries came after `seen`.
    pub fn count_since(&self, seen: Position) -> usize {
        let released: usize = self.since(seen).into_iter().filter(|s| s.version.is_some()).map(Section::count).sum();
        released + self.unreleased_since(seen)
    }

    fn unreleased(&self) -> Option<&Section> {
        self.sections.iter().find(|s| s.version.is_none())
    }

    /// `Unreleased` entries are not numbered, so on the same version only their count says how many are new.
    fn unreleased_since(&self, seen: Position) -> usize {
        let here = self.position();
        match here.version.cmp(&seen.version) {
            Ordering::Greater => here.unreleased,
            Ordering::Equal => here.unreleased.saturating_sub(seen.unreleased),
            Ordering::Less => 0,
        }
    }
}

/// `lines` cut before each line that starts with `marker`; the first part is what stands before any.
fn parts<'a>(lines: &[&'a str], marker: &str) -> Vec<Vec<&'a str>> {
    let mut parts = vec![vec![]];
    for line in lines {
        if line.starts_with(marker) {
            parts.push(vec![]);
        }
        if let Some(part) = parts.last_mut() {
            part.push(*line);
        }
    }
    parts
}

fn section(lines: &[&str]) -> Result<Section> {
    let heading = lines.first().and_then(|line| line.strip_prefix(SECTION)).unwrap_or_default().trim();
    let (version, date) = heading_of(heading)?;
    let groups = groups(&lines[1..]).with_context(|| format!("under `{heading}`"))?;
    let entries = |wanted: Group| groups.iter().filter(|(group, _)| *group == wanted).flat_map(|(_, entries)| entries.clone()).collect();
    Ok(Section { version, date, new: entries(Group::New), fixed: entries(Group::Fixed), removed: entries(Group::Removed) })
}

fn groups(lines: &[&str]) -> Result<Vec<(Group, Vec<Entry>)>> {
    let written = parts(lines, GROUP);
    ensure_blank(&written[0])?;
    written[1..].iter().map(|group| entries(group)).collect()
}

/// `Unreleased`, or `0.6.0 - 2026-10-06`: the heading cargo-dist looks for to fill a release's notes.
fn heading_of(heading: &str) -> Result<(Option<Version>, Option<String>)> {
    if heading == UNRELEASED {
        return Ok((None, None));
    }
    let (version, date) = heading.split_once(" - ").with_context(|| format!("`{heading}` should read `0.6.0 - 2026-10-06`"))?;
    chrono::NaiveDate::parse_from_str(date, DATE).with_context(|| format!("`{date}` is not a date like 2026-10-06"))?;
    Ok((Some(version.parse()?), Some(date.to_owned())))
}

fn entries(lines: &[&str]) -> Result<(Group, Vec<Entry>)> {
    let group = match lines.first().and_then(|line| line.strip_prefix(GROUP)).map(str::trim) {
        Some("New") => Group::New,
        Some("Fixed") => Group::Fixed,
        Some("Removed") => Group::Removed,
        other => bail!("`{}` is not a group: use `New`, `Fixed` or `Removed`", other.unwrap_or_default()),
    };
    let written = parts(&lines[1..], ENTRY);
    ensure_blank(&written[0])?;
    Ok((group, written[1..].iter().map(|lines| entry(lines, group)).collect::<Result<_>>()?))
}

/// `- **What.**`, then where, then how, each one sentence on its own line; a fix may stop at where.
fn entry(lines: &[&str], group: Group) -> Result<Entry> {
    let written: Vec<&str> = lines.iter().copied().filter(|line| !line.trim().is_empty()).collect();
    let shape = || format!("not `- **What.**`, then where, then how, one sentence per line: {}", written.join(" / "));
    let (what, place, how) = match written.as_slice() {
        [what, place] if group != Group::New => (what, place, None),
        [what, place, how] => (what, place, Some(how)),
        _ => bail!(shape()),
    };
    let what = what.strip_prefix("- **").and_then(|what| what.strip_suffix("**")).with_context(shape)?;
    let rest = [Some(place), how].into_iter().flatten().map(|line| line.strip_prefix(INDENT)).collect::<Option<Vec<_>>>();
    let sentences = std::iter::once(what).chain(rest.with_context(shape)?).map(str::trim).collect::<Vec<_>>();
    ensure!(sentences.iter().all(|sentence| is_one_sentence(sentence)), shape());
    Ok(Entry { what: sentences[0].to_owned(), place: sentences[1].to_owned(), how: sentences.get(2).map(|how| (*how).to_owned()) })
}

/// Ends with a period and starts no second sentence; what sits in backticks counts as one word.
fn is_one_sentence(text: &str) -> bool {
    let words: String = text.split('`').enumerate().map(|(i, part)| if i % 2 == 1 { "Code" } else { part }).collect();
    let starts_another = words.split(". ").skip(1).any(|next| next.starts_with(char::is_uppercase));
    words.ends_with('.') && !starts_another
}

fn ensure_blank(lines: &[&str]) -> Result<()> {
    match lines.iter().find(|line| !line.trim().is_empty()) {
        Some(stray) => bail!("`{stray}` belongs to no entry"),
        None => Ok(()),
    }
}

/// `Unreleased` first, then releases from the newest down, none past the version the binary runs.
fn check_order(sections: &[Section], running: Version) -> Result<()> {
    let late = sections.iter().skip(1).any(|s| s.version.is_none());
    ensure!(!late, "`{UNRELEASED}` goes on top");
    let versions: Vec<Version> = sections.iter().filter_map(|s| s.version).collect();
    ensure!(versions.windows(2).all(|pair| pair[0] > pair[1]), "versions go newest first, each once");
    ensure!(versions.first().is_none_or(|newest| *newest <= running), "a section is newer than the version in Cargo.toml");
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    pub(crate) const SAMPLE: &str = "\
# What's new

Words before the first section are for the page on the forge.

## Unreleased

### New

- **Pick a row with the mouse.**
  In the queue.
  Click a row to pick it.

## 0.6.0 - 2026-10-06

### New

- **Read a Markdown file as a page.**
  In the diff, on a Markdown file.
  `v` opens the page, and `v` again goes back.
- **Move a full page at a time.**
  In the queue and the diff.
  `ctrl-f` goes down and `ctrl-b` goes up.

### Fixed

- **A moved signature no longer shows as changed.**
  In the outline that `O` opens.

## 0.5.0 - 2026-10-02

### New

- **Keep an MR on top of the queue.**
  In the queue, under PINNED.
  `b` pins or unpins it.
";

    pub(crate) fn sample() -> Changelog {
        Changelog::parse(SAMPLE, version("0.6.0")).unwrap()
    }

    fn version(text: &str) -> Version {
        text.parse().unwrap()
    }

    fn position(text: &str) -> Position {
        text.parse().unwrap()
    }

    fn titles(sections: &[&Section]) -> Vec<String> {
        sections.iter().map(|s| s.title()).collect()
    }

    fn with_entry(entry: &str) -> Result<Changelog> {
        Changelog::parse(&format!("## 0.6.0 - 2026-10-06\n\n### New\n\n{entry}\n"), version("0.6.0"))
    }

    #[test]
    fn sections_come_newest_first_with_their_date_and_groups() {
        let log = sample();
        let shape: Vec<(String, Option<&str>, usize, usize)> =
            log.sections.iter().map(|s| (s.title(), s.date.as_deref(), s.new.len(), s.fixed.len())).collect();
        assert_eq!(
            shape,
            [
                ("Unreleased".to_owned(), None, 1, 0),
                ("0.6.0".to_owned(), Some("2026-10-06"), 2, 1),
                ("0.5.0".to_owned(), Some("2026-10-02"), 1, 0)
            ]
        );
    }

    #[test]
    fn an_entry_reads_as_what_where_and_how() {
        let log = sample();
        let prose = &log.sections[1].new[0];
        assert_eq!(prose.what, "Read a Markdown file as a page.");
        assert_eq!(prose.place, "In the diff, on a Markdown file.");
        assert_eq!(prose.how.as_deref(), Some("`v` opens the page, and `v` again goes back."));
        assert_eq!(log.sections[1].fixed[0].how, None, "a fix may have nothing to press");
    }

    #[test]
    fn a_removal_reads_after_the_fixes_and_may_say_what_to_use_instead() {
        let text = "## Unreleased\n\n### Fixed\n\n- **A fix.**\n  In zen.\n\n### Removed\n\n- **`S` in the queue.**\n  In the queue.\n\n- **`J` and `K`.**\n  In the thread pane.\n  `j` and `k` walk the notes.\n";
        let log = Changelog::parse(text, version("0.6.0")).unwrap();
        let unreleased = &log.sections[0];
        assert_eq!((unreleased.removed.len(), unreleased.count()), (2, 3));
        assert_eq!(unreleased.removed[1].how.as_deref(), Some("`j` and `k` walk the notes."));
        assert_eq!(unreleased.groups().map(|(name, _)| name).collect::<Vec<_>>(), ["Fixed", "Removed"]);
    }

    #[test]
    fn an_entry_out_of_shape_is_refused() {
        let refused = [
            ("- **New thing.**\n  In the diff.", "a new entry says how"),
            ("- New thing.\n  In the diff.\n  Press `v`.", "what is in bold"),
            ("- **New thing.**\nIn the diff.\n  Press `v`.", "the lines after the first are indented"),
            ("- **New thing.**\n  In the diff. In zen too.\n  Press `v`.", "one sentence per line"),
            ("- **New thing.**\n  In the diff.\n  Press `v`. `q` leaves.", "one sentence per line, a key first too"),
            ("- **New thing**\n  In the diff.\n  Press `v`.", "a sentence ends with a period"),
            ("- **New thing.**\n  In the diff.\n  Press `v`.\n  Then `q`.", "three lines at most"),
        ];
        for (entry, why) in refused {
            assert!(with_entry(entry).is_err(), "{why}: {entry}");
        }
        assert!(
            with_entry("- **New thing.**\n  In `[queue.rules] enabled`, at v1.2.\n  Press `v`.").is_ok(),
            "periods inside a sentence are fine"
        );
    }

    #[test]
    fn a_page_out_of_shape_is_refused() {
        let entry = "- **Thing.**\n  In the diff.\n  Press `v`.\n";
        let refused = [
            (format!("## 0.6.0\n\n### New\n\n{entry}"), "a release has its date"),
            (format!("## 0.6.0 - yesterday\n\n### New\n\n{entry}"), "the date is a date"),
            (format!("## 0.6.0 - 2026-10-06\n\n### Changed\n\n{entry}"), "two groups only"),
            (format!("## 0.6.0 - 2026-10-06\n\nA word.\n\n### New\n\n{entry}"), "no text outside an entry"),
            (format!("## 0.5.0 - 2026-10-02\n\n## 0.6.0 - 2026-10-06\n\n### New\n\n{entry}"), "newest first"),
            (format!("## 0.6.0 - 2026-10-06\n\n## Unreleased\n\n### New\n\n{entry}"), "Unreleased on top"),
            (format!("## 0.7.0 - 2026-10-20\n\n### New\n\n{entry}"), "no section past the running version"),
        ];
        for (text, why) in refused {
            assert!(Changelog::parse(&text, version("0.6.0")).is_err(), "{why}:\n{text}");
        }
    }

    #[test]
    fn since_keeps_the_sections_newer_than_the_version_given() {
        let log = sample();
        assert_eq!(titles(&log.since(position("0.5.0"))), ["Unreleased", "0.6.0"], "an older version");
        assert_eq!(titles(&log.since(position("0.6.0"))), ["Unreleased"], "the running version: only what main added");
        assert_eq!(titles(&log.since(position("0.7.0"))), [] as [&str; 0], "a newer version");
        assert_eq!(titles(&log.since(position("0.5.5"))), ["Unreleased", "0.6.0"], "a version that was never released");
        assert_eq!(titles(&log.since(position("0.1.0"))), ["Unreleased", "0.6.0", "0.5.0"], "a version from before the page");
    }

    #[test]
    fn a_build_from_main_is_its_version_plus_the_unreleased_entries_it_carries() {
        let log = sample();
        assert_eq!(log.position(), position("0.6.0+1"));
        assert_eq!(titles(&log.since(position("0.6.0+1"))), [] as [&str; 0], "the same build again");
        assert_eq!(titles(&log.since(position("0.6.0+3"))), [] as [&str; 0], "an entry was taken back: nothing new");
        assert_eq!(titles(&log.since(position("0.5.0+4"))), ["Unreleased", "0.6.0"], "a count from an older version says nothing here");
        let released = Changelog::parse(&SAMPLE.replace("## Unreleased", "## 0.7.0 - 2026-10-20"), version("0.7.0")).unwrap();
        assert_eq!(released.position(), position("0.7.0"));
        assert_eq!(titles(&released.since(position("0.6.0+1"))), ["0.7.0"], "what was unreleased comes back under its version");
    }

    #[test]
    fn the_count_is_every_entry_past_a_version_and_only_the_added_ones_on_main() {
        let log = sample();
        assert_eq!(log.count_since(position("0.5.0")), 4);
        assert_eq!(log.count_since(position("0.6.0")), 1);
        assert_eq!(log.count_since(position("0.6.0+1")), 0);
        assert_eq!(log.count_since(position("0.7.0")), 0);
        let grown = Changelog::parse(
            &SAMPLE.replace("## 0.6.0 - 2026-10-06\n\n### New", "### Fixed\n\n- **A fix.**\n  In zen.\n\n## 0.6.0 - 2026-10-06\n\n### New"),
            version("0.6.0"),
        );
        assert_eq!(grown.unwrap().count_since(position("0.6.0+1")), 1, "one entry joined Unreleased since");
    }

    #[test]
    fn a_reader_never_seen_stands_on_the_release_before() {
        assert_eq!(sample().previous(), position("0.5.0"));
        assert_eq!(Changelog::parse(SAMPLE, version("0.6.1")).unwrap().previous(), position("0.6.0"));
        let first = Changelog::parse("## 0.2.0 - 2026-09-30\n", version("0.2.0")).unwrap();
        assert_eq!(first.previous(), position("0.0.0"));
    }

    #[test]
    fn a_position_is_written_as_a_version_and_read_back() {
        for text in ["0.6.0", "0.6.0+2", "12.0.3"] {
            assert_eq!(position(text).to_string(), text);
        }
        assert_eq!(position("v0.6.0"), position("0.6.0"), "the tag's v is fine");
        assert!(version("0.10.0") > version("0.9.9"), "numbers, not text");
        for text in ["banana", "0.6", "0.6.0.1", "0.6.0+many", ""] {
            assert!(text.parse::<Position>().is_err(), "{text}");
        }
    }

    #[test]
    fn a_section_serializes_with_where_and_the_unreleased_name() {
        let json = serde_json::to_value(&sample().sections).unwrap();
        assert_eq!(json[0]["version"], "Unreleased");
        assert_eq!(json[0]["date"], serde_json::Value::Null);
        assert_eq!(json[1]["version"], "0.6.0");
        assert_eq!(json[1]["date"], "2026-10-06");
        assert_eq!(
            json[1]["new"][0],
            serde_json::json!({
                "what": "Read a Markdown file as a page.",
                "where": "In the diff, on a Markdown file.",
                "how": "`v` opens the page, and `v` again goes back.",
            })
        );
        assert_eq!(json[1]["fixed"][0]["how"], serde_json::Value::Null);
    }

    #[test]
    fn every_entry_of_the_shipped_page_says_what_where_and_how() {
        let log = Changelog::embedded().unwrap_or_else(|e| panic!("CHANGELOG.md: {e:#}"));
        assert!(log.sections.iter().any(|s| s.version.is_some()), "at least one release");
        let listed = TEXT.lines().filter(|line| line.starts_with(ENTRY)).count();
        assert_eq!(log.sections.iter().map(Section::count).sum::<usize>(), listed, "every `- ` line is an entry");
    }

    /// A version bump moves `Unreleased` under the new number: with entries still waiting and no
    /// section for the version in Cargo.toml, the release would ship without its notes.
    #[test]
    fn a_version_bump_moves_unreleased_into_its_section() {
        let log = Changelog::embedded().unwrap();
        let has_section = log.sections.iter().any(|s| s.version == Some(log.running));
        assert!(
            has_section || log.position().unreleased == 0,
            "Cargo.toml says {}: rename `## Unreleased` in CHANGELOG.md to `## {} - <date>`, and start a new `## Unreleased`",
            log.running,
            log.running
        );
    }
}
