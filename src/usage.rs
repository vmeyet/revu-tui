//! What revu is used for, counted on this machine only: which actions, how long on each screen,
//! and a few habits a faster key would replace. Opt-in with `[usage] enabled = true`.
//! Nothing about the MRs is kept: no titles, paths, text, hosts or projects, only action names.
use crate::act::{self, Section};
use crate::cache::Cache;
use anyhow::{Result, bail};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

/// The file, at the cache root; deleting it starts the counts again.
pub const FILE: &str = "usage.jsonl";

/// A habit a faster key replaces, with what to try instead.
pub const HINTS: &[(&str, &str)] = &[
    ("long_walk", "walked 15 lines or more with j or k to a hunk or a thread; ]c (next hunk) or ]n (next thread) jumps there"),
    ("queue_after_zen", "left zen to open another MR from the queue; [m and ]m change MR, in zen too"),
    ("palette_number", "typed a bare number in the search; !42 (or #42 on GitHub) finds that MR directly"),
];

/// Every action the report knows, bindable ones first.
pub fn known() -> Vec<&'static str> {
    act::listed_names()
}

/// How often a hint shows as a toast, over every session; after that only the report counts it.
const HINT_TOASTS: u64 = 3;

/// How time and hints are counted now: time stops a minute after the last input and while the terminal is in the
/// background, and a hint fires only where a jump would have helped. Days counted before it keep only their actions.
const COUNTING: u32 = 2;

/// What one session counted since the last flush: kept in memory, never written by the App itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    actions: BTreeMap<&'static str, u64>,
    screens_ms: BTreeMap<&'static str, u64>,
    hints: BTreeMap<&'static str, u64>,
    shown: BTreeMap<&'static str, u64>,
    shown_ever: BTreeMap<String, u64>,
}

/// What a flush writes: whole seconds only; the milliseconds below a second wait for the next one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub actions: BTreeMap<String, u64>,
    pub screens: BTreeMap<String, u64>,
    pub hints: BTreeMap<String, u64>,
    pub shown: BTreeMap<String, u64>,
}

impl Tally {
    /// A session's tally, knowing how often each hint was shown in the sessions before.
    pub fn after(days: &[Day]) -> Self {
        let mut shown_ever = BTreeMap::new();
        for day in days {
            add(&mut shown_ever, &day.shown);
        }
        Self { shown_ever, ..Self::default() }
    }

    pub fn act(&mut self, name: &'static str) {
        *self.actions.entry(name).or_default() += 1;
    }

    /// Counts a hint, and says whether to show it: yes for its first `HINT_TOASTS` times, every session counted.
    pub fn hint(&mut self, name: &'static str) -> bool {
        *self.hints.entry(name).or_default() += 1;
        let shown = self.shown_ever.entry(name.to_owned()).or_default();
        let is_shown = *shown < HINT_TOASTS;
        if is_shown {
            *shown += 1;
            *self.shown.entry(name).or_default() += 1;
        }
        is_shown
    }

    pub fn spend(&mut self, screen: &'static str, time: Duration) {
        *self.screens_ms.entry(screen).or_default() += u64::try_from(time.as_millis()).unwrap_or(u64::MAX);
    }

    /// The counts to write, leaving behind the part of a second not yet spent on each screen.
    pub fn take(&mut self) -> Option<Counts> {
        let screens: BTreeMap<String, u64> =
            self.screens_ms.iter().filter(|(_, ms)| **ms >= 1000).map(|(s, ms)| ((*s).to_owned(), ms / 1000)).collect();
        for ms in self.screens_ms.values_mut() {
            *ms %= 1000;
        }
        let counts = Counts { actions: owned(&mut self.actions), screens, hints: owned(&mut self.hints), shown: owned(&mut self.shown) };
        let empty = counts.actions.is_empty() && counts.screens.is_empty() && counts.hints.is_empty();
        (!empty).then_some(counts)
    }
}

fn owned(counts: &mut BTreeMap<&'static str, u64>) -> BTreeMap<String, u64> {
    std::mem::take(counts).into_iter().map(|(k, v)| (k.to_owned(), v)).collect()
}

/// One line of the file: everything one local day counted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Day {
    pub date: NaiveDate,
    /// The oldest `COUNTING` that went into the day; lines from before it have none.
    #[serde(default = "first_counting")]
    pub version: u32,
    #[serde(default)]
    pub actions: BTreeMap<String, u64>,
    /// Seconds per screen.
    #[serde(default)]
    pub screens: BTreeMap<String, u64>,
    #[serde(default)]
    pub hints: BTreeMap<String, u64>,
    /// How many hints showed as a toast.
    #[serde(default)]
    pub shown: BTreeMap<String, u64>,
}

impl Day {
    fn new(date: NaiveDate) -> Self {
        Self { date, version: COUNTING, actions: BTreeMap::new(), screens: BTreeMap::new(), hints: BTreeMap::new(), shown: BTreeMap::new() }
    }

    /// Its time and hints were counted the way revu counts them now.
    fn counts_time_and_hints(&self) -> bool {
        self.version >= COUNTING
    }
}

fn first_counting() -> u32 {
    1
}

/// The file's days, oldest first; a line that does not parse is skipped rather than failing.
pub fn parse(text: &str) -> Vec<Day> {
    let mut days: Vec<Day> = text.lines().filter_map(|line| serde_json::from_str(line).ok()).collect();
    days.sort_by_key(|day| day.date);
    days
}

/// The file with `counts` added to `date`'s line: one line per day, however often it is merged.
pub fn merge(text: &str, date: NaiveDate, counts: &Counts) -> Result<String> {
    let mut days = parse(text);
    let index = if let Some(index) = days.iter().position(|day| day.date == date) {
        index
    } else {
        days.push(Day::new(date));
        days.len() - 1
    };
    let day = &mut days[index];
    add(&mut day.actions, &counts.actions);
    add(&mut day.screens, &counts.screens);
    add(&mut day.hints, &counts.hints);
    add(&mut day.shown, &counts.shown);
    days.sort_by_key(|day| day.date);
    let mut out = String::new();
    for day in &days {
        out.push_str(&serde_json::to_string(day)?);
        out.push('\n');
    }
    Ok(out)
}

/// A renamed action counts under its new name.
fn add(into: &mut BTreeMap<String, u64>, from: &BTreeMap<String, u64>) {
    for (name, count) in from {
        *into.entry(crate::keymap::current(name).to_owned()).or_default() += count;
    }
}

/// Adds `counts` to today's line in the cache's usage file, privately and atomically.
pub fn record(cache: &Cache, date: NaiveDate, counts: &Counts) -> Result<()> {
    let text = cache.read_bytes(FILE).map(|bytes| String::from_utf8_lossy(&bytes).into_owned()).unwrap_or_default();
    cache.write_bytes(FILE, merge(&text, date, counts)?.as_bytes())
}

/// Every recorded day in the cache's usage file.
pub fn read(cache: &Cache) -> Vec<Day> {
    cache.read_bytes(FILE).map(|bytes| parse(&String::from_utf8_lossy(&bytes))).unwrap_or_default()
}

/// `30d`, `4w`, `all`: how far back a report looks.
pub fn since(text: &str) -> Result<Option<u32>> {
    if text == "all" {
        return Ok(None);
    }
    let (number, unit) = text.split_at(text.len().saturating_sub(1));
    let Ok(number) = number.parse::<u32>() else { bail!("--since takes a number of days or weeks, like 30d or 4w, or all") };
    match unit {
        "d" => Ok(Some(number)),
        "w" => Ok(Some(number * 7)),
        _ => bail!("--since takes a number of days or weeks, like 30d or 4w, or all"),
    }
}

/// The days of the window, summed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    pub days: usize,
    /// Days counted before `COUNTING`: their time and hints are left out.
    pub old_days: usize,
    pub never: Vec<(String, Vec<String>)>,
    pub rarely: Vec<(String, u64)>,
    pub most: Vec<(String, u64)>,
    pub screens: Vec<(String, u64)>,
    pub hints: Vec<(String, u64, String)>,
}

const RARELY: u64 = 2;
const MOST: usize = 10;

/// Sums the days within `window` days of `today`, and sorts every known action into the sections.
pub fn report(days: &[Day], today: NaiveDate, window: Option<u32>) -> Report {
    let kept: Vec<&Day> = days.iter().filter(|day| window.is_none_or(|w| (today - day.date).num_days() < i64::from(w))).collect();
    let mut actions = BTreeMap::<String, u64>::new();
    let mut screens = BTreeMap::<String, u64>::new();
    let mut hints = BTreeMap::<String, u64>::new();
    for day in &kept {
        add(&mut actions, &day.actions);
    }
    let (counted, old): (Vec<&Day>, Vec<&Day>) = kept.iter().partition(|day| day.counts_time_and_hints());
    for day in &counted {
        add(&mut screens, &day.screens);
        add(&mut hints, &day.hints);
    }
    let known = known();
    let count = |name: &str| actions.get(name).copied().unwrap_or(0);
    let mut most: Vec<(String, u64)> = actions.iter().map(|(n, c)| (n.clone(), *c)).collect();
    most.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    most.truncate(MOST);
    let mut screens: Vec<(String, u64)> = screens.into_iter().collect();
    screens.sort_by_key(|(_, seconds)| std::cmp::Reverse(*seconds));
    Report {
        days: kept.len(),
        old_days: old.len(),
        never: grouped(known.iter().copied().filter(|name| count(name) == 0)),
        rarely: known.iter().filter(|name| (1..=RARELY).contains(&count(name))).map(|name| ((*name).to_owned(), count(name))).collect(),
        most,
        screens,
        hints: HINTS.iter().filter_map(|(name, text)| hints.get(*name).map(|n| ((*name).to_owned(), *n, (*text).to_owned()))).collect(),
    }
}
/// Names by report group, groups in review order, OTHER last for anything no group lists.
fn grouped<'a>(names: impl Iterator<Item = &'a str>) -> Vec<(String, Vec<String>)> {
    let names: Vec<&str> = names.collect();
    let members_of =
        |section: Section| act::listed_in(section).into_iter().filter(|m| names.contains(m)).map(str::to_owned).collect::<Vec<_>>();
    let mut out: Vec<(String, Vec<String>)> = Section::ALL
        .iter()
        .map(|report| (report.title().to_owned(), members_of(*report)))
        .filter(|(_, members)| !members.is_empty())
        .collect();
    let is_in_a_group = |name: &str| Section::ALL.iter().any(|report| act::listed_in(*report).contains(&name));
    let other: Vec<String> = names.iter().filter(|name| !is_in_a_group(name)).map(|n| (*n).to_owned()).collect();
    if !other.is_empty() {
        out.push(("other".to_owned(), other));
    }
    out
}

/// `1h 12m`, `4m`, `35s`.
pub fn duration(seconds: u64) -> String {
    match (seconds / 3600, (seconds % 3600) / 60) {
        (0, 0) => format!("{seconds}s"),
        (0, m) => format!("{m}m"),
        (h, m) => format!("{h}h {m}m"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    #[test]
    fn a_tally_hands_whole_seconds_and_keeps_the_rest() {
        let mut tally = Tally::default();
        assert_eq!(tally.take(), None, "nothing to write");
        tally.act("next_thread");
        tally.spend("diff", Duration::from_millis(2500));
        tally.hint("long_walk");
        let counts = tally.take().unwrap();
        assert_eq!(counts.actions["next_thread"], 1);
        assert_eq!(counts.screens["diff"], 2);
        assert_eq!(counts.hints["long_walk"], 1);
        tally.spend("diff", Duration::from_millis(600));
        assert_eq!(tally.take().unwrap().screens["diff"], 1, "the half second waited for the next flush");
    }

    #[test]
    fn merging_keeps_one_line_per_day_and_adds() {
        let counts = Counts { actions: BTreeMap::from([("zen".to_owned(), 2)]), ..Counts::default() };
        let once = merge("", date("2026-09-24"), &counts).unwrap();
        let twice = merge(&once, date("2026-09-24"), &counts).unwrap();
        assert_eq!(twice.lines().count(), 1);
        assert_eq!(parse(&twice)[0].actions["zen"], 4);
        let other_day = merge(&twice, date("2026-09-23"), &counts).unwrap();
        let days = parse(&other_day);
        assert_eq!(days.iter().map(|d| d.date).collect::<Vec<_>>(), [date("2026-09-23"), date("2026-09-24")], "oldest first");
        assert_eq!(merge(&other_day, date("2026-09-24"), &Counts::default()).unwrap(), other_day, "an empty merge changes nothing");
    }

    #[test]
    fn a_broken_line_is_skipped() {
        let text = "{not json}\n{\"date\":\"2026-09-24\",\"actions\":{\"zen\":1}}\n";
        assert_eq!(parse(text).len(), 1);
    }

    #[test]
    fn the_file_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::in_dir(dir.path().join("revu"));
        let counts = Counts { actions: BTreeMap::from([("zen".to_owned(), 1)]), ..Counts::default() };
        record(&cache, date("2026-09-24"), &counts).unwrap();
        record(&cache, date("2026-09-24"), &counts).unwrap();
        assert_eq!(read(&cache)[0].actions["zen"], 2);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("revu").join(FILE)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn since_reads_days_weeks_and_all() {
        assert_eq!(since("30d").unwrap(), Some(30));
        assert_eq!(since("2w").unwrap(), Some(14));
        assert_eq!(since("all").unwrap(), None);
        assert!(since("30").is_err());
        assert!(since("d").is_err());
    }

    fn fixture() -> Vec<Day> {
        parse(
            "{\"date\":\"2026-08-01\",\"actions\":{\"merge\":9}}\n\
             {\"date\":\"2026-09-20\",\"version\":2,\"actions\":{\"next_thread\":40,\"zen\":2,\"move_down\":120},\"screens\":{\"diff\":3600,\"queue\":120},\"hints\":{\"long_walk\":3}}\n\
             {\"date\":\"2026-09-24\",\"actions\":{\"next_thread\":2,\"react\":1}}\n",
        )
    }

    #[test]
    fn a_report_sorts_every_known_action() {
        let report = report(&fixture(), date("2026-09-24"), Some(30));
        assert_eq!(report.days, 2, "August is outside the window");
        assert_eq!(report.most[0], ("move_down".to_owned(), 120));
        assert_eq!(report.most[1], ("next_thread".to_owned(), 42));
        assert!(report.rarely.contains(&("zen".to_owned(), 2)) && report.rarely.contains(&("react".to_owned(), 1)));
        let never: Vec<&String> = report.never.iter().flat_map(|(_, names)| names).collect();
        assert!(never.iter().any(|n| *n == "merge"), "merge only happened in August");
        assert!(!never.iter().any(|n| *n == "next_thread"));
        assert_eq!(
            never.len() + report.rarely.len() + 2,
            known().len(),
            "every known action lands somewhere: never, rarely, or next_thread and move_down"
        );
        assert_eq!(report.screens[0], ("diff".to_owned(), 3600));
        assert_eq!(report.hints[0].0, "long_walk");
        assert_eq!(super::report(&fixture(), date("2026-09-24"), None).days, 3);
    }

    #[test]
    fn a_day_counted_the_old_way_keeps_its_actions_and_leaves_out_its_time_and_hints() {
        let old = "{\"date\":\"2026-09-23\",\"actions\":{\"zen\":5},\"screens\":{\"diff\":36000},\"hints\":{\"queue_after_zen\":40}}\n";
        let counts = Counts { screens: BTreeMap::from([("diff".to_owned(), 60)]), ..Counts::default() };
        let text = merge(&merge(old, date("2026-09-24"), &counts).unwrap(), date("2026-09-23"), &counts).unwrap();
        let report = report(&parse(&text), date("2026-09-24"), None);
        assert_eq!((report.days, report.old_days), (2, 1));
        assert!(report.rarely.is_empty() && report.most[0] == ("zen".to_owned(), 5), "{:?}", report.most);
        assert_eq!(report.screens, [("diff".to_owned(), 60)], "only the new day's minute, even with new time merged into the old day");
        assert_eq!(report.hints, vec![], "the old day's hints are left out");
    }

    #[test]
    fn a_hint_shows_its_first_times_over_every_session_then_only_counts() {
        let mut first = Tally::default();
        assert_eq!([first.hint("long_walk"), first.hint("long_walk")], [true, true]);
        let text = merge("", date("2026-09-23"), &first.take().unwrap()).unwrap();
        let mut next = Tally::after(&parse(&text));
        assert_eq!([next.hint("long_walk"), next.hint("long_walk"), next.hint("palette_number")], [true, false, true]);
        let counts = next.take().unwrap();
        assert_eq!((counts.hints["long_walk"], counts.shown["long_walk"]), (2, 1));
    }

    #[test]
    fn a_renamed_action_counts_under_its_new_name() {
        let day = |name: &str| Day { actions: BTreeMap::from([(name.to_owned(), 1)]), ..Day::new(date("2026-09-24")) };
        let report = report(&[day("split"), day("side_by_side")], date("2026-09-24"), None);
        assert!(report.rarely.contains(&("side_by_side".to_owned(), 2)), "{:?}", report.rarely);
        assert!(!report.most.iter().any(|(name, _)| name == "split"));
    }

    #[test]
    fn every_countable_action_is_known_once_in_the_order_of_the_table() {
        let known = known();
        for name in ["pin", ":ready", ":pin", ":unpin", ":outline", ":session", ":news"] {
            assert!(known.contains(&name), "`{name}` is counted, so it shows when unused");
        }
        assert_eq!(known.iter().filter(|name| **name == "viewed").count(), 1, "`m` and `zv` share a name");
        let commands: Vec<String> = act::verbs().map(|verb| format!(":{}", verb.word)).collect();
        let report = grouped(known.iter().copied());
        assert_eq!(
            report.iter().find(|(group, _)| group == "commands").map(|(_, names)| names),
            Some(&commands),
            "as the palette lists them"
        );
    }

    #[test]
    fn an_action_no_group_lists_still_shows() {
        let groups = grouped(["next_thread", "brand_new"].into_iter());
        assert_eq!(groups.last().unwrap(), &("other".to_owned(), vec!["brand_new".to_owned()]));
        for name in known() {
            let listed = Section::ALL.iter().any(|report| act::listed_in(*report).contains(&name));
            assert!(listed, "`{name}` belongs in a report group");
        }
    }

    #[test]
    fn durations_read_short() {
        assert_eq!(duration(35), "35s");
        assert_eq!(duration(240), "4m");
        assert_eq!(duration(4320), "1h 12m");
    }
}
