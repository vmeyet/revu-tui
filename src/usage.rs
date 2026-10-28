//! What revu is used for, counted on this machine only: which actions, how long on each screen,
//! and a few habits a faster key would replace. Opt-in with `[usage] enabled = true`.
//! Nothing about the MRs is kept: no titles, paths, text, hosts or projects, only action names.
//! vtui counts, keeps the file's format and sums the report; revu names what it counts and keeps the file in its cache.
use crate::act::{self, Section};
use crate::cache::Cache;
use anyhow::Result;
use chrono::NaiveDate;
pub(crate) use vtui::usage::{Counts, Tally};
use vtui::usage::{Day, Group, Hint, Rules};

/// The file, at the cache root; deleting it starts the counts again.
pub(crate) const FILE: &str = "usage.jsonl";

/// A habit a faster key replaces, with what to try instead.
const HINTS: &[Hint] = &[
    Hint {
        name: "long_walk",
        text: "walked 15 lines or more with j or k to a hunk or a thread; ]c (next hunk) or ]n (next thread) jumps there",
    },
    Hint { name: "queue_after_zen", text: "left zen to open another MR from the queue; [m and ]m change MR, in zen too" },
    Hint { name: "palette_number", text: "typed a bare number in the search; !42 (or #42 on GitHub) finds that MR directly" },
];

/// How time and hints are counted now: time stops a minute after the last input and while the terminal is in the
/// background, and a hint fires only where a jump would have helped. Days counted before it keep only their actions.
const COUNTING: u32 = 2;

/// What revu counts: every action the table lists, grouped as the report shows them, and its hints.
pub(crate) fn rules() -> Rules {
    Rules {
        actions: act::listed_names(),
        groups: Section::ALL.iter().map(|section| Group { title: section.title(), actions: act::listed_in(*section) }).collect(),
        hints: HINTS,
        renamed: crate::keymap::RENAMED,
        counting: COUNTING,
    }
}

/// Adds `counts` to today's line in the cache's usage file, privately and atomically.
pub(crate) fn record(cache: &Cache, date: NaiveDate, counts: &Counts) -> Result<()> {
    let text = cache.read_bytes(FILE).map(|bytes| String::from_utf8_lossy(&bytes).into_owned()).unwrap_or_default();
    cache.write_bytes(FILE, rules().merge(&text, date, counts)?.as_bytes())
}

/// Every recorded day in the cache's usage file.
pub(crate) fn read(cache: &Cache) -> Vec<Day> {
    cache.read_bytes(FILE).map(|bytes| rules().parse(&String::from_utf8_lossy(&bytes))).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use vtui::usage::{Count, Window};

    fn date(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    #[test]
    fn the_file_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::in_dir(dir.path().join("revu"));
        let counts = Counts { actions: BTreeMap::from([("zen".to_owned(), 1)]), ..Counts::default() };
        record(&cache, date("2026-09-24"), &counts).unwrap();
        record(&cache, date("2026-09-24"), &counts).unwrap();
        assert_eq!(read(&cache)[0].counts.actions["zen"], 2);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("revu").join(FILE)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn a_file_written_before_vtui_counted_still_reads() {
        let text = "{\"date\":\"2026-09-23\",\"actions\":{\"zen\":1},\"screens\":{\"diff\":60}}\n\
                    {\"date\":\"2026-09-24\",\"version\":2,\"actions\":{\"next_thread\":3},\"screens\":{},\"hints\":{\"long_walk\":1},\"shown\":{\"long_walk\":1}}\n";
        let days = rules().parse(text);
        assert_eq!((days[0].version, days[1].version, days[1].counts.hints["long_walk"]), (1, 2, 1));
        assert_eq!(rules().merge(text, date("2026-09-25"), &Counts::default()).unwrap().lines().nth(1), text.lines().nth(1));
    }

    #[test]
    fn a_report_sorts_every_known_action() {
        let days = rules().parse(
            "{\"date\":\"2026-09-20\",\"version\":2,\"actions\":{\"next_thread\":40,\"zen\":2,\"move_down\":120}}\n\
             {\"date\":\"2026-09-24\",\"actions\":{\"next_thread\":2,\"react\":1}}\n",
        );
        let report = rules().report(&days, date("2026-09-24"), Window::Days(30));
        let never = report.never.iter().map(|unused| unused.actions.len()).sum::<usize>();
        assert_eq!(
            never + report.rarely.len() + 2,
            rules().actions.len(),
            "every known action lands somewhere: never, rarely, or next_thread and move_down"
        );
    }

    #[test]
    fn a_renamed_action_counts_under_its_new_name() {
        let days = rules().parse("{\"date\":\"2026-09-24\",\"actions\":{\"split\":1,\"side_by_side\":1}}");
        let report = rules().report(&days, date("2026-09-24"), Window::All);
        assert!(report.rarely.contains(&Count { action: "side_by_side".to_owned(), count: 2 }), "{:?}", report.rarely);
        assert!(!report.most.iter().any(|count| count.action == "split"));
    }

    #[test]
    fn every_countable_action_is_known_once_in_the_order_of_the_table() {
        let known = rules().actions;
        for name in ["pin", ":ready", ":pin", ":unpin", ":outline", ":session", ":news"] {
            assert!(known.contains(&name), "`{name}` is counted, so it shows when unused");
        }
        assert_eq!(known.iter().filter(|name| **name == "viewed").count(), 1, "`m` and `zv` share a name");
        let commands: Vec<String> = act::verbs().map(|verb| format!(":{}", verb.word)).collect();
        let listed = rules().groups.into_iter().find(|group| group.title == "commands").map(|group| group.actions);
        assert_eq!(listed, Some(commands.iter().map(String::as_str).collect()), "as the palette lists them");
    }

    #[test]
    fn every_known_action_belongs_in_a_report_group() {
        let rules = rules();
        for name in &rules.actions {
            assert!(rules.groups.iter().any(|group| group.actions.contains(name)), "`{name}` belongs in a report group");
        }
    }
}
