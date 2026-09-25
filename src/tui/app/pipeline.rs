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
    fn with_pipeline(&self, pipeline: Option<Pipeline>) -> Self {
        Self { pipeline, ..self.clone() }
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
            self.open = Some(open.with_pipeline(None));
            self.focus = Focus::Review;
            return vec![];
        }
        let action = load(open);
        self.open = Some(Open { pane: None, tree: None, answer: None, ..open.with_pipeline(Some(Pipeline::waiting())) });
        self.focus = Focus::Side;
        vec![action]
    }

    pub(super) fn handle_pipeline_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let Some(open) = self.open.clone() else { return vec![] };
        let Some(pipeline) = open.pipeline.clone() else { return vec![] };
        let jobs = pipeline.jobs();
        let last = jobs.len().saturating_sub(1);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let moved = |selected: usize| Some(Pipeline { selected: selected.min(last), ..pipeline.clone() });
        let url = jobs.get(pipeline.selected).map(|j| j.web_url.clone()).or_else(|| match &pipeline.run {
            Run::Ready(checks) => checks.web_url.clone(),
            _ => open.review.mr.pipeline.as_ref().and_then(|p| p.web_url.clone()),
        });
        let next = match key.code {
            KeyCode::Char('j') | KeyCode::Down => moved(pipeline.selected + 1),
            KeyCode::Char('k') | KeyCode::Up => moved(pipeline.selected.saturating_sub(1)),
            KeyCode::Char('d') if ctrl => moved(pipeline.selected + 10),
            KeyCode::Char('u') if ctrl => moved(pipeline.selected.saturating_sub(10)),
            KeyCode::Char('g') => moved(0),
            KeyCode::Char('G') => moved(last),
            KeyCode::Char('o') => return url.map(|u| vec![Action::OpenUrl(u)]).unwrap_or_default(),
            KeyCode::Char('y') => return url.map(|u| vec![Action::Yank(u)]).unwrap_or_default(),
            KeyCode::Char('r') => {
                self.open = Some(open.with_pipeline(Some(Pipeline { due: None, ..pipeline })));
                return vec![load(&open)];
            }
            KeyCode::Esc | KeyCode::Char('p' | 'x') => None,
            _ => return vec![],
        };
        if next.is_none() {
            self.focus = Focus::Review;
        }
        self.open = Some(open.with_pipeline(next));
        vec![]
    }

    /// The open MR's review apps asked of the forge; what was known stays shown until the answer.
    pub(super) fn ask_deployments(&mut self) {
        let Some(open) = self.open.clone() else { return };
        let mr = &open.review.mr;
        self.composed.push(Action::LoadDeployments { key: open.key.clone(), branch: mr.source_branch.clone(), head: mr.refs.head.clone() });
        self.open = Some(Open { deployments: Some(open.deployments.unwrap_or_default()), ..open });
    }

    /// The run arrived; a run still going is asked again in a while, as long as the pane shows it.
    pub(super) fn apply_checks(&mut self, key: &super::MrKey, checks: Option<Checks>) {
        let Some(open) = self.open.clone().filter(|o| &o.key == key) else { return };
        let Some(pipeline) = open.pipeline.clone() else { return };
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
        self.open = Some(open.with_pipeline(Some(Pipeline { run, selected, due })));
        if finished {
            self.ask_deployments();
        }
    }

    pub(super) fn checks_failed(&mut self, message: String) {
        let Some(open) = self.open.clone() else { return };
        if open.pipeline.is_some() {
            self.open = Some(open.with_pipeline(Some(Pipeline { run: Run::Failed(message), selected: 0, due: None })));
        }
    }

    /// The pane's run, asked again once its refresh is due.
    pub(super) fn pipeline_tick(&mut self) -> Option<Action> {
        let open = self.open.clone()?;
        let pipeline = open.pipeline.clone()?;
        if pipeline.due.is_none_or(|due| self.now < due) {
            return None;
        }
        self.open = Some(open.with_pipeline(Some(Pipeline { due: None, ..pipeline })));
        Some(load(&open))
    }
}

fn load(open: &Open) -> Action {
    Action::LoadChecks { key: open.key.clone(), head: open.review.mr.refs.head.clone() }
}
