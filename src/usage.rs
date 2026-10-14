//! What revu is used for, counted on this machine only: which actions, how long on each screen,
//! and a few habits a faster key would replace. Opt-in with `[usage] enabled = true`.
//! Nothing about the MRs is kept: no titles, paths, text, hosts or projects, only action names.
use crate::act::{self, Act, Place, Row};
use crate::cache::Cache;
use anyhow::{Result, bail};
use chrono::NaiveDate;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

/// The file, at the cache root; deleting it starts the counts again.
pub const FILE: &str = "usage.jsonl";

/// How the report groups actions, in the order a review goes; anything not listed lands in OTHER,
/// so a new action shows up without anyone updating this table.
const GROUPS: &[(&str, &[&str])] = &[
    (
        "move",
        &[
            "move_down",
            "move_up",
            "page_down",
            "page_up",
            "full_page_down",
            "full_page_up",
            "top",
            "bottom",
            "focus_left",
            "focus_right",
            "open",
            "back",
        ],
    ),
    (
        "jump",
        &[
            "next_thread",
            "prev_thread",
            "next_any_thread",
            "prev_any_thread",
            "next_hunk",
            "prev_hunk",
            "next_file",
            "prev_file",
            "next_file_unresolved",
            "prev_file_unresolved",
            "next_review",
            "prev_review",
            "next_mr",
            "prev_mr",
        ],
    ),
    ("queue", &["refresh", "scope", "sort_queue", "group_by_author", "filter", "views"]),
    (
        "view",
        &[
            "fold_toggle",
            "fold_open",
            "fold_close",
            "fold_all",
            "unfold_all",
            "fold_header",
            "viewed",
            "zen",
            "side_by_side",
            "peek_after",
            "peek_before",
            "tree",
            "pipeline",
            "outline",
            "prose",
            "wrap",
            "whitespace",
            "more_context",
            "view_file",
            "description",
        ],
    ),
    (
        "comment & publish",
        &["comment", "comment_old", "select", "suggest", "editor", "resolve", "publish", "approve", "merge", "ready", "share"],
    ),
    (
        "thread pane",
        &[
            "every_thread",
            "reply",
            "react",
            "apply_suggestion",
            "edit_draft",
            "delete_draft",
            "close_pane",
            "next_in_line",
            "prev_in_line",
            "open_link",
            "copy_first_link",
        ],
    ),
    ("ask claude", &["ask_explain", "ask_risks", "ask_summary", "ask_thread", "ask_comment", "ask_free"]),
    ("search & app", &["palette", "jump", "palette_mr", "palette_file", "open_browser", "copy_link", "session", "help", "quit"]),
    (
        "commands",
        &[
            ":go", ":open", ":copy", ":approve", ":merge", ":ready", ":publish", ":threads", ":all", ":outline", ":set", ":view", ":ai",
            ":ask", ":share", ":session", ":help", ":quit",
        ],
    ),
];

/// A habit a faster key replaces, with what to try instead.
pub const HINTS: &[(&str, &str)] = &[
    ("long_walk", "walked 15 lines or more with j or k to a hunk or a thread; ]c (next hunk) or ]n (next thread) jumps there"),
    ("queue_after_zen", "left zen to open another MR from the queue; [m and ]m change MR, in zen too"),
    ("palette_number", "typed a bare number in the search; !42 (or #42 on GitHub) finds that MR directly"),
];

/// Every action the report knows, bindable ones first.
pub fn known() -> Vec<&'static str> {
    crate::act::listed_names().collect()
}
/// The key that opens the saved views; any letter after it picks one.
const VIEWS_PREFIX: char = '\'';

/// Moves held with ctrl are not counted.
const NOT_WITH_CTRL: &[Act] = &[Act::MoveDown, Act::MoveUp, Act::Top, Act::Bottom, Act::Open];

/// The action a key stands for, after the user's bindings turned it into revu's own.
/// `prefix` is the first key of a two-key sequence already typed (`z`, `[`, `]`, `a`, `'`).
pub fn action(prefix: Option<char>, key: KeyEvent, place: Place) -> Option<&'static str> {
    let act = match prefix {
        Some(prefix) => prefixed(prefix, key),
        None => single(key, place),
    };
    act?.row().name.counted()
}

fn prefixed(prefix: char, key: KeyEvent) -> Option<Act> {
    let KeyCode::Char(c) = key.code else { return None };
    if prefix == VIEWS_PREFIX {
        return Some(Act::Views);
    }
    act::act_after(prefix, c)
}

/// A key alone: ctrl keys first, then the key whatever its modifiers, among the actions counted in `place`.
fn single(key: KeyEvent, place: Place) -> Option<Act> {
    if is_palette_chord(key) {
        return Some(Act::Jump);
    }
    let counted_here = |row: &Row| row.counts.contains(&place);
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if ctrl && let Some(act) = act::with_key(key.code, KeyModifiers::CONTROL, counted_here) {
        return Some(act);
    }
    if let KeyCode::Char(c) = key.code
        && (c == VIEWS_PREFIX || act::is_prefix(c))
    {
        return None;
    }
    act::with_key(key.code, KeyModifiers::NONE, counted_here).filter(|act| !(ctrl && NOT_WITH_CTRL.contains(act)))
}

/// `ctrl-k` or `⌘k`, either case.
fn is_palette_chord(key: KeyEvent) -> bool {
    let is_k = matches!(key.code, KeyCode::Char('k' | 'K'));
    is_k && key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER)
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
    let mut out: Vec<(String, Vec<String>)> = GROUPS
        .iter()
        .map(|(group, members)| {
            ((*group).to_owned(), members.iter().filter(|m| names.contains(m)).map(|m| (*m).to_owned()).collect::<Vec<_>>())
        })
        .filter(|(_, members)| !members.is_empty())
        .collect();
    let other: Vec<String> =
        names.iter().filter(|name| !GROUPS.iter().any(|(_, members)| members.contains(name))).map(|n| (*n).to_owned()).collect();
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
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn date(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    #[test]
    fn keys_name_their_action_by_where_they_land() {
        assert_eq!(action(None, key('s'), Place::Queue), Some("sort_queue"));
        assert_eq!(action(None, key('s'), Place::Diff), Some("suggest"));
        assert_eq!(action(None, key('S'), Place::Pane), Some("apply_suggestion"));
        assert_eq!(action(None, key('+'), Place::Pane), Some("react"));
        assert_eq!(action(None, key('+'), Place::Diff), Some("react"));
        assert_eq!(action(None, key('='), Place::Diff), Some("more_context"));
        assert_eq!(action(Some(']'), key('m'), Place::Diff), Some("next_mr"));
        assert_eq!(action(None, KeyEvent::new(KeyCode::Right, KeyModifiers::NONE), Place::Diff), Some("focus_right"));
        assert_eq!(action(Some(']'), key('n'), Place::Diff), Some("next_thread"));
        assert_eq!(action(Some('['), key('N'), Place::Diff), Some("prev_any_thread"));
        assert_eq!(action(Some('a'), key('r'), Place::Diff), Some("ask_risks"));
        assert_eq!(action(None, key('z'), Place::Diff), None, "a prefix alone is no action");
        assert_eq!(action(None, KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL), Place::Queue), Some("jump"));
    }

    #[test]
    fn every_bindable_action_can_be_counted() {
        let places = [Place::Queue, Place::Diff, Place::Pane];
        for (name, keys) in crate::act::bindable() {
            let events = crate::keymap::parse(keys).unwrap();
            let (prefix, last) = match events.as_slice() {
                [only] if only.plain_char() == Some('\'') => (Some('\''), key('m')),
                [only] => (None, only.event()),
                [first, second] => (first.plain_char(), second.event()),
                _ => panic!("{name}"),
            };
            let found = places.iter().any(|place| action(prefix, last, *place) == Some(name));
            assert!(found, "`{name}` ({keys}) is never counted");
        }
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
    fn an_action_no_group_lists_still_shows() {
        let groups = grouped(["next_thread", "brand_new"].into_iter());
        assert_eq!(groups.last().unwrap(), &("other".to_owned(), vec!["brand_new".to_owned()]));
        for name in known() {
            let listed = GROUPS.iter().any(|(_, members)| members.contains(&name));
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
