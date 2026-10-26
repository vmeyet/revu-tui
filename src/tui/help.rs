//! The `?` overlay: the keys of the focused pane, or every key after a second `?`, grouped by task in the order a
//! review goes. In the compact layout the first `?` lists the keys of the tap bar alone. vtui draws it.
use super::app::{App, Focus};
use crate::act::Group;
use crate::keymap::Keymap;
use ratatui::Frame;
use ratatui::layout::Rect;
use vtui::help::{Help, Row, Section};

/// The groups, in the order a review goes.
pub(crate) const GROUPS: [Group; 8] =
    [Group::Move, Group::Queue, Group::View, Group::Comment, Group::Thread, Group::Outline, Group::Ask, Group::App];

/// The panes whose first `?` shows the group.
fn panes(group: Group) -> &'static [Focus] {
    match group {
        Group::Move => &[Focus::Queue, Focus::Review],
        Group::Queue => &[Focus::Queue],
        Group::View => &[Focus::Review],
        Group::Comment | Group::Ask => &[Focus::Review, Focus::Side],
        Group::Thread | Group::Outline => &[Focus::Side],
        Group::App => &[Focus::Queue, Focus::Review, Focus::Side],
    }
}

/// The groups the overlay lists: every one, or those of the focused pane.
fn groups(every_key: bool, focus: Focus) -> impl Iterator<Item = Group> {
    GROUPS.into_iter().filter(move |g| every_key || panes(*g).contains(&focus))
}

/// The last row the list can scroll to.
pub(super) fn last_row(app: &App, every_key: bool) -> usize {
    vtui::help::last_row(&shown(app, every_key))
}

/// The groups with their keys as they are in effect: the user's bindings, `(`/`)` on AZERTY. In the compact layout,
/// until every key is asked for, one group: the keys of the page's tap bar.
fn shown(app: &App, every_key: bool) -> Vec<Section> {
    let keymap: &Keymap = &app.keymap;
    if app.compact && !every_key {
        let page = app.page();
        let rows = page.taps().iter().map(|tap| Row { keys: keymap.label(tap.keys), does: tap.does }).collect();
        return vec![Section { title: page.name(), rows }];
    }
    groups(every_key, app.focus)
        .map(|g| Section { title: g.title(), rows: g.lines().map(|line| Row { keys: keymap.label(line.keys), does: line.does }).collect() })
        .collect()
}

/// The overlay, centred, at most 90 % of the screen each way; it scrolls by `scroll` rows when taller than that.
pub(crate) fn draw(f: &mut Frame<'_>, app: &App, area: Rect, help: Help) {
    vtui::help::draw(f, &app.theme, area, help, &shown(app, help.every_key));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// A help word as the key map writes a default: `S-tab` is `backtab`, `^k` is `ctrl-k`.
    fn as_default(word: &str) -> String {
        match word {
            "S-tab" => "backtab".to_owned(),
            "←" => "left".to_owned(),
            "→" => "right".to_owned(),
            _ => word.strip_prefix('^').map_or_else(|| word.to_owned(), |rest| format!("ctrl-{rest}")),
        }
    }

    #[test]
    fn every_bindable_action_has_its_key_in_a_group() {
        let words: HashSet<String> = GROUPS.iter().flat_map(|g| g.lines()).flat_map(|line| line.keys.split(' ')).map(as_default).collect();
        let missing: Vec<&str> = crate::act::bindable().filter(|(_, key)| !words.contains(*key)).map(|(name, _)| name).collect();
        assert!(missing.is_empty(), "actions with no line in the help: {missing:?}");
    }

    #[test]
    fn a_key_says_one_thing_per_group() {
        for group in &GROUPS {
            let mut seen = HashSet::new();
            for line in group.lines() {
                assert!(seen.insert(line.keys), "{} twice in {}", line.keys, group.title());
            }
        }
    }

    #[test]
    fn a_pane_lists_its_own_groups_and_every_key_lists_them_all() {
        let titles = |every_key, focus| groups(every_key, focus).map(Group::title).collect::<Vec<_>>();
        assert_eq!(titles(false, Focus::Queue), ["move", "queue", "search & app"]);
        assert_eq!(titles(false, Focus::Review), ["move", "view", "comment & publish", "ask claude", "search & app"]);
        assert_eq!(titles(false, Focus::Side), ["comment & publish", "thread pane", "outline pane", "ask claude", "search & app"]);
        assert_eq!(titles(true, Focus::Side), GROUPS.map(Group::title));
    }
}
