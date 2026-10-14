//! `a g` and `:session`: the coding-agent session that belongs to an MR, found or started, then shown.
//! Claude Code is the built-in agent (`claude.rs`); `[agent] start` names another one (`custom.rs`).
//! Every program runs through [`Run`], so a test never starts a session, a tmux window or a git command.
mod claude;
mod custom;
mod place;
mod show;
mod terminal;
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
pub use place::check_split;
pub use terminal::Terminal;

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
    /// revu's `PATH`, for a program a terminal starts with an environment of its own.
    pub path: Option<String>,
}

impl Call {
    fn new<S: AsRef<str>>(argv: &[S]) -> Self {
        Self { argv: argv.iter().map(|word| word.as_ref().to_owned()).collect(), dir: None, path: None }
    }

    /// The words that run the program as it is meant to run: behind `env PATH=…` when it carries a `PATH`.
    fn words(&self) -> Vec<String> {
        let with_path = self.path.iter().flat_map(|path| ["/usr/bin/env".to_owned(), format!("PATH={path}")]);
        with_path.chain(self.argv.iter().cloned()).collect()
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
        if let Some(path) = &self.path {
            command.env("PATH", path);
        }
        command
    }
}

/// Where programs run: on this machine, or in a test's script.
pub trait Run: Send + Sync {
    /// What the program printed on its output; a failure names the program and gives the first line it complained with.
    fn output(&self, call: &Call) -> Result<String>;
    /// The program on the terminal as it is, until it exits; the error is a line for the reader.
    fn foreground(&self, call: &Call) -> Result<(), String>;
    /// How many columns revu's own terminal has, when it says.
    fn width(&self) -> Option<u16>;
    /// revu's own `PATH`, which a pane opened by a terminal does not get by itself.
    fn search_path(&self) -> Option<String>;
    /// The full path of `program` as revu's own `PATH` finds it.
    fn locate(&self, program: &str) -> Option<String>;
}

/// The first folder of `search_path` that holds something `is_runnable` under the name `program`;
/// a program already written as a path is that path.
fn find_in(search_path: &str, program: &str, is_runnable: impl Fn(&Path) -> bool) -> Option<String> {
    if program.contains('/') {
        return Some(program.to_owned());
    }
    let found = std::env::split_paths(search_path).map(|folder| folder.join(program)).find(|file| is_runnable(file))?;
    Some(found.to_string_lossy().into_owned())
}

fn is_runnable(file: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    file.metadata().is_ok_and(|about| about.is_file() && about.permissions().mode() & 0o111 != 0)
}

/// The line of what a failed program wrote that carries the reason: git's `fatal:`, `error:` or
/// `!` line when there is one, else the last line, since the first is often only a heading.
fn reason<'a>(complained: &'a str, printed: &'a str) -> &'a str {
    let lines: Vec<&str> = complained.lines().chain(printed.lines()).map(str::trim).filter(|line| !line.is_empty()).collect();
    let marked = lines.iter().find(|line| ["fatal:", "error:", "!"].iter().any(|mark| line.starts_with(mark)));
    marked.or(lines.last()).copied().unwrap_or("no message")
}

/// The programs of this machine.
pub struct Programs;

impl Run for Programs {
    fn output(&self, call: &Call) -> Result<String> {
        let program = call.program();
        let output = call.command().stdin(Stdio::null()).output().with_context(|| format!("cannot run `{program}`"))?;
        let (printed, complained) = (String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        if !output.status.success() {
            bail!("`{program} {}` failed: {}", call.argv.get(1).map_or("", String::as_str), reason(&complained, &printed));
        }
        Ok(printed.into_owned())
    }

    fn foreground(&self, call: &Call) -> Result<(), String> {
        let program = call.program();
        let status = call.command().status().map_err(|e| format!("cannot run {program}: {e}"))?;
        if status.success() { Ok(()) } else { Err(format!("{program} ended with {status}")) }
    }

    fn width(&self) -> Option<u16> {
        crossterm::terminal::size().ok().map(|(columns, _)| columns)
    }

    fn search_path(&self) -> Option<String> {
        std::env::var("PATH").ok().filter(|path| !path.is_empty())
    }

    fn locate(&self, program: &str) -> Option<String> {
        find_in(&self.search_path()?, program, is_runnable)
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
    /// The pane of the terminal revu showed it in, to go back to while the terminal still has it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
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
    /// Something the reader should know about the folder it works in, for the toast.
    notice: Option<String>,
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
    /// The session is on show beside revu or in a tmux window; the text says which.
    Shown(String),
    /// Nothing could split: the loop lends the terminal to the session.
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
    /// The terminal revu runs in, when it is one revu can split.
    pub terminal: Option<Terminal>,
    /// Under this many columns revu's own window is compact: a pane beside it must leave it that much.
    pub compact_width: u16,
    /// The checkout revu was started in: its project and its root, which may be a worktree.
    pub started_in: Option<(String, PathBuf)>,
}

/// `[agent]` switched on: everything `a g` needs to find, start and show a session.
#[derive(Clone)]
pub struct Agent {
    harness: Harness,
    /// Project path → its checkout; the one revu was started in wins over the config's.
    checkouts: BTreeMap<String, PathBuf>,
    /// What can show a session beside revu, and when it should.
    place: place::Place,
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
        let screen = match (machine.tmux, machine.terminal, &config.split) {
            (true, _, _) => place::Screen::Tmux,
            (false, Some(terminal), _) => place::Screen::Terminal(terminal),
            (false, None, Some(command)) => place::Screen::Command(command.clone()),
            (false, None, None) => place::Screen::Plain,
        };
        let place = place::Place { screen, show: config.show, compact_width: machine.compact_width };
        Self { harness, checkouts, place, run }
    }

    /// Finds or starts the session of `mr`, remembers it in `cache`, and shows it beside revu where
    /// something can split. Where nothing can, the caller owns the terminal, so the program comes back.
    pub fn open(&self, mr: &Mr, cache: &Cache, choice: Choice) -> Result<Outcome> {
        let run = self.run.as_ref();
        let key = keys::session(&mr.key);
        let before = cache.read::<Kept>(&key);
        let origin = self.place.origin(run);
        let step = match &self.harness {
            Harness::Claude { jobs } => claude::session(self, jobs, mr, before.as_ref(), choice)?,
            Harness::Custom { start, open } => custom::session(self, start, open.as_deref(), mr, before.as_ref())?,
        };
        let ready = match step {
            Step::Pick(sessions) => return Ok(Outcome::Pick(sessions)),
            Step::Ready(ready) => ready,
        };
        let is_same = |kept: &Kept| kept.id == ready.kept.id && kept.name == ready.kept.name;
        let shown_in = before.filter(is_same).and_then(|kept| kept.pane);
        cache.write(&key, &Kept { pane: shown_in.clone(), ..ready.kept.clone() })?;
        let placed = self.place.put(run, origin.as_deref(), &ready, shown_in.as_deref(), &mr.fields.reference)?;
        if placed.pane != shown_in {
            cache.write(&key, &Kept { pane: placed.pane, ..ready.kept })?;
        }
        Ok(placed.outcome)
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
        width: Option<u16>,
        search_path: Option<String>,
        missing: Vec<String>,
    }

    impl Script {
        pub fn says(self, starts_with: &str, printed: &str) -> Self {
            self.then(starts_with, Ok(printed.to_owned()))
        }

        /// revu's terminal is `width` columns wide.
        pub fn wide(self, width: u16) -> Self {
            Self { width: Some(width), ..self }
        }

        /// revu's `PATH` is `search_path`, and every program but the missing ones is in its first folder.
        pub fn with_path(self, search_path: &str) -> Self {
            Self { search_path: Some(search_path.to_owned()), ..self }
        }

        pub fn without(mut self, program: &str) -> Self {
            self.missing.push(program.to_owned());
            self
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

        fn width(&self) -> Option<u16> {
            self.width
        }

        fn search_path(&self) -> Option<String> {
            self.search_path.clone()
        }

        /// With no `PATH` set by the test, a program is found under its own name.
        fn locate(&self, program: &str) -> Option<String> {
            if self.missing.iter().any(|missing| missing == program) {
                return None;
            }
            let Some(search_path) = &self.search_path else { return Some(program.to_owned()) };
            find_in(search_path, program, |_| true)
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
        let screen = if tmux { place::Screen::Tmux } else { place::Screen::Plain };
        on_screen(harness, script, screen)
    }

    pub(super) fn on_screen(harness: Harness, script: Script, screen: place::Screen) -> (Agent, Arc<Script>) {
        let script = Arc::new(script);
        let checkouts = BTreeMap::from([("acme/widgets".to_owned(), PathBuf::from(REPO))]);
        let place = place::Place { screen, show: config::AgentShow::Auto, compact_width: 70 };
        (Agent { harness, checkouts, place, run: script.clone() }, script)
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
            terminal: Some(Terminal::Ghostty),
            compact_width: 60,
            started_in: Some(("acme/widgets".into(), REPO.into())),
        };
        let agent = Agent::new(&config, machine(), Arc::new(Script::default()));
        assert_eq!(agent.checkout("acme/widgets").unwrap(), Path::new(REPO));
        assert_eq!(agent.checkout("acme/api").unwrap(), Path::new("/Users/nina/code/api"));
        assert_eq!(agent.harness, Harness::Claude { jobs: "/Users/nina/.claude/jobs".into() });
        let placed = |screen| place::Place { screen, show: config::AgentShow::Auto, compact_width: 60 };
        assert_eq!(agent.place, placed(place::Screen::Tmux), "tmux first, whatever the terminal around it");
        let with_split = config::Agent { split: Some("kitty @ launch {command}".into()), ..config.clone() };
        let screen_of = |machine: Machine| Agent::new(&with_split, machine, Arc::new(Script::default())).place.screen;
        assert_eq!(
            screen_of(Machine { tmux: false, ..machine() }),
            place::Screen::Terminal(Terminal::Ghostty),
            "then a terminal revu knows"
        );
        let unknown = Machine { tmux: false, terminal: None, ..machine() };
        assert_eq!(screen_of(unknown), place::Screen::Command("kitty @ launch {command}".into()), "then the split command");
        let plain = Agent::new(&config, Machine { tmux: false, terminal: None, ..machine() }, Arc::new(Script::default()));
        assert_eq!(plain.place, placed(place::Screen::Plain), "then the terminal itself");
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
    fn the_pane_a_session_was_shown_in_is_remembered_with_it_and_forgotten_once_gone() {
        let jobs = tempfile::tempdir().unwrap();
        let item = |live: &str| {
            format!(
                r#"[{{"id": "ab12cd34", "sessionId": "ab12cd34-0000", "name": "revu: nxt", "cwd": "/code/widgets", "kind": "background"{live}}}]"#
            )
        };
        let listed = item(r#", "status": "idle""#);
        let ghostty = |script: Script| {
            on_screen(Harness::Claude { jobs: jobs.path().to_owned() }, script.wide(200), place::Screen::Terminal(Terminal::Ghostty))
        };
        let front = "osascript -e tell application";
        let (dir, cache) = cache();
        let kept = || Cache::in_dir(dir.path()).read::<Kept>(&keys::session(&mr().key)).unwrap();
        cache
            .write(
                &keys::session(&mr().key),
                &Kept { name: "revu: nxt".into(), dir: REPO.into(), id: Some("ab12cd34-0000".into()), pane: None },
            )
            .unwrap();

        let (agent, script) = ghostty(Script::default().says("claude agents", &listed).says(front, "A1\n").says("osascript", "B2\n"));
        let shown = agent.open(&mr(), &cache, Choice::Find).unwrap();
        assert_eq!(shown, Outcome::Shown("the session revu remembered for !42 · revu: nxt · in a pane beside revu".into()));
        assert!(script.lines()[0].starts_with(front), "revu's own terminal is asked before anything slow: {:?}", script.lines());
        assert_eq!(kept().pane.as_deref(), Some("B2"));

        let (agent, script) = ghostty(Script::default().says("claude agents", &listed).says(front, "A1\n").says("osascript", "focused\n"));
        let shown = agent.open(&mr(), &cache, Choice::Find).unwrap();
        assert_eq!(shown, Outcome::Shown("the session revu remembered for !42 · revu: nxt · already open beside revu".into()));
        assert_eq!(script.calls().last().unwrap().argv[3..], ["B2"], "the remembered pane gets the keys");
        assert_eq!(kept().pane.as_deref(), Some("B2"));

        let gone = Script::default()
            .says("claude agents", &listed)
            .says(front, "A1\n")
            .says("osascript -e on run argv\n  tell application \"Ghostty\"\n    set found", "gone\n");
        let (agent, _script) = ghostty(gone.says("osascript", "C3\n"));
        agent.open(&mr(), &cache, Choice::Find).unwrap();
        assert_eq!(kept().pane.as_deref(), Some("C3"), "a pane that is gone is forgotten for the new one");

        let (agent, _script) = ghostty(Script::default().says("claude agents", &listed).fails("osascript", "not authorized"));
        let Outcome::Handoff(handoff) = agent.open(&mr(), &cache, Choice::Find).unwrap() else { panic!("no hand-off") };
        assert!(handoff.said.contains("Ghostty did not split"), "{}", handoff.said);
        assert_eq!(kept().pane, None, "a session lent the terminal is in no pane");
    }

    /// A file named `program` in `folder`, runnable or not.
    fn file_in(folder: &Path, program: &str, runnable: bool) {
        use std::os::unix::fs::PermissionsExt;
        let file = folder.join(program);
        std::fs::write(&file, "").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(if runnable { 0o755 } else { 0o644 })).unwrap();
    }

    #[test]
    fn a_program_is_found_in_the_first_folder_of_the_path_that_has_a_runnable_one() {
        let root = tempfile::tempdir().unwrap();
        let folders: Vec<PathBuf> = ["first", "second", "third", "my tools"].iter().map(|name| root.path().join(name)).collect();
        for folder in &folders {
            std::fs::create_dir(folder).unwrap();
        }
        file_in(&folders[0], "agent", false);
        std::fs::create_dir(folders[0].join("folder")).unwrap();
        file_in(&folders[1], "agent", true);
        file_in(&folders[2], "agent", true);
        file_in(&folders[2], "folder", true);
        file_in(&folders[3], "tool", true);
        let search_path = std::env::join_paths(&folders).unwrap().into_string().unwrap();
        let found = |program: &str| find_in(&search_path, program, is_runnable);
        assert_eq!(found("agent"), Some(folders[1].join("agent").to_string_lossy().into_owned()), "a file that cannot run is passed over");
        assert_eq!(found("folder"), Some(folders[2].join("folder").to_string_lossy().into_owned()), "so is a folder of that name");
        assert_eq!(found("tool"), Some(folders[3].join("tool").to_string_lossy().into_owned()), "a folder with a space is read whole");
        assert_eq!(found("missing"), None);
        assert_eq!(find_in("", "agent", is_runnable), None);
        assert_eq!(found("/opt/tools/agent").as_deref(), Some("/opt/tools/agent"), "a path is kept as written, there or not");
        assert_eq!(found("./agent").as_deref(), Some("./agent"));
    }

    #[test]
    fn a_failed_program_gives_the_line_that_carries_its_reason() {
        let fetch = "From gitlab.com:acme/widgets\n ! [rejected] feat/checkout -> origin/feat/checkout  (non-fast-forward)\n";
        assert_eq!(reason(fetch, ""), "! [rejected] feat/checkout -> origin/feat/checkout  (non-fast-forward)", "not the heading");
        assert_eq!(
            reason("From gitlab.com:acme/widgets\nfatal: couldn't find remote ref x\nmore\n", ""),
            "fatal: couldn't find remote ref x"
        );
        assert_eq!(reason("first\nlast\n", ""), "last");
        assert_eq!(reason("", "said on its output\n"), "said on its output");
        assert_eq!(reason("", ""), "no message");
    }

    /// Only `sh` is asked of the machine, found through `PATH` wherever it lives.
    fn sh(script: &str) -> Call {
        Call::new(&["sh", "-c", script])
    }

    #[test]
    fn the_real_runner_returns_what_a_program_printed_and_names_the_one_that_failed() {
        let dir = tempfile::tempdir().unwrap();
        let printed = Programs.output(&sh("pwd").in_dir(dir.path())).unwrap();
        assert!(printed.trim().ends_with(dir.path().file_name().unwrap().to_str().unwrap()), "{printed}");
        let failed = Programs.output(&sh("echo 'a heading' >&2; echo 'fatal: broken' >&2; exit 3")).unwrap_err().to_string();
        assert_eq!(failed, "`sh -c` failed: fatal: broken");
        let warned = Programs.output(&sh("echo '[]'; echo 'a warning' >&2")).unwrap();
        assert_eq!(warned, "[]\n", "a warning beside the answer is not part of it");
        assert!(Programs.output(&Call::new(&["revu-no-such-program"])).unwrap_err().to_string().contains("revu-no-such-program"));
        assert_eq!(Programs.foreground(&sh("exit 0")), Ok(()));
        assert!(Programs.foreground(&sh("exit 3")).unwrap_err().starts_with("sh ended with"));
    }
}
