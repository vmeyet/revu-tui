//! The CI run of an MR's head commit, whichever forge ran it: jobs grouped by stage, in the order
//! the stages ran. GitLab calls it a pipeline, GitHub check runs grouped by workflow.
use serde::{Deserialize, Serialize};

/// Every job of the run, and where the forge shows the whole of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checks {
    pub web_url: Option<String>,
    pub stages: Vec<Stage>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stage {
    pub name: String,
    pub jobs: Vec<Job>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub name: String,
    pub state: JobState,
    /// How long it ran, once it started.
    pub seconds: Option<u64>,
    pub web_url: String,
    /// A failure the forge does not count against the run.
    pub allowed_to_fail: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum JobState {
    Failed,
    Running,
    Pending,
    Manual,
    Canceled,
    Skipped,
    Passed,
}

/// One job as a backend found it, with the stage it belongs to and when it started, so stages
/// line up in the order they ran whatever order the forge listed the jobs in.
pub struct Found {
    pub stage: String,
    pub order: String,
    pub job: Job,
}

impl Checks {
    /// Jobs grouped by stage; stages ordered by their earliest job, failed jobs first inside each.
    pub fn from_jobs(web_url: Option<String>, found: Vec<Found>) -> Self {
        let mut found = found;
        found.sort_by(|a, b| a.order.cmp(&b.order));
        let mut stages: Vec<Stage> = vec![];
        for Found { stage, job, .. } in found {
            match stages.iter_mut().find(|s| s.name == stage) {
                Some(existing) => existing.jobs.push(job),
                None => stages.push(Stage { name: stage, jobs: vec![job] }),
            }
        }
        for stage in &mut stages {
            stage.jobs.sort_by_key(|job| job.state != JobState::Failed || job.allowed_to_fail);
        }
        Self { web_url, stages }
    }

    pub fn jobs(&self) -> impl Iterator<Item = &Job> {
        self.stages.iter().flat_map(|s| s.jobs.iter())
    }

    /// How the run stands as a whole: a counted failure wins, then anything still to finish.
    pub fn state(&self) -> JobState {
        let counted = || self.jobs().filter(|j| !(j.allowed_to_fail && j.state == JobState::Failed));
        if counted().any(|j| j.state == JobState::Failed) {
            JobState::Failed
        } else if counted().any(|j| matches!(j.state, JobState::Running | JobState::Pending)) {
            JobState::Running
        } else if counted().any(|j| j.state == JobState::Canceled) {
            JobState::Canceled
        } else {
            JobState::Passed
        }
    }

    pub fn count(&self, state: JobState) -> usize {
        self.jobs().filter(|j| j.state == state).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(stage: &str, order: &str, name: &str, state: JobState) -> Found {
        Found {
            stage: stage.into(),
            order: order.into(),
            job: Job { name: name.into(), state, seconds: None, web_url: String::new(), allowed_to_fail: false },
        }
    }

    #[test]
    fn stages_follow_the_order_they_ran_and_failures_come_first() {
        let checks = Checks::from_jobs(
            None,
            vec![
                found("test", "2026-09-23T11:10:30Z", "unit", JobState::Passed),
                found("test", "2026-09-23T11:10:31Z", "flaky", JobState::Failed),
                found("check", "2026-09-23T11:10:22Z", "lint", JobState::Passed),
            ],
        );
        let shape: Vec<(&str, Vec<&str>)> =
            checks.stages.iter().map(|s| (s.name.as_str(), s.jobs.iter().map(|j| j.name.as_str()).collect())).collect();
        assert_eq!(shape, vec![("check", vec!["lint"]), ("test", vec!["flaky", "unit"])]);
        assert_eq!(checks.state(), JobState::Failed);
        assert_eq!((checks.count(JobState::Passed), checks.count(JobState::Failed)), (2, 1));
    }

    #[test]
    fn the_run_state_ignores_failures_allowed_to_fail_and_waits_on_running_jobs() {
        let mut allowed = found("test", "b", "optional", JobState::Failed);
        allowed.job.allowed_to_fail = true;
        let passed = Checks::from_jobs(None, vec![found("test", "a", "unit", JobState::Passed), allowed]);
        assert_eq!(passed.state(), JobState::Passed);
        assert_eq!(passed.stages[0].jobs[0].name, "unit", "an allowed failure is not put first");
        let running = Checks::from_jobs(None, vec![found("test", "a", "unit", JobState::Running)]);
        assert_eq!(running.state(), JobState::Running);
        assert_eq!(Checks::from_jobs(None, vec![]).state(), JobState::Passed);
    }
}
