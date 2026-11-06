//! `a` and `:session`: the agent session of the MR, or on `revu local` the branch, under the cursor. The runner finds
//! or starts it; when several could be the one, the search box lists them, a new one first.
use super::{Action, App, Focus};
use crate::agent::{Branch, Choice, Mr, Session, SessionMark, Subject};
use crate::forge::QueueMr;
use crate::share::Fields;
use crate::tui::palette::{Candidate, Mode, Target};
use vtui::fuzzy;
use vtui::palette::MAX_SHOWN;

/// The sessions the reader picks among, and what they are for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionPick {
    pub(crate) subject: Subject,
    pub(crate) sessions: Vec<Session>,
}

impl App {
    pub(super) fn open_session(&mut self) -> Vec<Action> {
        if !self.agent {
            self.toast("agent sessions are off: set `[agent] enabled = true` in the config (docs/guides/agent.md)");
            return vec![];
        }
        let Some(subject) = self.session_subject() else {
            self.toast("select an MR first");
            return vec![];
        };
        self.toast(format!("looking for the session of {}…", subject.reference()));
        vec![Action::Session { subject: Box::new(subject), choice: Choice::Find }]
    }

    /// What is under the cursor: the queue's row, or the open MR or worktree.
    fn session_subject(&self) -> Option<Subject> {
        match (self.focus, &self.open) {
            (Focus::Queue, _) | (_, None) => self.selected_mr().map(|row| self.subject_of(row)),
            (_, Some(open)) if open.key.is_local() => self.queue_mr(&open.key).map(|row| self.subject_of(row)),
            (_, Some(open)) => {
                let mr = &open.review.mr;
                let fields = Fields::from_mr(mr, self.hosts.kind_of(&open.key));
                Some(Subject::Mr(Mr { key: open.key.clone(), fields, target: mr.target_branch.clone(), mine: mr.mine }))
            }
        }
    }

    /// A queue row's subject: a worktree's branch, else the MR.
    fn subject_of(&self, row: &QueueMr) -> Subject {
        let key = row.key();
        match &row.worktree {
            Some(worktree) => Subject::Branch(Branch {
                key,
                name: row.source_branch.clone(),
                dir: worktree.dir.clone(),
                target: row.target_branch.clone(),
            }),
            None => Subject::Mr(Mr {
                fields: Fields::from_queue(row, self.hosts.kind_of(&key)),
                key,
                target: row.target_branch.clone(),
                mine: row.author == self.me,
            }),
        }
    }

    /// Which rows have a session, asked of the agent once for every row of the queue.
    pub(super) fn ask_session_marks(&mut self) {
        let Some(sections) = self.sections.as_ref().filter(|_| self.agent) else { return };
        let subjects = sections.all().map(|row| self.subject_of(row)).collect();
        self.composed.push(Action::SessionMarks(subjects));
    }

    /// The row's session, as the last listing said.
    pub(crate) fn session_mark(&self, row: &QueueMr) -> Option<SessionMark> {
        self.session_marks.get(&row.key()).copied()
    }

    pub(super) fn offer_sessions(&mut self, subject: Subject, sessions: Vec<Session>) {
        self.session_pick = Some(SessionPick { subject, sessions });
        self.open_palette(Mode::Sessions);
    }

    /// A new session first, so `enter` alone starts one, then the sessions that match what is typed.
    pub(super) fn session_candidates(&self, typed: &str) -> Vec<Candidate> {
        let Some(pick) = &self.session_pick else { return vec![] };
        let new = Candidate {
            label: format!("new session for {}", pick.subject.reference()),
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
        vec![Action::Session { subject: Box::new(pick.subject), choice }]
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
        self.ask_session_marks();
    }
}

#[cfg(test)]
mod tests {
    use crate::agent::{Call, Choice, Handoff, Session, SessionMark, Subject};
    use crate::forge::QueueMr;
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

    fn asked(actions: &[Action]) -> (&Subject, &Choice) {
        let [Action::Session { subject, choice }] = actions else { panic!("no session asked: {actions:?}") };
        (subject, choice)
    }

    fn asked_mr(actions: &[Action]) -> (&crate::agent::Mr, &Choice) {
        let (Subject::Mr(mr), choice) = asked(actions) else { panic!("not an MR: {actions:?}") };
        (mr, choice)
    }

    #[test]
    fn disabled_the_key_says_how_to_turn_it_on_and_asks_for_nothing() {
        let mut app = with_queue();
        assert_eq!(press(&mut app, "a"), [] as [Action; 0]);
        let toast = app.live_toast().unwrap().text.clone();
        assert!(toast.contains("[agent] enabled = true") && toast.contains("docs/guides/agent.md"), "{toast}");
        press(&mut app, ":session");
        assert_eq!(app.handle_key(code(KeyCode::Enter)), [] as [Action; 0]);
    }

    #[test]
    fn in_the_queue_a_asks_for_the_session_of_the_selected_mr() {
        let mut app = with_agent(with_queue());
        app.queue_move(0);
        let selected = app.selected_mr().unwrap().clone();
        let actions = press(&mut app, "a");
        let (mr, choice) = asked_mr(&actions);
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
        let actions = press(&mut app, "a");
        let (mr, _) = asked_mr(&actions);
        assert_eq!((mr.key.clone(), mr.fields.title.as_str(), mr.mine), (mr_key(), open.title.as_str(), true));
        assert_eq!((mr.fields.branch.as_str(), mr.target.as_str()), (open.source_branch.as_str(), open.target_branch.as_str()));
        press(&mut app, "zz");
        assert!(app.zen);
        assert_eq!(asked(&press(&mut app, "a")).0.key(), &mr_key());
        press(&mut app, ":session");
        assert_eq!(asked(&app.handle_key(code(KeyCode::Enter))).0.key(), &mr_key());
    }

    #[test]
    fn the_compact_layout_reads_the_same_keys() {
        let mut app = with_agent(with_review());
        phone(&mut app);
        assert!(app.compact);
        assert_eq!(asked(&press(&mut app, "a")).0.key(), &mr_key());
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
        let actions = press(&mut app, "a");
        let subject = asked(&actions).0.clone();
        let sessions = vec![session("11ve0000", "revu: nxt", true), session("01d00000", "widgets: cache", false)];
        app.apply(Incoming::SessionPick { subject: Box::new(subject), sessions });
        app
    }

    #[test]
    fn several_sessions_are_listed_under_a_new_one_with_their_state_and_age() {
        let mut app = picking();
        let palette = app.palette.clone().unwrap();
        assert_eq!(palette.mode, crate::tui::palette::Mode::Sessions);
        let rows: Vec<(String, String)> = app.palette_candidates(&palette).into_iter().map(|c| (c.label, c.detail)).collect();
        let reference = app.session_pick.as_ref().unwrap().subject.reference().to_owned();
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
    fn a_held_down_asks_once() {
        let mut app = with_agent(with_review());
        assert_eq!(press(&mut app, "aaa").len(), 1);
        app.now += Duration::from_secs(1);
        assert_eq!(asked(&press(&mut app, "a")).0.key(), &mr_key(), "a press after the key was let go asks again");
    }

    #[test]
    fn a_alone_asks_and_waits_for_no_second_key() {
        let mut app = with_agent(with_review());
        assert_eq!(asked(&press(&mut app, "a")).0.key(), &mr_key());
        assert_eq!(app.pending, None);
    }

    #[test]
    fn a_is_in_the_key_list_of_every_pane() {
        let mut app = with_agent(with_queue());
        press(&mut app, "?");
        assert!(render(&mut app, 120, 40).contains("the agent session of the MR"));
    }

    fn queue(app: &mut App) {
        let sections = sections();
        app.apply(Incoming::Queue { scope: None, me: "nina".into(), sections, opened: HashMap::new(), cached: false });
    }

    fn marks_asked(actions: &[Action]) -> Vec<&Subject> {
        actions
            .iter()
            .filter_map(|action| if let Action::SessionMarks(subjects) = action { Some(subjects) } else { None })
            .flatten()
            .collect()
    }

    #[test]
    fn a_fresh_queue_asks_once_which_of_its_rows_have_a_session_and_only_with_the_agent_on() {
        let mut on = with_agent(app());
        queue(&mut on);
        let actions = on.take_actions();
        let keys: Vec<&MrKey> = marks_asked(&actions).into_iter().map(Subject::key).collect();
        let rows: Vec<MrKey> = on.sections.as_ref().unwrap().all().map(QueueMr::key).collect();
        assert_eq!(keys, rows.iter().collect::<Vec<_>>());
        assert_eq!(actions.iter().filter(|action| matches!(action, Action::SessionMarks(_))).count(), 1);
        let mut off = app();
        queue(&mut off);
        assert_eq!(marks_asked(&off.take_actions()), [] as [&Subject; 0]);
    }

    #[test]
    fn a_session_shown_or_left_asks_the_marks_again() {
        let mut app = with_agent(with_queue());
        app.take_actions();
        app.apply(Incoming::SessionShown("a new session for !42 · in a tmux window".into()));
        assert_eq!(app.live_toast().unwrap().text, "a new session for !42 · in a tmux window");
        assert_ne!(marks_asked(&app.take_actions()), [] as [&Subject; 0]);
        app.apply(Incoming::SessionLeft { said: "a new session for !42".into(), outcome: Ok(()) });
        assert_ne!(marks_asked(&app.take_actions()), [] as [&Subject; 0]);
    }

    /// The queue with the agent on, the first row's session running and the second's stopped.
    fn marked(app: App) -> App {
        let mut app = with_agent(app);
        let keys: Vec<MrKey> = app.sections.as_ref().unwrap().all().map(QueueMr::key).collect();
        let marks = [(keys[0].clone(), SessionMark::Running), (keys[1].clone(), SessionMark::Stopped)];
        app.apply(Incoming::SessionMarks(marks.into_iter().collect()));
        app.opened.insert(keys[1].clone(), "2026-09-01T00:00:00Z".parse().unwrap());
        app.now = app.started;
        app
    }

    #[test]
    fn a_running_session_is_lit_and_a_stopped_one_faded() {
        let mut app = marked(with_queue());
        let screen = cells(&mut app, 100, 16);
        assert_eq!((cell_of(&screen, "✻ 2h").fg, cell_of(&screen, "✻ 4h").fg), (app.theme.link, app.theme.faded));
    }

    #[test]
    fn snapshot_a_session_marks_its_row_before_the_age_away_from_the_badges() {
        insta::assert_snapshot!("queue_sessions", render(&mut marked(with_queue()), 100, 16));
    }

    #[test]
    fn snapshot_in_the_compact_queue_a_session_takes_a_column_of_its_own() {
        let mut app = with_queue();
        app.queue_layout = crate::config::QueueLayout::Compact;
        insta::assert_snapshot!("queue_compact_sessions", render(&mut marked(app), 100, 16));
    }

    #[test]
    fn snapshot_on_a_phone_the_session_stays_before_the_age_and_the_marks_follow() {
        insta::assert_snapshot!("queue_phone_sessions", phone(&mut marked(with_queue())));
    }
}
