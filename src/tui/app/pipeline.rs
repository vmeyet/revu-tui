//! The pipeline pane (`p`): the jobs of the head commit's CI run, failed ones first in each stage.
use super::{Action, App, Focus, Open};
use crate::forge::checks::{Checks, Job, JobState};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::time::{Duration, Instant};

/// How often a run still going is asked again while the pane shows it.
const WHILE_RUNNING: Duration = Duration::from_secs(15);

/// The pane while it is open: what the forge said, the job under the cursor, the next refresh.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pipeline {
    pub run: Run,
    pub selected: usize,
    pub due: Option<Instant>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Run {
    Waiting,
    /// Nothing ran on the head commit.
    Nothing,
    Ready(Checks),
    Failed(String),
}

impl Pipeline {
    fn waiting() -> Self {
        Self { run: Run::Waiting, selected: 0, due: None }
    }

    pub fn jobs(&self) -> Vec<&Job> {
        match &self.run {
            Run::Ready(checks) => checks.jobs().collect(),
            _ => vec![],
        }
    }

    /// The first failed job, where the cursor lands when the run arrives.
    fn first_failure(checks: &Checks) -> usize {
        checks.jobs().position(|j| j.state == JobState::Failed && !j.allowed_to_fail).unwrap_or(0)
    }
}

impl Open {
    fn with_pipeline(self, pipeline: Option<Pipeline>) -> Self {
        Self { pipeline, ..self }
    }
}

impl App {
    pub(super) fn pipeline_open(&self) -> bool {
        self.open.as_ref().is_some_and(|o| o.pipeline.is_some())
    }

    /// `p`: the pipeline takes the right pane; `p` again gives it back.
    pub(super) fn toggle_pipeline(&mut self) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        if open.pipeline.is_some() {
            self.update_open(|open| open.with_pipeline(None));
            self.focus = Focus::Review;
            return vec![];
        }
        let action = load(open);
        self.update_open(|open| Open {
            pane: None,
            tree: None,
            answer: None,
            outline: None,
            ..open.with_pipeline(Some(Pipeline::waiting()))
        });
        self.focus = Focus::Side;
        vec![action]
    }

    pub(super) fn handle_pipeline_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let Some(pipeline) = &open.pipeline else { return vec![] };
        let jobs = pipeline.jobs();
        let last = jobs.len().saturating_sub(1);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let url = jobs.get(pipeline.selected).map(|j| j.web_url.clone()).or_else(|| match &pipeline.run {
            Run::Ready(checks) => checks.web_url.clone(),
            _ => open.review.mr.pipeline.as_ref().and_then(|p| p.web_url.clone()),
        });
        let selected = match key.code {
            KeyCode::Char('j') | KeyCode::Down => pipeline.selected + 1,
            KeyCode::Char('k') | KeyCode::Up => pipeline.selected.saturating_sub(1),
            KeyCode::Char('d') if ctrl => pipeline.selected + 10,
            KeyCode::Char('u') if ctrl => pipeline.selected.saturating_sub(10),
            KeyCode::Char('g') => 0,
            KeyCode::Char('G') => last,
            KeyCode::Char('o') => return url.map(|u| vec![Action::OpenUrl(u)]).unwrap_or_default(),
            KeyCode::Char('y') => return url.map(|u| vec![Action::Yank(u)]).unwrap_or_default(),
            KeyCode::Char('r') => {
                let action = load(open);
                self.update_open(|open| Open { pipeline: open.pipeline.map(|p| Pipeline { due: None, ..p }), ..open });
                return vec![action];
            }
            KeyCode::Esc | KeyCode::Char('p' | 'x') => {
                self.focus = Focus::Review;
                self.update_open(|open| open.with_pipeline(None));
                return vec![];
            }
            _ => return vec![],
        };
        self.update_open(|open| Open { pipeline: open.pipeline.map(|p| Pipeline { selected: selected.min(last), ..p }), ..open });
        vec![]
    }

    /// The open MR's review apps asked of the forge; what was known stays shown until the answer.
    pub(super) fn ask_deployments(&mut self) {
        let Some(open) = &self.open else { return };
        let mr = &open.review.mr;
        self.composed.push(Action::LoadDeployments { key: open.key.clone(), branch: mr.source_branch.clone(), head: mr.refs.head.clone() });
        self.update_open(|open| Open { deployments: Some(open.deployments.unwrap_or_default()), ..open });
    }

    /// The run arrived; a run still going is asked again in a while, as long as the pane shows it.
    pub(super) fn apply_checks(&mut self, key: &super::MrKey, checks: Option<Checks>) {
        let Some(pipeline) = self.open.as_ref().filter(|o| &o.key == key).and_then(|o| o.pipeline.clone()) else { return };
        let (run, due) = match checks {
            None => (Run::Nothing, None),
            Some(checks) => {
                let due = (checks.state() == JobState::Running).then(|| self.now + WHILE_RUNNING);
                (Run::Ready(checks), due)
            }
        };
        let selected = match (&pipeline.run, &run) {
            (Run::Ready(_), Run::Ready(_)) => pipeline.selected,
            (_, Run::Ready(checks)) => Pipeline::first_failure(checks),
            _ => 0,
        };
        let finished = matches!(&run, Run::Ready(checks) if checks.state() != JobState::Running);
        self.update_open(|open| open.with_pipeline(Some(Pipeline { run, selected, due })));
        if finished {
            self.ask_deployments();
        }
    }

    pub(super) fn checks_failed(&mut self, message: String) {
        if self.pipeline_open() {
            self.update_open(|open| open.with_pipeline(Some(Pipeline { run: Run::Failed(message), selected: 0, due: None })));
        }
    }

    /// The pane's run, asked again once its refresh is due.
    pub(super) fn pipeline_tick(&mut self) -> Option<Action> {
        let open = self.open.as_ref()?;
        let pipeline = open.pipeline.clone()?;
        if pipeline.due.is_none_or(|due| self.now < due) {
            return None;
        }
        let action = load(open);
        self.update_open(|open| open.with_pipeline(Some(Pipeline { due: None, ..pipeline })));
        Some(action)
    }
}

fn load(open: &Open) -> Action {
    Action::LoadChecks { key: open.key.clone(), head: open.review.mr.refs.head.clone() }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crate::tui::app::test_support::*;

    #[test]
    fn review_apps_show_in_the_header_the_pipeline_and_the_cover_and_reload_once_a_run_ends() {
        use crate::forge::Deployment;
        let mut app = with_pipeline();
        assert!(app.take_actions().iter().any(|a| matches!(a, Action::LoadDeployments { .. })), "a finished run");
        let deployments = vec![
            Deployment { environment: "storybook/feat-checkout".into(), url: "https://sb.acme.test/feat-checkout".into(), current: false },
            Deployment { environment: "review/feat-checkout".into(), url: "https://feat-checkout.review.acme.test".into(), current: true },
        ];
        app.apply(Incoming::Deployments { key: mr_key(), deployments });
        let screen = render(&mut app, 200, 30);
        assert!(screen.contains("⧉ review/feat-checkout +1"), "the review app comes first in the header:\n{screen}");
        assert!(screen.contains("REVIEW APPS") && screen.contains("https://sb.acme.test/feat-checkout"), "{screen}");
        assert!(screen.contains("⧉ storybook/feat-checkout · older push"), "{screen}");
        assert!(app.links.iter().any(|l| l.text == "⧉ review/feat-checkout" && l.url == "https://feat-checkout.review.acme.test"));
        press(&mut app, "p");
        press(&mut app, "i");
        let cover = render(&mut app, 200, 50);
        assert!(cover.contains("REVIEW APPS") && cover.contains("https://feat-checkout.review.acme.test"), "{cover}");
        assert!(!cover.contains("https://sb.acme.test"), "the cover shows the one to try");
    }

    #[test]
    fn a_click_on_a_review_app_opens_it_from_the_pipeline_and_the_cover() {
        use crate::forge::Deployment;
        use crossterm::event::{MouseButton, MouseEventKind};
        let mut app = with_pipeline();
        let url = "https://feat-checkout.review.acme.test";
        let deployments = vec![Deployment { environment: "review/feat-checkout".into(), url: url.into(), current: true }];
        app.apply(Incoming::Deployments { key: mr_key(), deployments });
        let click = |app: &mut App, at: (u16, u16)| {
            mouse(app, MouseEventKind::Down(MouseButton::Left), at);
            mouse(app, MouseEventKind::Up(MouseButton::Left), at)
        };
        for (keys, height) in [("", 30), ("pi", 50)] {
            press(&mut app, keys);
            render(&mut app, 200, height);
            let link = app.links.iter().find(|l| l.text == url).unwrap_or_else(|| panic!("after {keys:?}: {:?}", app.links)).clone();
            assert_eq!(click(&mut app, (link.x + 2, link.y)), vec![Action::OpenUrl(url.into())], "after {keys:?}");
        }
    }

    #[test]
    fn p_opens_the_pipeline_on_its_first_failure_and_o_opens_that_job() {
        let mut app = with_pipeline();
        assert_eq!(app.focus, Focus::Side);
        let pipeline = app.open.as_ref().unwrap().pipeline.clone().unwrap();
        assert_eq!(pipeline.jobs()[pipeline.selected].name, "flaky", "the cursor lands on the failure");
        assert_eq!(press(&mut app, "o"), vec![Action::OpenUrl("https://ci.example/flaky".into())]);
        press(&mut app, "j");
        assert_eq!(press(&mut app, "y"), vec![Action::Yank("https://ci.example/unit".into())]);
        press(&mut app, "p");
        assert!(app.open.as_ref().unwrap().pipeline.is_none());
        assert_eq!(app.focus, Focus::Review);
    }

    #[test]
    fn a_run_still_going_is_asked_again_while_the_pane_shows_it() {
        use crate::forge::checks::JobState;
        let mut app = with_review();
        press(&mut app, "p");
        let mut going = run();
        going.stages[1].jobs[0].state = JobState::Running;
        app.apply(Incoming::Checks { key: mr_key(), checks: Some(going) });
        assert!(app.tick().iter().all(|a| !matches!(a, Action::LoadChecks { .. })), "not before its time");
        app.now += Duration::from_secs(16);
        assert!(app.tick().iter().any(|a| matches!(a, Action::LoadChecks { .. })));
        assert!(app.tick().iter().all(|a| !matches!(a, Action::LoadChecks { .. })), "asked once");
        app.apply(Incoming::Checks { key: mr_key(), checks: Some(run()) });
        app.now += Duration::from_secs(60);
        assert!(app.tick().iter().all(|a| !matches!(a, Action::LoadChecks { .. })), "a finished run is left alone");
    }

    #[test]
    fn r_asks_again_and_a_failure_or_no_run_says_so_in_the_pane() {
        let mut app = with_pipeline();
        assert!(matches!(press(&mut app, "r").as_slice(), [Action::LoadChecks { .. }]));
        app.apply(Incoming::Failed { what: Failure::Checks, message: "HTTP 500".into() });
        assert_eq!(app.open.as_ref().unwrap().pipeline.as_ref().unwrap().run, Run::Failed("HTTP 500".into()));
        app.apply(Incoming::Checks { key: mr_key(), checks: None });
        assert_eq!(app.open.as_ref().unwrap().pipeline.as_ref().unwrap().run, Run::Nothing);
    }

    #[test]
    fn the_pipeline_and_the_file_tree_share_the_right_pane() {
        let mut app = with_pipeline();
        press(&mut app, "h");
        press(&mut app, "t");
        let open = app.open.as_ref().unwrap();
        assert!(open.tree.is_some() && open.pipeline.is_none());
        app.handle_key(code(KeyCode::Esc));
        press(&mut app, "p");
        let open = app.open.as_ref().unwrap();
        assert!(open.pipeline.is_some() && open.tree.is_none());
    }
}
