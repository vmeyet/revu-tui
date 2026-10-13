//! `a g` and `:session`: the coding-agent session that belongs to an MR, found or started, then shown.
//! Claude Code is the built-in agent (`claude.rs`); `[agent] start` names another one (`custom.rs`).
//! Every program runs through [`Run`], so a test never starts a session, a tmux window or a git command.
mod claude;
mod custom;
mod show;
mod worktree;

use crate::cache::{Cache, keys};
use crate::config;
use crate::forge::MrKey;
use crate::share::Fields;
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

pub use custom::check;

/// A session name longer than this is cut: it is a tmux window's name too.
const NAME_CHARS: usize = 60;

/// What a session is told about its MR.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mr {
    pub key: MrKey,
    pub fields: Fields,
    pub target: String,
    /// I wrote it: the session may change it, where someone else's only gets a plan.
    pub mine: bool,
}

impl Mr {
    /// `revu: !42 charge cards at checkout`, as the reader names sessions: a scope, then a title.
    fn session_name(&self) -> String {
        let name = format!("revu: {} {}", self.fields.reference, self.fields.title);
        if name.chars().count() <= NAME_CHARS {
            return name;
        }
        let kept: String = name.chars().take(NAME_CHARS - 1).collect();
        format!("{}…", kept.trim_end())
    }
}

/// One program to run: its words, and the folder it runs in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    pub argv: Vec<String>,
    pub dir: Option<PathBuf>,
}

impl Call {
    fn new<S: AsRef<str>>(argv: &[S]) -> Self {
        Self { argv: argv.iter().map(|word| word.as_ref().to_owned()).collect(), dir: None }
    }

    fn in_dir(self, dir: &Path) -> Self {
        Self { dir: Some(dir.to_owned()), ..self }
    }

    fn program(&self) -> &str {
        self.argv.first().map_or("", String::as_str)
    }

    fn command(&self) -> Command {
        let mut command = Command::new(self.program());
        command.args(self.argv.iter().skip(1));
        if let Some(dir) = &self.dir {
            command.current_dir(dir);
        }
        command
    }
}

/// Where programs run: on this machine, or in a test's script.
pub trait Run: Send + Sync {
    /// What the program printed; a failure names the program and gives its first line.
    fn output(&self, call: &Call) -> Result<String>;
    /// The program on the terminal as it is, until it exits; the error is a line for the reader.
    fn foreground(&self, call: &Call) -> Result<(), String>;
}

/// The programs of this machine.
pub struct Programs;

impl Run for Programs {
    fn output(&self, call: &Call) -> Result<String> {
        let program = call.program();
        let output = call.command().stdin(Stdio::null()).output().with_context(|| format!("cannot run `{program}`"))?;
        let printed = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        if !output.status.success() {
            let why = printed.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("no message");
            bail!("`{program} {}` failed: {why}", call.argv.get(1).map_or("", String::as_str));
        }
        Ok(printed)
    }

    fn foreground(&self, call: &Call) -> Result<(), String> {
        let program = call.program();
        let status = call.command().status().map_err(|e| format!("cannot run {program}: {e}"))?;
        if status.success() { Ok(()) } else { Err(format!("{program} ended with {status}")) }
    }
}

/// A session the agent knows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    /// What opens it.
    pub id: String,
    /// What starts it again once it stopped.
    pub resume_id: String,
    pub name: String,
    /// The folder it was started in.
    pub dir: PathBuf,
    /// Running or waiting, so it opens as it is; a stopped one is started again first.
    pub live: bool,
    pub started: Option<DateTime<Utc>>,
}

/// The session revu last opened for an MR, kept in the cache so a restart finds it again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Kept {
    pub name: String,
    pub dir: PathBuf,
    /// The agent's own id for it, when it gives one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// Which session `a g` is after.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    /// The one that belongs to the MR, wherever it is found.
    Find,
    /// The one the reader picked in the list.
    Session(Box<Session>),
    New,
}

/// How the session was found, for the toast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case {
    Remembered,
    Opener,
    Picked,
    New,
}

impl Case {
    fn said(self, reference: &str) -> String {
        match self {
            Case::Remembered => format!("the session revu remembered for {reference}"),
            Case::Opener => format!("the session that opened {reference}"),
            Case::Picked => format!("the session picked for {reference}"),
            Case::New => format!("a new session for {reference}"),
        }
    }
}

/// A session ready to show: what to remember, and the program that shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Ready {
    kept: Kept,
    call: Call,
    case: Case,
}

enum Step {
    Pick(Vec<Session>),
    Ready(Ready),
}

/// The session's program, waiting for the terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Handoff {
    pub call: Call,
    /// Which session this is and how it was found, for the toast on the way back.
    pub said: String,
}

/// What `a g` led to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Several sessions could be the one: the reader picks, or starts a new one.
    Pick(Vec<Session>),
    /// The session is on show in a tmux window; the text says which.
    Window(String),
    /// Outside tmux: the loop lends the terminal to the session.
    Handoff(Handoff),
}

/// Which agent runs the sessions.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Harness {
    /// Claude Code; `jobs` is where it keeps one folder per background session.
    Claude { jobs: PathBuf },
    /// The commands of `[agent] start` and `open`.
    Custom { start: String, open: Option<String> },
}

/// What revu learns from the machine at start.
pub struct Machine {
    pub home: PathBuf,
    /// Claude Code's own folder, `~/.claude` unless `CLAUDE_CONFIG_DIR` moves it.
    pub claude_home: PathBuf,
    /// revu runs inside tmux.
    pub tmux: bool,
    /// The checkout revu was started in: its project and its root, which may be a worktree.
    pub started_in: Option<(String, PathBuf)>,
}

/// `[agent]` switched on: everything `a g` needs to find, start and show a session.
#[derive(Clone)]
pub struct Agent {
    harness: Harness,
    /// Project path → its checkout; the one revu was started in wins over the config's.
    checkouts: BTreeMap<String, PathBuf>,
    tmux: bool,
    pub run: Arc<dyn Run>,
}

impl Agent {
    pub fn new(config: &config::Agent, machine: Machine, run: Arc<dyn Run>) -> Self {
        let harness = match &config.start {
            Some(start) => Harness::Custom { start: start.clone(), open: config.open.clone() },
            None => Harness::Claude { jobs: machine.claude_home.join("jobs") },
        };
        let started_in = machine.started_in.map(|(project, folder)| (project, worktree::main_checkout(run.as_ref(), &folder)));
        let checkouts = config.folders(&machine.home).into_iter().chain(started_in).collect();
        Self { harness, checkouts, tmux: machine.tmux, run }
    }

    /// Finds or starts the session of `mr`, remembers it in `cache`, and shows it in a tmux window
    /// when revu runs in tmux. Outside tmux the caller owns the terminal, so the program comes back.
    pub fn open(&self, mr: &Mr, cache: &Cache, choice: Choice) -> Result<Outcome> {
        let key = keys::session(&mr.key);
        let kept = cache.read::<Kept>(&key);
        let step = match &self.harness {
            Harness::Claude { jobs } => claude::session(self, jobs, mr, kept.as_ref(), choice)?,
            Harness::Custom { start, open } => custom::session(self, start, open.as_deref(), mr, kept.as_ref())?,
        };
        let ready = match step {
            Step::Pick(sessions) => return Ok(Outcome::Pick(sessions)),
            Step::Ready(ready) => ready,
        };
        cache.write(&key, &ready.kept)?;
        show::show(self.run.as_ref(), self.tmux, &ready, &mr.fields.reference)
    }

    fn checkout(&self, project: &str) -> Result<&Path> {
        let Some(folder) = self.checkouts.get(project) else {
            bail!("no checkout of {project} is known: add `\"{project}\" = \"/its/folder\"` under `[agent.checkouts]` in the config");
        };
        Ok(folder)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use std::sync::Mutex;

    /// Programs that answer from a list instead of running: the first entry whose words start the call's.
    #[derive(Default)]
    pub(super) struct Script {
        answers: Vec<(String, Result<String, String>)>,
        calls: Mutex<Vec<Call>>,
    }

    impl Script {
        pub fn says(self, starts_with: &str, printed: &str) -> Self {
            self.then(starts_with, Ok(printed.to_owned()))
        }

        pub fn fails(self, starts_with: &str, why: &str) -> Self {
            self.then(starts_with, Err(why.to_owned()))
        }

        fn then(mut self, starts_with: &str, answer: Result<String, String>) -> Self {
            self.answers.push((starts_with.to_owned(), answer));
            self
        }

        pub fn calls(&self) -> Vec<Call> {
            self.calls.lock().unwrap().clone()
        }

        /// Every call as one line, for a test that reads the order.
        pub fn lines(&self) -> Vec<String> {
            self.calls().iter().map(|call| call.argv.join(" ")).collect()
        }
    }

    impl Run for Script {
        fn output(&self, call: &Call) -> Result<String> {
            self.calls.lock().unwrap().push(call.clone());
            let line = call.argv.join(" ");
            match self.answers.iter().find(|(starts_with, _)| line.starts_with(starts_with)) {
                Some((_, Ok(printed))) => Ok(printed.clone()),
                Some((_, Err(why))) => Err(anyhow::anyhow!(why.clone())),
                None => Ok(String::new()),
            }
        }

        fn foreground(&self, call: &Call) -> Result<(), String> {
            self.calls.lock().unwrap().push(call.clone());
            Ok(())
        }
    }

    pub(super) fn mr() -> Mr {
        Mr {
            key: MrKey::new("acme/widgets", 42),
            fields: Fields {
                reference: "!42".into(),
                number: 42,
                title: "charge cards at checkout".into(),
                url: "https://gitlab.com/acme/widgets/-/merge_requests/42".into(),
                author: "omar".into(),
                project: "acme/widgets".into(),
                branch: "feat/checkout".into(),
            },
            target: "main".into(),
            mine: false,
        }
    }

    pub(super) const REPO: &str = "/code/widgets";

    pub(super) fn claude(script: Script, jobs: &Path, tmux: bool) -> (Agent, Arc<Script>) {
        let harness = Harness::Claude { jobs: jobs.to_owned() };
        agent(harness, script, tmux)
    }

    pub(super) fn agent(harness: Harness, script: Script, tmux: bool) -> (Agent, Arc<Script>) {
        let script = Arc::new(script);
        let checkouts = BTreeMap::from([("acme/widgets".to_owned(), PathBuf::from(REPO))]);
        (Agent { harness, checkouts, tmux, run: script.clone() }, script)
    }

    pub(super) fn cache() -> (tempfile::TempDir, Cache) {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::in_dir(dir.path());
        (dir, cache)
    }

    #[test]
    fn a_session_is_named_after_the_mr_and_a_long_title_is_cut() {
        assert_eq!(mr().session_name(), "revu: !42 charge cards at checkout");
        let long = Mr { fields: Fields { title: "é".repeat(80), ..mr().fields }, ..mr() };
        let name = long.session_name();
        assert_eq!(name.chars().count(), NAME_CHARS);
        assert!(name.starts_with("revu: !42 é") && name.ends_with('…'), "{name}");
    }

    #[test]
    fn the_checkout_revu_was_started_in_wins_over_the_config_and_the_agent_follows_start() {
        let config = config::Agent {
            enabled: true,
            checkouts: BTreeMap::from([("acme/widgets".into(), "~/old/widgets".into()), ("acme/api".into(), "~/code/api".into())]),
            ..config::Agent::default()
        };
        let machine = || Machine {
            home: "/Users/nina".into(),
            claude_home: "/Users/nina/.claude".into(),
            tmux: true,
            started_in: Some(("acme/widgets".into(), REPO.into())),
        };
        let agent = Agent::new(&config, machine(), Arc::new(Script::default()));
        assert_eq!(agent.checkout("acme/widgets").unwrap(), Path::new(REPO));
        assert_eq!(agent.checkout("acme/api").unwrap(), Path::new("/Users/nina/code/api"));
        assert_eq!(agent.harness, Harness::Claude { jobs: "/Users/nina/.claude/jobs".into() });
        assert!(agent.tmux);
        let missing = agent.checkout("acme/other").unwrap_err().to_string();
        assert!(missing.contains("[agent.checkouts]") && missing.contains("acme/other"), "{missing}");
        let in_a_worktree =
            Machine { started_in: Some(("acme/widgets".into(), "/code/widgets/.claude/worktrees/fix".into())), ..machine() };
        let listed = Script::default().says("git worktree list", "worktree /code/widgets\nHEAD aaaa\nbranch refs/heads/main\n");
        let agent = Agent::new(&config, in_a_worktree, Arc::new(listed));
        assert_eq!(agent.checkout("acme/widgets").unwrap(), Path::new(REPO), "started in a worktree, the checkout is the main one");
        let custom = config::Agent { start: Some("aider".into()), open: Some("aider --restore".into()), ..config };
        let agent = Agent::new(&custom, machine(), Arc::new(Script::default()));
        assert_eq!(agent.harness, Harness::Custom { start: "aider".into(), open: Some("aider --restore".into()) });
    }

    #[test]
    fn the_real_runner_returns_what_a_program_printed_and_names_the_one_that_failed() {
        let dir = tempfile::tempdir().unwrap();
        let printed = Programs.output(&Call::new(&["pwd"]).in_dir(dir.path())).unwrap();
        assert!(printed.trim().ends_with(dir.path().file_name().unwrap().to_str().unwrap()), "{printed}");
        let failed = Programs.output(&Call::new(&["ls", "/revu-no-such-folder"])).unwrap_err().to_string();
        assert!(failed.starts_with("`ls /revu-no-such-folder` failed:"), "{failed}");
        assert!(Programs.output(&Call::new(&["revu-no-such-program"])).unwrap_err().to_string().contains("revu-no-such-program"));
        assert_eq!(Programs.foreground(&Call::new(&["true"])), Ok(()));
        assert!(Programs.foreground(&Call::new(&["false"])).unwrap_err().starts_with("false ended with"));
    }
}
