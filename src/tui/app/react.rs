//! `+` in the thread pane, or on a diff line with a thread: react to a note with one of the eight reactions
//! both forges share, or on GitLab with any emoji found by name after `/`. The count moves at once;
//! a refusal puts it back. The reactions I give most on the host lead the row.
use super::{Action, App, EntryKind, Failure, MrKey};
use crate::forge::{Emoji, Note, ReactionChange, ThreadId};
use crossterm::event::{KeyCode, KeyEvent};

/// The picker open on note `note` of thread `thread`, `selected` the reaction `enter` gives.
/// The choices are `row`, or with a `search` (GitLab only) the emoji whose name matches it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pick {
    pub thread: ThreadId,
    pub note: usize,
    pub selected: usize,
    pub row: Vec<Emoji>,
    pub search: Option<Search>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Search {
    pub query: String,
    pub found: Vec<Emoji>,
}

/// How many choices the status line shows at once.
pub const PAGE: usize = 8;

/// How many of my most used reactions lead the row: under half of it, so most of the eight keep their place.
const LEADING: usize = 3;

/// The picker's row: my `LEADING` most used reactions, then the eight in their fixed order without those.
fn row(most_used: &[Emoji]) -> Vec<Emoji> {
    let leading = &most_used[..most_used.len().min(LEADING)];
    let rest = Emoji::ALL.into_iter().filter(|e| !leading.contains(e));
    leading.iter().copied().chain(rest).take(PAGE).collect()
}

impl Pick {
    fn choices(&self) -> &[Emoji] {
        self.search.as_ref().map_or(&self.row, |s| &s.found)
    }
}

impl Search {
    /// Equally good matches keep the order they are listed in, so my most used come first among them.
    fn ranked(query: String, most_used: &[Emoji]) -> Self {
        let rest = Emoji::ALL.into_iter().chain(Emoji::others()).filter(|e| !most_used.contains(e));
        let names = most_used.iter().copied().chain(rest).map(|e| (e.gitlab().to_owned(), e));
        let found = crate::fuzzy::rank(&query, names).into_iter().map(|(_, e)| e).collect();
        Self { query, found }
    }
}

impl App {
    /// `+`: the picker on the focused note, or why there is none.
    pub(super) fn open_react(&mut self) {
        let Some(stop) = self.stop_here() else {
            self.toast("open a thread first");
            return;
        };
        let EntryKind::Note(note) = stop.kind else {
            self.toast("a draft is not published yet: nothing to react to");
            return;
        };
        let Some(thread) = stop.thread else { return };
        self.react = Some(self.pick_on(thread, note));
    }

    /// `+` on a diff line with a thread: the picker on its first note, as the pane would open it,
    /// with the diff keeping the focus so zen stays on it.
    pub(super) fn open_react_here(&mut self) -> bool {
        let Some(open) = &self.open else { return false };
        let place = open.row().filter(|row| open.is_marked(row)).and_then(|row| open.review.place_of(row));
        let Some(thread) = place.and_then(|place| open.review.conversations(&place).into_iter().find_map(|c| c.thread)) else {
            return false;
        };
        self.react = Some(self.pick_on(thread, 0));
        true
    }

    fn pick_on(&self, thread: ThreadId, note: usize) -> Pick {
        Pick { thread, note, selected: 0, row: row(&self.most_used()), search: None }
    }

    pub(super) fn host_of<'a>(&'a self, key: &'a MrKey) -> &'a str {
        key.host.as_deref().unwrap_or(&self.host)
    }

    /// The reactions I gave on the open MR's host that its forge takes, the most used first.
    fn most_used(&self) -> Vec<Emoji> {
        let Some(open) = &self.open else { return vec![] };
        let any = self.any_emoji();
        self.reactions.most_used(self.host_of(&open.key)).into_iter().filter(|e| any || Emoji::ALL.contains(e)).collect()
    }

    /// Whether the open MR's forge takes any emoji, not only the eight.
    fn any_emoji(&self) -> bool {
        self.open.as_ref().is_some_and(|o| self.hosts.kind_of(&o.key).any_emoji())
    }

    pub(super) fn handle_react_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let Some(pick) = self.react.take() else { return vec![] };
        let last = pick.choices().len().saturating_sub(1);
        let moved = |selected: usize| Some(Pick { selected, ..pick.clone() });
        let find = |query: String| Search::ranked(query, &self.most_used());
        let (next, chosen) = match (key.code, &pick.search) {
            (KeyCode::Enter, _) => (None, pick.choices().get(pick.selected).copied()),
            (KeyCode::Right, _) | (KeyCode::Char('l'), None) => (moved((pick.selected + 1).min(last)), None),
            (KeyCode::Left, _) | (KeyCode::Char('h'), None) => (moved(pick.selected.saturating_sub(1)), None),
            (KeyCode::Char(c @ '1'..='8'), None) => (None, pick.row.get(c as usize - '1' as usize).copied()),
            (KeyCode::Char('/'), None) if self.any_emoji() => {
                (Some(Pick { selected: 0, search: Some(find(String::new())), ..pick.clone() }), None)
            }
            (KeyCode::Backspace, Some(search)) => {
                let shorter = search.query.char_indices().next_back().map(|(end, _)| find(search.query[..end].to_owned()));
                (Some(Pick { selected: 0, search: shorter, ..pick.clone() }), None)
            }
            (KeyCode::Char(c), Some(search)) => {
                (Some(Pick { selected: 0, search: Some(find(format!("{}{c}", search.query))), ..pick.clone() }), None)
            }
            _ => (None, None),
        };
        self.react = next;
        chosen.map(|emoji| self.toggle_reaction(&pick, emoji)).unwrap_or_default()
    }

    /// Mine on or off, shown at once, then asked of the forge; one given is counted.
    fn toggle_reaction(&mut self, pick: &Pick, emoji: Emoji) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let Some(note) = open.review.thread(&pick.thread).and_then(|t| t.notes.get(pick.note)).cloned() else { return vec![] };
        let change = if note.reactions.iter().any(|r| r.emoji == emoji && r.mine) { ReactionChange::Remove } else { ReactionChange::Add };
        let host = self.host_of(&open.key).to_owned();
        let react =
            Action::React { key: open.key.clone(), thread: pick.thread.clone(), index: pick.note, note: Box::new(note), emoji, change };
        self.update_open(|open| open.relaid(|review| review.with_reaction(&pick.thread, pick.note, emoji, change)));
        if change == ReactionChange::Remove {
            return vec![react];
        }
        vec![react, self.count_reaction(host, emoji)]
    }

    fn count_reaction(&mut self, host: String, emoji: Emoji) -> Action {
        self.reactions = self.reactions.with(&host, emoji);
        Action::CountReaction { host, emoji }
    }

    /// The forge refused: the count goes back to what it was.
    pub(super) fn react_failed(&mut self, thread: &ThreadId, index: usize, emoji: Emoji, change: ReactionChange, message: &str) {
        self.update_open(|open| open.relaid(|review| review.with_reaction(thread, index, emoji, change.undone())));
        self.warn(format!("no reaction: {message}"));
    }

    /// The picker as the status line shows it: the search typed so far, then the page of choices
    /// around the selected one, my reactions standing out, the selected one framed.
    pub fn react_prompt(&self) -> Option<Prompt> {
        let pick = self.react.as_ref()?;
        let note: Option<&Note> = self.open.as_ref()?.review.thread(&pick.thread).and_then(|t| t.notes.get(pick.note));
        let mine = |emoji: Emoji| note.is_some_and(|n| n.reactions.iter().any(|r| r.emoji == emoji && r.mine));
        let first = pick.selected - pick.selected % PAGE;
        let label = |i: usize, emoji: Emoji| {
            let face = if self.ascii { emoji.text() } else { emoji.glyph() };
            match &pick.search {
                None => format!("{} {face}", i + 1),
                Some(_) if self.ascii => face.to_owned(),
                Some(_) => format!("{face} {}", emoji.gitlab()),
            }
        };
        let choices = pick.choices().iter().enumerate().skip(first).take(PAGE);
        Some(Prompt {
            search: pick.search.as_ref().map(|s| s.query.clone()),
            more: self.any_emoji(),
            choices: choices.map(|(i, &emoji)| (label(i, emoji), mine(emoji), i == pick.selected)).collect(),
        })
    }
}

/// What the status line shows while the picker is open.
pub struct Prompt {
    /// The name typed so far, once `/` started a search.
    pub search: Option<String>,
    /// Whether `/` searches every emoji: the forge takes any.
    pub more: bool,
    /// Each choice's label, whether it is mine, and whether it is selected.
    pub choices: Vec<(String, bool, bool)>,
}

/// The reaction the failure was about, for `Failure::React`.
pub fn failure(thread: ThreadId, index: usize, emoji: Emoji, change: ReactionChange) -> Failure {
    Failure::React { thread, index, emoji, change }
}

#[cfg(test)]
mod tests {
    use super::row;
    use crate::reactions::History;
    use crate::tui::app::test_support::*;

    /// A thread open in the pane whose one note already has `👍 2`, one of them mine.
    fn with_reactions() -> App {
        let mut app = with_suggestion("Nice", json!([]));
        let mut all = discussions();
        let mut note: serde_json::Value = serde_json::from_str(include_str!("../../forge/gitlab/fixtures/diff_note.json")).unwrap();
        note["id"] = json!("5ugg");
        note["notes"][0]["body"] = json!("Nice");
        note["notes"][0]["position"]["new_line"] = json!(13);
        let mut thread = fixture::discussion(&note.to_string());
        thread.notes[0].node = Some("gid://gitlab/DiffNote/1".into());
        thread.notes[0].reactions = vec![crate::forge::Reaction { emoji: Emoji::ThumbsUp, count: 2, mine: true }];
        all.push(thread);
        app.apply(Incoming::Discussions { key: mr_key(), discussions: all });
        app
    }

    fn reactions_of(app: &App) -> Vec<crate::forge::Reaction> {
        app.open.as_ref().unwrap().review.thread(&"5ugg".into()).unwrap().notes[0].reactions.clone()
    }

    fn hundred() -> Emoji {
        Emoji::from_gitlab("100").unwrap()
    }

    /// The history after giving each of `given` once on gitlab.com, in that order.
    fn given(given: &[Emoji]) -> History {
        given.iter().fold(History::default(), |history, &emoji| history.with("gitlab.com", emoji))
    }

    fn row_on_plus(app: &mut App) -> Vec<Emoji> {
        press(app, "+");
        app.react.take().unwrap().row
    }

    /// The diff's cursor moved onto the line whose thread has the note, the pane closed.
    fn move_to_the_line(app: &mut App) {
        app.close_pane();
        let mut open = app.open.clone().unwrap();
        let place = Place::Line { file: 0, new: Some(13), old: None };
        open.selected = open.rows.iter().position(|r| open.review.place_of(r).as_ref() == Some(&place)).unwrap();
        app.open = Some(open);
    }

    fn on_the_line() -> App {
        let mut app = with_reactions();
        move_to_the_line(&mut app);
        app
    }

    fn in_zen() -> App {
        let mut app = on_the_line();
        app.zen = true;
        app
    }

    #[test]
    fn with_no_history_the_row_is_the_eight_in_their_order() {
        assert_eq!(row(&[]), Emoji::ALL);
        assert_eq!(row_on_plus(&mut with_reactions()), Emoji::ALL);
    }

    #[test]
    fn the_most_used_lead_and_the_eight_follow_without_them() {
        use Emoji::{Confused, Eyes, Heart, Hooray, Laugh, Rocket, ThumbsDown, ThumbsUp};
        assert_eq!(row(&[Eyes]), [Eyes, ThumbsUp, ThumbsDown, Laugh, Confused, Heart, Hooray, Rocket]);
        assert_eq!(row(&[Eyes, Rocket, Heart, Laugh]), [Eyes, Rocket, Heart, ThumbsUp, ThumbsDown, Laugh, Confused, Hooray], "three lead");
    }

    #[test]
    fn on_gitlab_an_emoji_found_by_name_can_lead_and_the_last_of_the_eight_make_room() {
        use Emoji::{Confused, Eyes, Heart, Hooray, Laugh, ThumbsDown, ThumbsUp};
        let mut app = with_reactions();
        app.reactions = given(&[hundred(), Eyes, hundred()]);
        assert_eq!(row_on_plus(&mut app), [hundred(), Eyes, ThumbsUp, ThumbsDown, Laugh, Confused, Heart, Hooray]);
    }

    #[test]
    fn on_github_only_the_eight_lead() {
        let mut app = with_reactions();
        app.hosts = crate::forge::Hosts::one("gitlab.com", Kind::GitHub);
        app.reactions = given(&[hundred(), hundred(), Emoji::Eyes]);
        let row = row_on_plus(&mut app);
        assert_eq!(row[0], Emoji::Eyes);
        assert!(row.iter().all(|e| Emoji::ALL.contains(e)), "{row:?}");
    }

    #[test]
    fn equal_counts_go_to_the_latest_given() {
        let mut app = with_reactions();
        app.reactions = given(&[Emoji::Heart, Emoji::Rocket, Emoji::Eyes, Emoji::Laugh]);
        assert_eq!(row_on_plus(&mut app)[..3], [Emoji::Laugh, Emoji::Eyes, Emoji::Rocket]);
    }

    #[test]
    fn counts_are_kept_per_host() {
        let mut app = with_reactions();
        app.reactions = History::default().with("github.com", Emoji::Eyes);
        assert_eq!(row_on_plus(&mut app), Emoji::ALL, "github.com's counts stay on github.com");
    }

    #[test]
    fn a_reaction_is_counted_when_given_not_when_taken_off() {
        let mut app = with_reactions();
        let given = press(&mut app, "+3");
        assert!(
            matches!(given.as_slice(), [Action::React { change: ReactionChange::Add, .. }, Action::CountReaction { host, emoji: Emoji::Laugh }] if host == "gitlab.com")
        );
        assert_eq!(app.reactions.most_used("gitlab.com"), [Emoji::Laugh]);
        let taken = press(&mut app, "+1");
        assert!(matches!(taken.as_slice(), [Action::React { emoji: Emoji::Laugh, change: ReactionChange::Remove, .. }]), "{taken:?}");
        assert_eq!(app.reactions, History::default().with("gitlab.com", Emoji::Laugh));
    }

    #[test]
    fn the_pane_and_the_diff_line_open_the_same_row() {
        let mut app = with_reactions();
        app.reactions = given(&[Emoji::Rocket]);
        let in_pane = row_on_plus(&mut app);
        move_to_the_line(&mut app);
        assert_eq!(row_on_plus(&mut app), in_pane);
        assert_eq!(in_pane[0], Emoji::Rocket);
    }

    #[test]
    fn a_search_puts_my_most_used_first_among_equal_matches() {
        let mut app = with_reactions();
        app.reactions = given(&[hundred()]);
        press(&mut app, "+/");
        assert_eq!(app.react.as_ref().unwrap().choices()[..2], [hundred(), Emoji::ThumbsUp], "an empty search matches all alike");
    }

    #[test]
    fn the_picker_row_with_history() {
        let mut app = with_reactions();
        app.reactions = given(&[hundred(), Emoji::Eyes, hundred(), Emoji::Rocket]);
        press(&mut app, "+");
        let screen = render(&mut app, 150, 24);
        insta::assert_snapshot!("react_row_with_history", screen.lines().last().unwrap());
    }

    #[test]
    fn plus_opens_the_picker_and_a_digit_toggles_my_reaction_at_once() {
        let mut app = with_reactions();
        let screen = render(&mut app, 150, 24);
        assert!(screen.contains("👍") && screen.contains(" 2"), "the count shows under the note:\n{screen}");
        assert_eq!(press(&mut app, "+"), vec![]);
        assert!(render(&mut app, 150, 24).contains("react"), "the picker takes the status line");
        let actions = press(&mut app, "7");
        let [Action::React { emoji, change, note, .. }, _] = actions.as_slice() else { panic!("{actions:?}") };
        assert_eq!((*emoji, *change, note.node.as_deref()), (Emoji::Rocket, ReactionChange::Add, Some("gid://gitlab/DiffNote/1")));
        assert!(reactions_of(&app).contains(&crate::forge::Reaction { emoji: Emoji::Rocket, count: 1, mine: true }));
        assert!(app.react.is_none(), "one pick closes the picker");
        press(&mut app, "+");
        assert!(app.react.is_some(), "the picker opens again");
        let actions = press(&mut app, "2");
        let [Action::React { emoji, change, .. }] = actions.as_slice() else { panic!("{actions:?}") };
        assert_eq!((*emoji, *change), (Emoji::ThumbsUp, ReactionChange::Remove), "a pick of my own reaction takes it off");
        assert_eq!(reactions_of(&app)[0], crate::forge::Reaction { emoji: Emoji::ThumbsUp, count: 1, mine: false });
    }

    #[test]
    fn plus_on_a_diff_line_with_a_thread_reacts_there_and_elsewhere_does_nothing() {
        let mut app = in_zen();
        assert_eq!(press(&mut app, "+"), vec![]);
        assert_eq!(app.react.as_ref().map(|p| (p.thread.as_str(), p.note)), Some(("5ugg", 0)), "the line's first note");
        assert_eq!(app.focus, Focus::Review, "the diff keeps the focus");
        assert!(render(&mut app, 150, 24).contains("react"), "zen shows the picker");
        let actions = press(&mut app, "7");
        assert!(matches!(actions.as_slice(), [Action::React { emoji: Emoji::Rocket, change: ReactionChange::Add, .. }, _]), "{actions:?}");
        let mut open = app.open.clone().unwrap();
        open.selected = open.rows.iter().position(|r| matches!(r, Row::Line { .. }) && !open.is_marked(r)).unwrap();
        app.open = Some(open);
        assert_eq!(press(&mut app, "+"), vec![]);
        assert!(app.react.is_none(), "a line without a thread");
    }

    #[test]
    fn on_gitlab_slash_searches_every_emoji_by_name() {
        let mut app = with_reactions();
        press(&mut app, "+");
        assert!(render(&mut app, 150, 24).contains("/ any emoji"), "the picker says it can search");
        press(&mut app, "/100");
        let screen = render(&mut app, 150, 24);
        assert!(screen.contains("/100▏") && screen.contains("💯"), "{screen}");
        let actions = app.handle_key(code(KeyCode::Enter));
        let [Action::React { emoji, change, .. }, _] = actions.as_slice() else { panic!("{actions:?}") };
        assert_eq!((emoji.gitlab(), *change), ("100", ReactionChange::Add));
        assert!(reactions_of(&app).iter().any(|r| r.emoji.glyph() == "💯" && r.mine), "the count moves at once");
        press(&mut app, "+");
        press(&mut app, "/r");
        app.handle_key(code(KeyCode::Backspace));
        assert_eq!(app.react.as_ref().unwrap().search.as_ref().map(|s| s.query.as_str()), Some(""));
        app.handle_key(code(KeyCode::Backspace));
        assert_eq!(app.react.as_ref().unwrap().search, None, "an empty search goes back to the eight");
        press(&mut app, "zzz");
        assert!(app.react.is_none(), "outside a search, other keys close the picker");
    }

    #[test]
    fn on_github_the_picker_keeps_the_eight() {
        let mut app = with_reactions();
        app.hosts = crate::forge::Hosts::one("gitlab.com", Kind::GitHub);
        press(&mut app, "+");
        assert!(!render(&mut app, 150, 24).contains("any emoji"));
        press(&mut app, "/");
        assert!(app.react.is_none(), "no search: the key closes the picker");
    }

    #[test]
    fn the_picker_moves_with_h_and_l_and_esc_closes_it_without_a_change() {
        let mut app = with_reactions();
        press(&mut app, "+");
        press(&mut app, "lll");
        assert_eq!(app.react.as_ref().unwrap().selected, 3);
        assert_eq!(app.handle_key(code(KeyCode::Esc)), vec![]);
        assert!(app.react.is_none());
        assert_eq!(reactions_of(&app), vec![crate::forge::Reaction { emoji: Emoji::ThumbsUp, count: 2, mine: true }]);
    }

    #[test]
    fn a_refused_reaction_puts_the_count_back() {
        let mut app = with_reactions();
        press(&mut app, "+");
        press(&mut app, "6");
        assert_eq!(reactions_of(&app).len(), 2);
        app.apply(Incoming::Failed {
            what: react_failure("5ugg".into(), 0, Emoji::Hooray, ReactionChange::Add),
            message: "GitLab refused the reaction".into(),
        });
        assert_eq!(reactions_of(&app), vec![crate::forge::Reaction { emoji: Emoji::ThumbsUp, count: 2, mine: true }]);
        assert!(app.live_toast().unwrap().text.starts_with("no reaction:"));
    }

    /// The status rows the picker takes, from its label to the bottom of the screen.
    fn picker_rows(app: &mut App, width: u16, height: u16) -> String {
        let screen = render(app, width, height);
        let rows: Vec<&str> = screen.lines().skip_while(|row| !row.starts_with(" react")).collect();
        assert!(!rows.is_empty(), "the picker shows:\n{screen}");
        rows.join("\n")
    }

    fn shows_every_choice(rows: &str) -> bool {
        Emoji::ALL.iter().enumerate().all(|(i, emoji)| rows.contains(&format!("{} {}", i + 1, emoji.glyph())))
    }

    #[test]
    fn snapshot_the_picker_on_small_screens() {
        for (width, ascii, search) in [
            (50, false, ""),
            (60, false, ""),
            (80, false, ""),
            (50, true, ""),
            (50, false, "/"),
            (60, false, "/"),
            (80, false, "/"),
            (50, true, "/"),
        ] {
            let mut app = with_reactions();
            app.ascii = ascii;
            press(&mut app, &format!("+{search}"));
            let name =
                format!("react_picker_{width}{}{}", if search.is_empty() { "" } else { "_search" }, if ascii { "_ascii" } else { "" });
            insta::assert_snapshot!(name, picker_rows(&mut app, width, 30));
        }
    }

    #[test]
    fn the_key_hint_goes_before_the_choices_wrap() {
        let mut app = with_reactions();
        press(&mut app, "+");
        assert!(picker_rows(&mut app, 150, 24).contains("esc"), "a wide screen keeps the hint");
        let rows = picker_rows(&mut app, 80, 24);
        assert_eq!(rows.lines().count(), 1, "{rows}");
        assert!(shows_every_choice(&rows) && !rows.contains("esc"), "{rows}");
        let rows = picker_rows(&mut app, 50, 24);
        assert!(rows.lines().count() > 1 && !rows.contains("esc"), "{rows}");
    }

    #[test]
    fn wrapped_every_choice_shows_and_its_digit_picks_it() {
        let mut app = with_reactions();
        press(&mut app, "+");
        let rows = picker_rows(&mut app, 50, 30);
        assert!(shows_every_choice(&rows), "{rows}");
        let actions = press(&mut app, "8");
        assert!(matches!(actions.as_slice(), [Action::React { emoji: Emoji::Eyes, change: ReactionChange::Add, .. }, _]), "{actions:?}");
    }

    fn on_a_thread_row() -> App {
        let mut app = with_reactions();
        render(&mut app, 50, 30);
        app.close_pane();
        walk_to_any_thread(&mut app, Toward::Next);
        press(&mut app, "j");
        assert!(app.on_thread_row());
        app
    }

    fn under_a_search() -> App {
        let mut app = with_reactions();
        app.close_pane();
        press(&mut app, "/client");
        app.handle_key(code(KeyCode::Enter));
        move_to_the_line(&mut app);
        assert!(app.search.is_some(), "the search stays");
        app
    }

    #[test]
    fn the_picker_wraps_wherever_it_opens() {
        let places = vec![
            ("the thread pane", with_reactions(), 30),
            ("a diff line", on_the_line(), 30),
            ("a diff line under a search", under_a_search(), 30),
            ("a thread row", on_a_thread_row(), 30),
            ("zen", in_zen(), 30),
            ("a short screen", on_the_line(), 14),
        ];
        for (place, mut app, height) in places {
            press(&mut app, "+");
            assert!(app.react.is_some(), "{place}: the picker opens");
            let screen = render(&mut app, 50, height);
            assert_eq!(screen.lines().count(), usize::from(height), "{place}: the bottom row is the picker's");
            assert!(shows_every_choice(&picker_rows(&mut app, 50, height)), "{place}:\n{screen}");
            assert!(!app.shows_taps(), "{place}: the tap bar hides");
        }
    }

    #[test]
    fn ascii_draws_reactions_in_plain_words() {
        let mut app = with_reactions();
        app.ascii = true;
        let screen = render(&mut app, 150, 24);
        assert!(screen.contains("+1 2") && !screen.contains("👍"), "{screen}");
    }
}
