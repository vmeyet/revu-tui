//! `a g` and `:session`: the agent session of the MR under the cursor. The runner finds or starts it;
//! when several could be the one, the search box lists them, a new one first.
use super::{Action, App, Focus};
use crate::agent::{Choice, Mr, Session};
use crate::fuzzy;
use crate::share::Fields;
use crate::tui::palette::{Candidate, MAX_SHOWN, Mode, Target};

/// The sessions the reader picks among, and the MR they are for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionPick {
    pub(crate) mr: Mr,
    pub(crate) sessions: Vec<Session>,
}

impl App {
    pub(super) fn open_session(&mut self) -> Vec<Action> {
        if !self.agent {
            self.toast("agent sessions are off: set `[agent] enabled = true` in the config (docs/guides/agent.md)");
            return vec![];
        }
        let Some(mr) = self.session_mr() else {
            self.toast("select an MR first");
            return vec![];
        };
        self.toast(format!("looking for the session of {}…", mr.fields.reference));
        vec![Action::Session { mr: Box::new(mr), choice: Choice::Find }]
    }

    /// The MR under the cursor: the queue's row, or the open MR.
    fn session_mr(&self) -> Option<Mr> {
        match (self.focus, &self.open) {
            (Focus::Queue, _) | (_, None) => self.selected_mr().map(|mr| Mr {
                key: mr.key(),
                fields: Fields::from_queue(mr, self.hosts.kind_of(&mr.key())),
                target: mr.target_branch.clone(),
                mine: mr.author == self.me,
            }),
            (_, Some(open)) => {
                let mr = &open.review.mr;
                let fields = Fields::from_mr(mr, self.hosts.kind_of(&open.key));
                Some(Mr { key: open.key.clone(), fields, target: mr.target_branch.clone(), mine: mr.mine })
            }
        }
    }

    pub(super) fn offer_sessions(&mut self, mr: Mr, sessions: Vec<Session>) {
        self.session_pick = Some(SessionPick { mr, sessions });
        self.open_palette(Mode::Sessions);
    }

    /// A new session first, so `enter` alone starts one, then the sessions that match what is typed.
    pub(super) fn session_candidates(&self, typed: &str) -> Vec<Candidate> {
        let Some(pick) = &self.session_pick else { return vec![] };
        let new = Candidate {
            label: format!("new session for {}", pick.mr.fields.reference),
            detail: String::new(),
            target: Target::Session(None),
        };
        let sessions = pick.sessions.iter().enumerate().map(|(index, session)| {
            let candidate =
                Candidate { label: session.name.clone(), detail: self.session_detail(session), target: Target::Session(Some(index)) };
            (session.name.clone(), candidate)
        });
        let matching = fuzzy::rank(typed, sessions).into_iter().map(|(_, candidate)| candidate);
        std::iter::once(new).chain(matching).take(MAX_SHOWN).collect()
    }

    fn session_detail(&self, session: &Session) -> String {
        let state = if session.live { "running" } else { "stopped" };
        let age = session.started.map(|at| crate::tui::ui::short_age((self.today - at).to_std().unwrap_or_default()));
        age.map_or_else(|| state.to_owned(), |age| format!("{state} · {age}"))
    }

    pub(super) fn pick_session(&mut self, index: Option<usize>) -> Vec<Action> {
        let Some(pick) = self.session_pick.take() else { return vec![] };
        let picked = index.and_then(|index| pick.sessions.get(index));
        let choice = picked.map_or(Choice::New, |session| Choice::Session(Box::new(session.clone())));
        vec![Action::Session { mr: Box::new(pick.mr), choice }]
    }

    /// Taken by the loop, which lends the terminal to the session.
    pub(crate) fn take_handoff(&mut self) -> Option<crate::agent::Handoff> {
        self.handoff.take()
    }

    pub(super) fn apply_session_left(&mut self, said: &str, outcome: Result<(), String>) {
        match outcome {
            Ok(()) => self.toast(format!("back from {said}")),
            Err(message) => self.warn(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::agent::{Call, Choice, Handoff, Session};
    use crate::tui::app::test_support::*;
    use std::sync::Arc;

    fn with_agent(mut app: App) -> App {
        app.agent = true;
        app
    }

    fn session(id: &str, name: &str, live: bool) -> Session {
        Session {
            id: id.into(),
            resume_id: format!("{id}-0000"),
            name: name.into(),
            dir: "/code/widgets".into(),
            live,
            started: Some(today() - chrono::TimeDelta::hours(2)),
        }
    }

    fn asked(actions: &[Action]) -> (&crate::agent::Mr, &Choice) {
        let [Action::Session { mr, choice }] = actions else { panic!("no session asked: {actions:?}") };
        (mr, choice)
    }

    #[test]
    fn disabled_the_key_says_how_to_turn_it_on_and_asks_for_nothing() {
        let mut app = with_queue();
        assert_eq!(press(&mut app, "ag"), [] as [Action; 0]);
        let toast = app.live_toast().unwrap().text.clone();
        assert!(toast.contains("[agent] enabled = true") && toast.contains("docs/guides/agent.md"), "{toast}");
        press(&mut app, ":session");
        assert_eq!(app.handle_key(code(KeyCode::Enter)), [] as [Action; 0]);
    }

    #[test]
    fn in_the_queue_a_g_asks_for_the_session_of_the_selected_mr() {
        let mut app = with_agent(with_queue());
        app.queue_move(0);
        let selected = app.selected_mr().unwrap().clone();
        let actions = press(&mut app, "ag");
        let (mr, choice) = asked(&actions);
        assert_eq!(*choice, Choice::Find);
        assert_eq!((mr.key.clone(), mr.fields.url.as_str()), (selected.key(), selected.web_url.as_str()));
        assert_eq!((mr.fields.branch.as_str(), mr.target.as_str()), (selected.source_branch.as_str(), selected.target_branch.as_str()));
        assert_eq!(mr.mine, selected.author == "nina", "someone else's MR gets a plan only");
        assert!(app.live_toast().unwrap().text.starts_with("looking for the session of !"));
    }

    #[test]
    fn in_the_diff_in_zen_and_from_the_command_line_it_is_the_open_mr() {
        let mut app = with_agent(with_mr(|mr| Mr { mine: true, ..mr }));
        let open = Arc::clone(&app.open.as_ref().unwrap().review.mr);
        let actions = press(&mut app, "ag");
        let (mr, _) = asked(&actions);
        assert_eq!((mr.key.clone(), mr.fields.title.as_str(), mr.mine), (mr_key(), open.title.as_str(), true));
        assert_eq!((mr.fields.branch.as_str(), mr.target.as_str()), (open.source_branch.as_str(), open.target_branch.as_str()));
        press(&mut app, "zz");
        assert!(app.zen);
        assert_eq!(asked(&press(&mut app, "ag")).0.key, mr_key());
        press(&mut app, ":session");
        assert_eq!(asked(&app.handle_key(code(KeyCode::Enter))).0.key, mr_key());
    }

    #[test]
    fn the_compact_layout_reads_the_same_keys() {
        let mut app = with_agent(with_review());
        phone(&mut app);
        assert!(app.compact);
        assert_eq!(asked(&press(&mut app, "ag")).0.key, mr_key());
    }

    #[test]
    fn a_key_bound_to_session_does_what_a_g_does() {
        let mut app = with_agent(with_queue());
        app.keymap = keymap("[bind]\nsession = \"F\"\n");
        app.queue_move(0);
        assert_eq!(*asked(&press(&mut app, "F")).1, Choice::Find);
    }

    fn picking() -> App {
        let mut app = with_agent(with_queue());
        app.queue_move(0);
        let actions = press(&mut app, "ag");
        let mr = asked(&actions).0.clone();
        let sessions = vec![session("11ve0000", "revu: nxt", true), session("01d00000", "widgets: cache", false)];
        app.apply(Incoming::SessionPick { mr: Box::new(mr), sessions });
        app
    }

    #[test]
    fn several_sessions_are_listed_under_a_new_one_with_their_state_and_age() {
        let mut app = picking();
        let palette = app.palette.clone().unwrap();
        assert_eq!(palette.mode, crate::tui::palette::Mode::Sessions);
        let rows: Vec<(String, String)> = app.palette_candidates(&palette).into_iter().map(|c| (c.label, c.detail)).collect();
        let reference = app.session_pick.as_ref().unwrap().mr.fields.reference.clone();
        assert_eq!(
            rows,
            [
                (format!("new session for {reference}"), String::new()),
                ("revu: nxt".into(), "running · 2h".into()),
                ("widgets: cache".into(), "stopped · 2h".into()),
            ]
        );
        let screen = render(&mut app, 120, 30);
        assert!(
            screen.contains("sessions of this checkout") && screen.contains("● revu: nxt") && screen.contains("enter opens it"),
            "{screen}"
        );
    }

    #[test]
    fn enter_on_the_first_row_starts_a_new_session_and_on_another_opens_that_one() {
        let mut app = picking();
        let actions = app.handle_key(code(KeyCode::Enter));
        assert_eq!(*asked(&actions).1, Choice::New);
        assert!(app.palette.is_none() && app.session_pick.is_none());
        let mut app = picking();
        app.handle_key(code(KeyCode::Down));
        app.handle_key(code(KeyCode::Down));
        let actions = app.handle_key(code(KeyCode::Enter));
        assert_eq!(*asked(&actions).1, Choice::Session(Box::new(session("01d00000", "widgets: cache", false))));
    }

    #[test]
    fn typing_narrows_the_sessions_and_keeps_the_new_one_and_esc_asks_for_nothing() {
        let mut app = picking();
        press(&mut app, "cache");
        let labels: Vec<String> = app.palette_candidates(app.palette.as_ref().unwrap()).into_iter().map(|c| c.label).collect();
        assert_eq!(labels.len(), 2, "{labels:?}");
        assert_eq!(labels[1], "widgets: cache");
        assert_eq!(app.handle_key(code(KeyCode::Esc)), [] as [Action; 0]);
        assert!(app.palette.is_none());
    }

    #[test]
    fn outside_tmux_the_loop_takes_the_session_once_and_the_way_back_is_said() {
        let mut app = with_agent(with_queue());
        let handoff = Handoff {
            call: Call {
                argv: vec!["claude".into(), "attach".into(), "ab12cd34".into()],
                dir: None,
                path: None,
                limit: Duration::from_secs(15),
            },
            said: "a new session for !42".into(),
        };
        app.apply(Incoming::SessionReady(handoff.clone()));
        assert_eq!(app.take_handoff(), Some(handoff));
        assert_eq!(app.take_handoff(), None);
        app.apply(Incoming::SessionLeft { said: "a new session for !42".into(), outcome: Ok(()) });
        assert_eq!(app.live_toast().unwrap().text, "back from a new session for !42");
        app.apply(Incoming::SessionLeft { said: "a new session for !42".into(), outcome: Err("cannot run claude: not found".into()) });
        assert!(app.live_toast().unwrap().danger);
    }

    #[test]
    fn a_g_is_in_the_key_list_of_every_pane_and_the_other_a_keys_still_ask() {
        let mut app = with_agent(with_queue());
        press(&mut app, "?");
        assert!(render(&mut app, 120, 40).contains("the agent session of the MR"));
        let mut app = with_agent(with_review());
        press(&mut app, "aa");
        assert!(app.live_toast().unwrap().text.contains("Claude is off"), "a a still asks Claude");
    }
}
