//! What changed in revu itself: one toast on the first start after an update, and the `:news` page.
use super::keys::scroll_key;
use super::{Action, App};
use crate::changelog::{Changelog, Position, Section};
use crossterm::event::{KeyCode, KeyEvent};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WhatsNew {
    pub changelog: Changelog,
    /// How far the reader had got before this start: what came after is new to them.
    pub seen: Position,
}

impl WhatsNew {
    /// What the first start after an update says: `revu 0.7.0 · 4 new things · :news`.
    pub fn announcement(&self) -> Option<String> {
        let count = self.changelog.count_since(self.seen);
        let things = if count == 1 { "thing" } else { "things" };
        (count > 0).then(|| format!("revu {} · {count} new {things} · :news", self.changelog.running))
    }

    /// Every section, newest first, each with whether it is new to the reader.
    pub fn sections(&self) -> Vec<(&Section, bool)> {
        let new = self.changelog.since(self.seen);
        let is_new = |section: &Section| new.iter().any(|other| other.version == section.version);
        self.changelog.since(Position::START).into_iter().map(|section| (section, is_new(section))).collect()
    }
}

impl App {
    /// `:news`: the page, on its newest version.
    pub(super) fn open_whats_new(&mut self) {
        self.news_page = Some(0);
    }

    /// The view stops the scroll at the last page, so no key needs to know how long the page is.
    pub(super) fn handle_news_page_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let Some(scroll) = self.news_page else { return vec![] };
        self.news_page = match key.code {
            KeyCode::Esc | KeyCode::Char('q') => None,
            _ => Some(scroll_key(scroll, usize::MAX, key).unwrap_or(scroll)),
        };
        vec![]
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::changelog::tests::sample;
    use crate::tui::app::test_support::*;

    fn updated_from(seen: &str) -> App {
        let whats_new = WhatsNew { changelog: sample(), seen: seen.parse().unwrap() };
        let mut app = App::new(Settings { whats_new, ..settings() });
        app.apply(queue_answer(None, sections(), false));
        app
    }

    fn open_page(app: &mut App) {
        press(app, ":news");
        app.handle_key(code(KeyCode::Enter));
    }

    #[test]
    fn the_first_start_after_an_update_says_how_much_is_new() {
        let mut app = updated_from("0.5.0");
        assert_eq!(app.live_toast().map(|t| t.text.as_str()), Some("revu 0.6.0 · 4 new things · :news"));
        assert!(!app.modal(), "a toast, never a modal");
        assert!(render(&mut app, 100, 12).contains("✓ revu 0.6.0 · 4 new things · :news"));
    }

    #[test]
    fn a_start_with_nothing_new_says_nothing() {
        assert_eq!(updated_from("0.6.0+1").live_toast(), None);
        assert_eq!(app().live_toast(), None);
    }

    #[test]
    fn one_new_thing_is_counted_in_the_singular() {
        assert_eq!(updated_from("0.6.0").live_toast().map(|t| t.text.as_str()), Some("revu 0.6.0 · 1 new thing · :news"));
    }

    #[test]
    fn news_opens_the_page_from_the_command_line_and_esc_closes_it() {
        let mut app = updated_from("0.5.0");
        open_page(&mut app);
        assert_eq!(app.news_page, Some(0));
        assert!(app.modal());
        app.handle_key(code(KeyCode::Esc));
        assert_eq!(app.news_page, None);
        open_page(&mut app);
        press(&mut app, "q");
        assert!(app.news_page.is_none() && !app.should_quit, "q closes the page, it does not quit");
    }

    #[test]
    fn the_page_marks_the_versions_the_reader_has_not_seen() {
        let whats_new = WhatsNew { changelog: sample(), seen: "0.6.0".parse().unwrap() };
        let marked: Vec<(String, bool)> = whats_new.sections().into_iter().map(|(section, new)| (section.title(), new)).collect();
        assert_eq!(marked, [("Unreleased".to_owned(), true), ("0.6.0".to_owned(), false), ("0.5.0".to_owned(), false)]);
    }

    #[test]
    fn the_page_scrolls_and_stops_at_its_last_row() {
        let mut app = updated_from("0.5.0");
        open_page(&mut app);
        press(&mut app, "jj");
        assert_eq!(app.news_page, Some(2));
        press(&mut app, "G");
        let screen = render(&mut app, 100, 14);
        let last = app.news_page.unwrap();
        assert!(last > 0 && last < 40, "the view stops the scroll at the last page: {last}");
        assert!(screen.contains("b pins or unpins it."), "the last entry is on screen:\n{screen}");
        press(&mut app, "g");
        assert_eq!(app.news_page, Some(0));
    }

    #[test]
    fn snapshot_whats_new() {
        let mut app = updated_from("0.5.0");
        open_page(&mut app);
        insta::assert_snapshot!("whats_new", render(&mut app, 100, 34));
        insta::assert_snapshot!("whats_new_narrow", render(&mut app, 60, 20));
    }
}
