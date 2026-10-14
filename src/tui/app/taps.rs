//! The tap bar: in the compact layout, one bottom row of labels a thumb can press. A tap presses the keys of its
//! label, so it does what they do, and the first `?` lists the same few keys instead of a pane's whole groups.
use super::{Action, App, Focus, Open};
use ratatui::layout::{Position, Rect};

/// One label of the tap bar: what it reads, the keys a tap presses, written as `[keys.bind]` writes them, and
/// what they do, for the short key list.
#[derive(Debug, PartialEq, Eq)]
pub struct Tap {
    pub label: &'static str,
    pub keys: &'static str,
    pub does: &'static str,
}

const fn tap(label: &'static str, keys: &'static str, does: &'static str) -> Tap {
    Tap { label, keys, does }
}

/// The last label of every page: what has no label of its own is a command away.
const MORE: Tap = tap("more", ":", "every command: approve, publish");

const QUEUE: [Tap; 5] = [
    tap("open", "enter", "open the MR"),
    tap("filter", "/", "filter the queue"),
    tap("refresh", "r", "refresh"),
    tap("copy", "y", "copy the MR's link"),
    MORE,
];
const DIFF: [Tap; 6] = [
    tap("file", "tab", "next file"),
    tap("thread", "]n", "next open thread"),
    tap("viewed", "m", "file viewed, on to the next one"),
    tap("comment", "c", "comment on the line"),
    tap("back", "h", "back to the queue"),
    MORE,
];
const THREAD: [Tap; 6] = [
    tap("reply", "r", "reply, as a draft"),
    tap("resolve", "R", "resolve, unresolve"),
    tap("react", "+", "react"),
    tap("copy", "y", "copy the thread's link"),
    tap("back", "h", "back a page"),
    MORE,
];
const PANE: [Tap; 2] = [tap("back", "h", "back to the diff"), MORE];

/// What the compact layout shows full screen, by what the keys act on: a thread is one under the diff's cursor
/// or the thread pane, a pane any other right pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Queue,
    Diff,
    Thread,
    Pane,
}

impl Page {
    pub fn name(self) -> &'static str {
        match self {
            Self::Queue => "queue",
            Self::Diff => "diff",
            Self::Thread => "thread",
            Self::Pane => "pane",
        }
    }

    pub fn taps(self) -> &'static [Tap] {
        match self {
            Self::Queue => &QUEUE,
            Self::Diff => &DIFF,
            Self::Thread => &THREAD,
            Self::Pane => &PANE,
        }
    }
}

/// A label as the last frame drew it: its cell, and the keys a tap in it presses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TapSpot {
    pub area: Rect,
    pub keys: &'static str,
}

impl App {
    pub fn page(&self) -> Page {
        let threads_pane = self.open.as_ref().is_some_and(Open::shows_threads);
        match self.focus {
            Focus::Queue => Page::Queue,
            Focus::Review if self.on_thread_row() => Page::Thread,
            Focus::Review => Page::Diff,
            Focus::Side if threads_pane => Page::Thread,
            Focus::Side => Page::Pane,
        }
    }

    /// Something asks the reader for an answer on the bottom rows: a confirmation, the quit prompt, the reaction
    /// picker, the `'` views or the diff's search.
    pub fn asks(&self) -> bool {
        self.confirm.is_some()
            || self.pending == Some('\'')
            || self.quit_prompt().is_some()
            || self.react.is_some()
            || self.search.is_some()
    }

    /// The tap bar has its row: in the compact layout, unless `[tui] tap_bar` hides it, and never under a
    /// question, text being typed or an overlay, which take the keys its labels would press.
    pub fn shows_taps(&self) -> bool {
        let typing = self.filtering || self.input.is_some();
        let taken = self.asks() || self.offer.is_some() || typing || self.modal();
        self.compact && self.tap_bar && !taken
    }

    /// A click on a label presses its keys, one after the other; a label drawn before the bar lost its row answers
    /// nothing.
    pub(super) fn tap_at(&mut self, at: Position) -> Option<Vec<Action>> {
        if !self.shows_taps() {
            return None;
        }
        let keys = self.taps.iter().find(|spot| spot.area.contains(at))?.keys;
        let keys = crate::keymap::parse(keys).unwrap_or_default();
        Some(keys.into_iter().flat_map(|key| self.handle_key(key.event())).collect())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::{DIFF, PANE, Page, QUEUE, THREAD, Tap};
    use crate::tui::app::test_support::*;

    /// The narrowest phone the bar is fitted for.
    const SMALL_PHONE: u16 = 45;

    fn bar(app: &mut App) -> String {
        phone(app).lines().last().unwrap().split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// Draws the phone, then clicks the middle of `label` in the bar.
    fn tap(app: &mut App, label: &str) -> Vec<Action> {
        let screen = phone(app);
        let row = screen.lines().last().unwrap();
        let start = row.find(label).unwrap_or_else(|| panic!("{label} in the bar: {row:?}"));
        click(app, ((row[..start].chars().count() + label.len() / 2) as u16, PHONE.1 - 1))
    }

    /// The app after `keys` were typed and what they asked for, to hold a tap against.
    fn after(mut app: App, keys: &str) -> (App, Vec<Action>) {
        phone(&mut app);
        let actions = crate::keymap::parse(keys).unwrap().into_iter().flat_map(|key| app.handle_key(key.event())).collect();
        (app, actions)
    }

    fn on_a_thread_row() -> App {
        let mut app = with_review();
        phone(&mut app);
        press(&mut app, "]Nj");
        assert!(app.on_thread_row());
        app
    }

    #[test]
    fn every_label_presses_keys_that_parse_and_every_bar_fits_a_small_phone() {
        for taps in [&QUEUE[..], &DIFF, &THREAD, &PANE] {
            let width: usize = taps.iter().map(|tap| tap.label.len() + 2).sum();
            assert!(width <= usize::from(SMALL_PHONE), "{width} columns: {taps:?}");
            for Tap { label, keys, .. } in taps {
                assert!(crate::keymap::parse(keys).is_ok(), "{label}: {keys}");
            }
            assert_eq!(taps.last().map(|tap| tap.keys), Some(":"), "the last label opens the commands");
        }
    }

    #[test]
    fn each_page_has_its_labels() {
        let mut queue = with_queue();
        assert_eq!(bar(&mut queue), "open filter refresh copy more");
        let mut diff = with_review();
        assert_eq!((diff.page(), bar(&mut diff)), (Page::Diff, "file thread viewed comment back more".to_owned()));
        let mut row = on_a_thread_row();
        assert_eq!((row.page(), bar(&mut row)), (Page::Thread, "reply resolve react copy back more".to_owned()));
        let mut pane = with_review();
        press(&mut pane, "T");
        assert_eq!((pane.page(), bar(&mut pane)), (Page::Thread, "reply resolve react copy back more".to_owned()));
        let mut tree = with_review();
        press(&mut tree, "t");
        tree.focus = Focus::Side;
        assert_eq!((tree.page(), bar(&mut tree)), (Page::Pane, "back more".to_owned()));
    }

    #[test]
    fn a_tap_on_a_queue_label_does_what_its_key_does() {
        for (label, keys) in [("open", "enter"), ("filter", "/"), ("refresh", "r"), ("copy", "y"), ("more", ":")] {
            let mut tapped = with_queue();
            let (typed, asked) = after(with_queue(), keys);
            assert_eq!(tap(&mut tapped, label), asked, "{label}");
            let state =
                |app: &App| (app.focus, app.filtering, app.queue_loading, app.palette.as_ref().map(|p| p.mode), app.opening.clone());
            assert_eq!(state(&tapped), state(&typed), "{label}");
        }
        assert_eq!(tap(&mut with_queue(), "open"), vec![Action::Open(mr_key())]);
        assert_eq!(tap(&mut with_queue(), "refresh"), vec![Action::LoadQueue { scope: None, from_cache: false }]);
    }

    #[test]
    fn a_tap_on_a_diff_label_does_what_its_key_does() {
        let on_a_line = || {
            let mut app = with_review();
            on_line(&mut app);
            app
        };
        for (label, keys) in [("file", "tab"), ("thread", "]n"), ("viewed", "m"), ("comment", "c"), ("back", "h"), ("more", ":")] {
            let mut tapped = on_a_line();
            let (typed, asked) = after(on_a_line(), keys);
            assert_eq!(tap(&mut tapped, label), asked, "{label}");
            let state = |app: &App| {
                let open = app.open.as_ref().unwrap();
                (app.focus, open.selected, open.review.viewed.clone(), app.input.clone(), app.palette.as_ref().map(|p| p.mode))
            };
            assert_eq!(state(&tapped), state(&typed), "{label}");
        }
        let mut app = on_a_line();
        tap(&mut app, "comment");
        assert!(matches!(app.input, Some(Input::Comment { .. })), "the box opens on the line");
        let mut app = on_a_line();
        tap(&mut app, "back");
        assert_eq!(app.focus, Focus::Queue);
    }

    #[test]
    fn the_react_label_and_plus_on_a_thread_row_open_the_same_row() {
        let mut app = on_a_thread_row();
        app.reactions = crate::reactions::History::default().with("gitlab.com", Emoji::Eyes);
        tap(&mut app, "react");
        let tapped = app.react.take().unwrap().row;
        press(&mut app, "+");
        assert_eq!(app.react.take().unwrap().row, tapped);
        assert_eq!(tapped[0], Emoji::Eyes);
    }

    #[test]
    fn a_tap_on_a_thread_label_acts_on_the_thread_under_the_cursor() {
        let mut app = on_a_thread_row();
        let resolved = tap(&mut app, "resolve");
        assert!(matches!(resolved.as_slice(), [Action::Resolve { resolved: false, .. }]), "{resolved:?}");
        let copied = tap(&mut app, "copy");
        assert!(matches!(copied.as_slice(), [Action::Copy { text, .. }] if text.contains("#note_")), "the thread's link: {copied:?}");
        tap(&mut app, "react");
        assert!(app.react.is_some(), "the picker opens on its note");
        app.handle_key(code(KeyCode::Esc));
        tap(&mut app, "reply");
        assert!(matches!(app.input, Some(Input::Reply { .. })));
        app.handle_key(code(KeyCode::Esc));
        tap(&mut app, "more");
        assert_eq!(app.palette.as_ref().map(|p| p.mode), Some(crate::tui::palette::Mode::Commands));
        let mut pane = with_review();
        press(&mut pane, "T");
        tap(&mut pane, "back");
        assert_eq!(pane.focus, Focus::Review, "from the pane, back is the diff");
    }

    #[test]
    fn the_bar_gives_its_row_to_what_asks_or_reads_text() {
        let last = |app: &mut App| phone(app).lines().last().unwrap().to_owned();
        let mut filter = with_queue();
        press(&mut filter, "/pay");
        assert!(last(&mut filter).contains("gitlab.com") && !filter.shows_taps(), "the filter keeps the rows it had");
        assert!(filter.taps.is_empty(), "and no label answers under it");
        let mut search = with_review();
        press(&mut search, "/client");
        assert!(last(&mut search).contains("/client"));
        let mut quit = with_review();
        press(&mut quit, "q");
        assert!(!quit.shows_taps() && !last(&mut quit).contains("more"), "the quit prompt");
        let mut compose = with_review();
        on_line(&mut compose);
        press(&mut compose, "c");
        assert!(!compose.shows_taps(), "typed text owns the keys");
        let mut palette = with_review();
        press(&mut palette, ":");
        assert!(!palette.shows_taps());
        let mut back = with_review();
        press(&mut back, "q");
        back.handle_key(code(KeyCode::Esc));
        assert!(last(&mut back).contains("more"), "and it comes back");
    }

    #[test]
    fn a_label_drawn_before_the_filter_opened_answers_nothing() {
        let mut app = with_queue();
        phone(&mut app);
        let open = app.taps[0].area;
        press(&mut app, "/");
        assert_eq!(click(&mut app, (open.x + 1, open.y)), vec![]);
        assert!(app.filtering && app.opening.is_none());
    }

    #[test]
    fn the_bar_shows_in_the_compact_layout_only_and_the_setting_hides_it() {
        let mut app = with_review();
        let wide = render(&mut app, 70, 30);
        assert!(wide.lines().last().unwrap().contains("gitlab.com") && app.taps.is_empty(), "never from 70 columns:\n{wide}");
        let mut hidden = with_review();
        hidden.tap_bar = false;
        let screen = phone(&mut hidden);
        assert!(screen.lines().last().unwrap().contains("gitlab.com") && hidden.taps.is_empty(), "{screen}");
    }

    #[test]
    fn the_bar_costs_one_row_and_stays_on_a_short_screen() {
        let mut app = with_review();
        let screen = render(&mut app, 50, 14);
        assert_eq!(app.areas.review.height, 13, "{screen}");
        assert!(screen.lines().last().unwrap().contains("comment"));
        phone(&mut app);
        assert_eq!(app.areas.review.height, 28, "the status row and the bar under the page");
    }

    #[test]
    fn a_bar_too_wide_drops_labels_before_the_last() {
        let mut app = with_review();
        let row = render(&mut app, 30, 30).lines().last().unwrap().split_whitespace().collect::<Vec<_>>().join(" ");
        assert_eq!(row, "file thread viewed more");
    }

    #[test]
    fn the_first_question_mark_lists_the_keys_of_the_bar_and_the_second_every_key() {
        let mut app = with_review();
        phone(&mut app);
        press(&mut app, "?");
        let short = phone(&mut app);
        assert!(short.contains("DIFF") && short.contains("tab  next file") && short.contains("]n  next open thread"), "{short}");
        assert!(!short.contains("MOVE") && !short.contains("half a page"), "{short}");
        assert_eq!(crate::tui::help::last_row(&app, false), DIFF.len(), "a header and one row per label");
        press(&mut app, "?");
        assert!(render(&mut app, 50, 60).contains("MOVE"), "every key, as on a wide screen");
        press(&mut app, "?");
        let mut wide = with_review();
        render(&mut wide, 120, 40);
        press(&mut wide, "?");
        assert!(render(&mut wide, 120, 40).contains("MOVE"), "from 70 columns the first list is the pane's");
    }

    #[test]
    fn every_key_still_works_on_a_phone() {
        let mut app = with_review();
        phone(&mut app);
        press(&mut app, "]c");
        assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::Hunk { .. })));
        press(&mut app, "zz");
        assert!(app.zen);
        press(&mut app, "zzT");
        assert_eq!(app.focus, Focus::Side);
    }

    #[test]
    fn a_comma_opens_the_palette_on_mrs_as_ctrl_k_does() {
        let mut typed = with_review();
        press(&mut typed, ",");
        let mut chord = with_review();
        chord.handle_key(ctrl_k());
        assert_eq!(typed.palette, chord.palette);
        assert_eq!(typed.palette.as_ref().map(|p| p.mode), Some(crate::tui::palette::Mode::Mrs));
        let mut queue = with_queue();
        press(&mut queue, ",");
        assert!(queue.palette.is_some(), "from the queue too");
        let mut filter = with_queue();
        press(&mut filter, "/a,b");
        assert_eq!((filter.filter.as_str(), filter.palette.is_none()), ("a,b", true), "typed text keeps its commas");
    }
}
