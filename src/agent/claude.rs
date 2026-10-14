//! Claude Code as the agent. `claude agents --json` lists its sessions; the MR a background session
//! opened is read from that session's `state.json`, a file Claude Code keeps for itself and documents
//! nowhere, so whatever cannot be read there means "not found" and never an error.
use super::{Agent, Call, Case, Choice, Kept, Mr, Ready, Session, Step, worktree};
use anyhow::{Context, Result};
use chrono::DateTime;
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

const PROGRAM: &str = "claude";

/// One item of `claude agents --json`, as far as revu reads it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Listed {
    id: String,
    session_id: String,
    kind: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    cwd: PathBuf,
    /// Only a session with a process has one.
    #[serde(default)]
    status: Option<String>,
    /// Milliseconds since 1970.
    #[serde(default)]
    started_at: Option<i64>,
}

/// The background sessions in what `claude agents --json` printed; an item revu cannot read is left out.
fn sessions(printed: &str) -> Vec<Session> {
    let items: Vec<Value> = serde_json::from_str(printed).unwrap_or_default();
    let listed = items.into_iter().filter_map(|item| serde_json::from_value::<Listed>(item).ok());
    listed.filter(|item| item.kind == "background").map(session_of).collect()
}

fn session_of(item: Listed) -> Session {
    Session {
        name: item.name.unwrap_or_else(|| item.id.clone()),
        id: item.id,
        resume_id: item.session_id,
        dir: item.cwd,
        live: item.status.is_some(),
        started: item.started_at.and_then(DateTime::from_timestamp_millis),
    }
}

pub(super) fn session(agent: &Agent, jobs: &Path, mr: &Mr, kept: Option<&Kept>, choice: Choice) -> Result<Step> {
    let run = agent.run.as_ref();
    let (session, case) = match choice {
        Choice::New => return start(agent, mr).map(Step::Ready),
        Choice::Session(session) => (*session, Case::Picked),
        Choice::Find => {
            let known = sessions(&run.output(&Call::new(&[PROGRAM, "agents", "--json", "--all"]))?);
            match find(&known, kept, || openers(jobs, &mr.fields.url), agent.checkouts.get(&mr.fields.project)) {
                Found::Session(session, case) => (session, case),
                Found::Pick(sessions) => return Ok(Step::Pick(sessions)),
                Found::New => return start(agent, mr).map(Step::Ready),
            }
        }
    };
    if !session.live {
        run.output(&Call::new(&[PROGRAM, "--bg", "--resume", &session.resume_id]).in_dir(&session.dir).slow())?;
    }
    Ok(Step::Ready(ready(session, case)))
}

enum Found {
    Session(Session, Case),
    Pick(Vec<Session>),
    New,
}

/// The session of an MR among the `known` ones: the one revu remembers, then the one that opened the
/// MR, then a pick among those started in the project's checkout, then a new one. `openers` reads
/// every session's state file, so it is asked only once nothing is remembered.
fn find(known: &[Session], kept: Option<&Kept>, openers: impl FnOnce() -> Vec<String>, checkout: Option<&PathBuf>) -> Found {
    let remembered = kept.and_then(|kept| known.iter().find(|session| kept.id.as_deref() == Some(session.resume_id.as_str())));
    if let Some(session) = remembered {
        return Found::Session(session.clone(), Case::Remembered);
    }
    let openers = openers();
    let opened = known.iter().filter(|session| openers.contains(&session.id) || openers.contains(&session.resume_id));
    if let Some(session) = opened.max_by_key(|session| (session.live, session.started)) {
        return Found::Session(session.clone(), Case::Opener);
    }
    let mut here: Vec<Session> =
        known.iter().filter(|session| checkout.is_some_and(|root| session.dir.starts_with(root))).cloned().collect();
    if here.is_empty() {
        return Found::New;
    }
    here.sort_by_key(|session| std::cmp::Reverse((session.live, session.started)));
    Found::Pick(here)
}

/// The ids of the background sessions whose `state.json` lists the page `url` among the links they produced.
fn openers(jobs: &Path, url: &str) -> Vec<String> {
    let Ok(folders) = std::fs::read_dir(jobs) else { return vec![] };
    folders.filter_map(Result::ok).filter_map(|folder| opener(&folder.path(), url)).flatten().collect()
}

/// Both ids of the session kept in the folder `job`, when it opened `url`.
fn opener(job: &Path, url: &str) -> Option<Vec<String>> {
    let state: Value = serde_json::from_slice(&std::fs::read(job.join("state.json")).ok()?).ok()?;
    let mut links = state.get("children")?.as_array()?.iter().filter_map(|child| child.get("href")?.as_str());
    if !links.any(|link| same_page(link, url)) {
        return None;
    }
    let folder = job.file_name().and_then(|name| name.to_str()).map(str::to_owned);
    let session = state.get("sessionId").and_then(Value::as_str).map(str::to_owned);
    Some(folder.into_iter().chain(session).collect())
}

/// Two links to one MR: the scheme, a trailing slash, GitLab's `/-/`, a query and letter case do not count.
fn same_page(a: &str, b: &str) -> bool {
    page(a) == page(b)
}

fn page(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let path = rest.split(['?', '#']).next().unwrap_or(rest);
    path.trim_end_matches('/').replace("/-/", "/").to_ascii_lowercase()
}

/// A new background session in a worktree on the MR's branch: a plan only on someone else's MR.
fn start(agent: &Agent, mr: &Mr) -> Result<Ready> {
    let run = agent.run.as_ref();
    let folder = worktree::on_branch(run, agent.checkout(&mr.fields.project)?, &mr.fields.branch)?;
    let name = mr.session_name();
    let printed = run.output(&start_call(mr, &name).in_dir(&folder.dir).slow())?;
    let active = sessions(&run.output(&Call::new(&[PROGRAM, "agents", "--json"]))?);
    let session =
        started(&active, &printed, &name).with_context(|| format!("claude started a session it does not list: {}", printed.trim()))?;
    Ok(Ready { notice: folder.notice, ..ready(session, Case::New) })
}

fn start_call(mr: &Mr, name: &str) -> Call {
    let mode: &[&str] = if mr.mine { &[] } else { &["--permission-mode", "plan"] };
    let prompt = prompt(mr);
    Call::new(&[&[PROGRAM, "--bg", "--name", name], mode, &[prompt.as_str()]].concat())
}

/// Sets the scene and asks for nothing, so the first turn costs one short answer.
fn prompt(mr: &Mr) -> String {
    let Mr { fields, target, .. } = mr;
    format!(
        "This session belongs to {} \"{}\" by {}: {} (branch {} into {target}). \
         `revu show {number}`, `revu diff {number}` and `revu comment {number} --at <path>:<line> <text>` read it and comment on it. \
         Do nothing yet: wait for my instructions.",
        fields.reference,
        fields.title,
        fields.author,
        fields.url,
        fields.branch,
        number = fields.number,
    )
}

/// The session `claude --bg` just started: the one whose id it printed, else the newest of that name.
fn started(active: &[Session], printed: &str, name: &str) -> Option<Session> {
    let printed_one = active.iter().find(|session| printed.contains(&session.id));
    printed_one.or_else(|| active.iter().filter(|session| session.name == name).max_by_key(|session| session.started)).cloned()
}

fn ready(session: Session, case: Case) -> Ready {
    let call = Call::new(&[PROGRAM, "attach", &session.id]);
    Ready { kept: Kept { name: session.name, dir: session.dir, id: Some(session.resume_id), pane: None }, call, case, notice: None }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::super::tests::{REPO, Script, cache, claude, mr};
    use super::super::{Handoff, Outcome};
    use super::*;

    const WORKTREE: &str = "/code/widgets/.claude/worktrees/feat+checkout";
    const LIST: &str = "claude agents --json --all";

    fn item(id: &str, name: &str, cwd: &str, live: bool, started_at: u64) -> String {
        let process = if live { r#""pid": 4242, "status": "idle","# } else { "" };
        format!(
            r#"{{"id": "{id}", "sessionId": "{id}-0000-4000-8000-000000000000", "name": "{name}", "cwd": "{cwd}", "kind": "background", {process} "state": "done", "startedAt": {started_at}}}"#
        )
    }

    fn list(items: &[String]) -> String {
        format!("[{}]", items.join(","))
    }

    fn resume_id(id: &str) -> String {
        format!("{id}-0000-4000-8000-000000000000")
    }

    /// A jobs folder where the session `id` produced the links `links`.
    fn job(jobs: &Path, id: &str, links: &[&str]) {
        let children: Vec<Value> = links.iter().map(|link| serde_json::json!({"id": "42", "href": link, "kind": "pr"})).collect();
        let state = serde_json::json!({"sessionId": resume_id(id), "name": "x", "cwd": REPO, "children": children});
        std::fs::create_dir_all(jobs.join(id)).unwrap();
        std::fs::write(jobs.join(id).join("state.json"), state.to_string()).unwrap();
    }

    fn handed(outcome: Outcome) -> Handoff {
        let Outcome::Handoff(handoff) = outcome else { panic!("no hand-off: {outcome:?}") };
        handoff
    }

    fn attach(id: &str) -> Call {
        Call::new(&["claude", "attach", id])
    }

    #[test]
    fn the_list_keeps_background_sessions_and_skips_what_it_cannot_read() {
        let printed = format!(
            r#"[{}, {{"id": "cc", "kind": "interactive", "sessionId": "cc-1"}}, {{"id": 7}}, "noise", {{"id": "dd", "sessionId": "dd-1", "kind": "background"}}]"#,
            item("ab12cd34", "revu: nxt", REPO, true, 1_782_983_839_707)
        );
        let sessions = sessions(&printed);
        assert_eq!(sessions.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["ab12cd34", "dd"]);
        assert!(sessions[0].live && sessions[0].started.is_some_and(|at| at.timestamp() == 1_782_983_839));
        assert_eq!((sessions[1].name.as_str(), sessions[1].live, sessions[1].started), ("dd", false, None), "a name falls back to the id");
        assert_eq!(super::sessions("claude: not logged in"), []);
    }

    #[test]
    fn the_remembered_session_wins_and_a_live_one_is_attached_as_it_is() {
        let jobs = tempfile::tempdir().unwrap();
        job(jobs.path(), "0pener00", &["https://gitlab.com/acme/widgets/-/merge_requests/42"]);
        let listed = list(&[item("0pener00", "opened it", REPO, true, 2), item("ab12cd34", "revu: !42 charge cards", WORKTREE, true, 1)]);
        let (agent, script) = claude(Script::default().says(LIST, &listed), jobs.path(), false);
        let (_dir, cache) = cache();
        cache
            .write(
                &keys_session(),
                &Kept { name: "revu: !42 charge cards".into(), dir: WORKTREE.into(), id: Some(resume_id("ab12cd34")), pane: None },
            )
            .unwrap();
        let handoff = handed(agent.open(&mr(), &cache, Choice::Find).unwrap());
        assert_eq!(handoff.call, attach("ab12cd34"));
        assert_eq!(handoff.said, "the session revu remembered for !42 · revu: !42 charge cards");
        assert_eq!(script.lines(), [LIST], "a live session needs no resume");
    }

    fn keys_session() -> String {
        crate::cache::keys::session(&mr().key)
    }

    #[test]
    fn a_remembered_session_claude_no_longer_knows_is_looked_for_again() {
        let jobs = tempfile::tempdir().unwrap();
        job(jobs.path(), "0pener00", &["https://gitlab.com/acme/widgets/-/merge_requests/42/"]);
        let listed = list(&[item("0pener00", "opened it", REPO, true, 2)]);
        let (agent, _script) = claude(Script::default().says(LIST, &listed), jobs.path(), false);
        let (_dir, cache) = cache();
        cache
            .write(&keys_session(), &Kept { name: "gone".into(), dir: WORKTREE.into(), id: Some(resume_id("90900000")), pane: None })
            .unwrap();
        let handoff = handed(agent.open(&mr(), &cache, Choice::Find).unwrap());
        assert_eq!((handoff.call, handoff.said.as_str()), (attach("0pener00"), "the session that opened !42 · opened it"));
        assert_eq!(cache.read::<Kept>(&keys_session()).unwrap().id, Some(resume_id("0pener00")), "the session found is remembered instead");
    }

    #[test]
    fn the_session_that_opened_the_mr_is_found_in_its_state_file_and_resumed_when_stopped() {
        let jobs = tempfile::tempdir().unwrap();
        job(jobs.path(), "0ther000", &["https://gitlab.com/acme/widgets/-/merge_requests/41"]);
        job(jobs.path(), "0pener00", &["https://claude.ai/x", "http://GitLab.com/acme/widgets/merge_requests/42/?tab=diffs#note_1"]);
        std::fs::create_dir_all(jobs.path().join("br0ken00")).unwrap();
        std::fs::write(jobs.path().join("br0ken00/state.json"), "{ not json").unwrap();
        std::fs::create_dir_all(jobs.path().join("n0state0")).unwrap();
        std::fs::write(jobs.path().join("n0field0"), "a file, not a folder").unwrap();
        job(jobs.path(), "n0links0", &[]);
        let listed = list(&[item("0ther000", "other", REPO, true, 3), item("0pener00", "revu: nxt", REPO, false, 1)]);
        let (agent, script) = claude(Script::default().says(LIST, &listed), jobs.path(), false);
        let (_dir, cache) = cache();
        let handoff = handed(agent.open(&mr(), &cache, Choice::Find).unwrap());
        assert_eq!(handoff.call, attach("0pener00"));
        assert_eq!(handoff.said, "the session that opened !42 · revu: nxt");
        assert_eq!(script.lines(), [LIST.to_owned(), format!("claude --bg --resume {}", resume_id("0pener00"))]);
        assert_eq!(script.calls()[1].dir.as_deref(), Some(Path::new(REPO)), "resumed where it was started");
    }

    #[test]
    fn of_two_sessions_that_opened_the_mr_the_live_one_wins_then_the_newest() {
        let known =
            sessions(&list(&[item("aa", "old", REPO, false, 9), item("bb", "live", REPO, true, 1), item("cc", "new", REPO, false, 5)]));
        let pick = |openers: &[&str]| {
            let openers: Vec<String> = openers.iter().map(|id| (*id).to_owned()).collect();
            let Found::Session(session, Case::Opener) = find(&known, None, || openers, None) else { panic!("not found") };
            session.id
        };
        assert_eq!(pick(&["aa", "bb", "cc"]), "bb");
        assert_eq!(pick(&["cc", &resume_id("aa")]), "aa", "the full id counts too");
    }

    #[test]
    fn a_missing_jobs_folder_or_a_page_nobody_opened_finds_no_opener() {
        assert_eq!(openers(Path::new("/revu-no-such-folder/jobs"), &mr().fields.url), [] as [String; 0]);
        let jobs = tempfile::tempdir().unwrap();
        job(jobs.path(), "0ther000", &["https://gitlab.com/acme/widgets/-/merge_requests/420"]);
        assert_eq!(openers(jobs.path(), &mr().fields.url), [] as [String; 0]);
        assert!(same_page("https://github.com/acme/widgets/pull/42/", "https://github.com/Acme/widgets/pull/42"));
        assert!(!same_page("https://github.com/acme/widgets/pull/42", "https://github.com/acme/widgets/pull/4"));
    }

    #[test]
    fn with_nothing_remembered_or_opened_the_sessions_of_the_checkout_are_offered_live_and_newest_first() {
        let jobs = tempfile::tempdir().unwrap();
        let listed = list(&[
            item("e1sewher", "another repo", "/code/api", true, 9),
            item("01d00000", "old", REPO, false, 1),
            item("11ve0000", "live", WORKTREE, true, 2),
            item("new00000", "new", REPO, false, 3),
        ]);
        let (agent, script) = claude(Script::default().says(LIST, &listed), jobs.path(), true);
        let (_dir, cache) = cache();
        let Outcome::Pick(offered) = agent.open(&mr(), &cache, Choice::Find).unwrap() else { panic!("no pick") };
        assert_eq!(offered.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["11ve0000", "new00000", "01d00000"]);
        assert_eq!(script.lines(), [LIST], "nothing starts, nothing shows, until the reader picks");
        assert!(cache.read::<Kept>(&keys_session()).is_none());
    }

    #[test]
    fn a_picked_session_is_resumed_when_stopped_attached_and_remembered() {
        let jobs = tempfile::tempdir().unwrap();
        let picked = sessions(&list(&[item("01d00000", "old", REPO, false, 1)])).remove(0);
        let (agent, script) = claude(Script::default(), jobs.path(), false);
        let (_dir, cache) = cache();
        let handoff = handed(agent.open(&mr(), &cache, Choice::Session(Box::new(picked))).unwrap());
        assert_eq!((handoff.call, handoff.said.as_str()), (attach("01d00000"), "the session picked for !42 · old"));
        assert_eq!(script.lines(), [format!("claude --bg --resume {}", resume_id("01d00000"))]);
        assert_eq!(cache.read::<Kept>(&keys_session()).unwrap().id, Some(resume_id("01d00000")));
    }

    fn starting(listed_before: &str) -> Script {
        let after = list(&[item("5e5510n0", "revu: !42 charge cards at checkout", WORKTREE, true, 5)]);
        Script::default()
            .says(LIST, listed_before)
            .fails("git rev-parse", "`git rev-parse` failed: no message")
            .says("claude --bg", "backgrounded · 5e5510n0 · revu: !42 charge cards at checkout\n")
            .says("claude agents --json", &after)
    }

    #[test]
    fn with_no_session_anywhere_a_new_one_starts_in_plan_mode_in_a_worktree_on_the_branch() {
        let jobs = tempfile::tempdir().unwrap();
        let (agent, script) = claude(starting(&list(&[item("e1sewher", "another repo", "/code/api", true, 9)])), jobs.path(), false);
        let (_dir, cache) = cache();
        let handoff = handed(agent.open(&mr(), &cache, Choice::Find).unwrap());
        assert_eq!(handoff.call, attach("5e5510n0"));
        assert_eq!(handoff.said, "a new session for !42 · revu: !42 charge cards at checkout");
        let calls = script.calls();
        let started = calls.iter().find(|call| call.argv.get(1).is_some_and(|word| word == "--bg")).unwrap();
        let prompt = "This session belongs to !42 \"charge cards at checkout\" by omar: https://gitlab.com/acme/widgets/-/merge_requests/42 \
                      (branch feat/checkout into main). `revu show 42`, `revu diff 42` and `revu comment 42 --at <path>:<line> <text>` \
                      read it and comment on it. Do nothing yet: wait for my instructions.";
        assert_eq!(started.argv, ["claude", "--bg", "--name", "revu: !42 charge cards at checkout", "--permission-mode", "plan", prompt]);
        assert_eq!(started.dir.as_deref(), Some(Path::new(WORKTREE)));
        assert_eq!(
            script.lines()[..5],
            [
                LIST,
                "git worktree list --porcelain",
                "git fetch origin +refs/heads/feat/checkout:refs/remotes/origin/feat/checkout",
                "git rev-parse --verify --quiet refs/heads/feat/checkout",
                "git worktree add -b feat/checkout /code/widgets/.claude/worktrees/feat+checkout origin/feat/checkout",
            ]
        );
        assert_eq!(script.lines()[6], "claude agents --json");
        let kept = cache.read::<Kept>(&keys_session()).unwrap();
        assert_eq!(
            kept,
            Kept { name: "revu: !42 charge cards at checkout".into(), dir: WORKTREE.into(), id: Some(resume_id("5e5510n0")), pane: None }
        );
    }

    #[test]
    fn my_own_mr_starts_in_my_default_mode_and_new_skips_the_search() {
        let jobs = tempfile::tempdir().unwrap();
        let (agent, script) = claude(starting("[]"), jobs.path(), false);
        let (_dir, cache) = cache();
        let mine = Mr { mine: true, ..mr() };
        agent.open(&mine, &cache, Choice::New).unwrap();
        let lines = script.lines();
        assert!(!lines.iter().any(|line| line == LIST), "{lines:?}");
        let started = lines.iter().find(|line| line.starts_with("claude --bg")).unwrap();
        assert!(started.starts_with("claude --bg --name revu: !42 charge cards at checkout This session belongs to"), "{started}");
    }

    #[test]
    fn a_new_session_reuses_the_worktree_that_has_the_branch() {
        let jobs = tempfile::tempdir().unwrap();
        let worktrees = format!("worktree {REPO}\nHEAD aaaa\nbranch refs/heads/feat/checkout\n");
        let behind = starting("[]").says("git worktree list", &worktrees).says("git rev-list", "0\t2\n");
        let (agent, script) = claude(behind, jobs.path(), false);
        let (_dir, cache) = cache();
        let handoff = handed(agent.open(&mr(), &cache, Choice::New).unwrap());
        let said =
            "a new session for !42 · revu: !42 charge cards at checkout · its branch is 2 commits behind origin: pull before you trust it";
        assert_eq!(handoff.said, said, "the branch is used as it is, and the toast says where it stands");
        assert!(!script.lines().iter().any(|line| line.starts_with("git worktree add")));
        let started = script.calls().into_iter().find(|call| call.argv.get(1).is_some_and(|word| word == "--bg")).unwrap();
        assert_eq!(started.dir.as_deref(), Some(Path::new(REPO)));
    }

    #[test]
    fn a_fork_branch_or_an_unknown_checkout_starts_no_session_and_says_why() {
        let jobs = tempfile::tempdir().unwrap();
        let (agent, script) = claude(
            Script::default().fails("git rev-parse", "no").fails("git fetch", "`git fetch` failed: couldn't find remote ref"),
            jobs.path(),
            false,
        );
        let (_dir, cache) = cache();
        let fork = format!("{:#}", agent.open(&mr(), &cache, Choice::Find).unwrap_err());
        assert!(fork.contains("fork") && fork.contains("feat/checkout"), "{fork}");
        let elsewhere = Mr { fields: crate::share::Fields { project: "acme/api".into(), ..mr().fields }, ..mr() };
        let unknown = format!("{:#}", agent.open(&elsewhere, &cache, Choice::Find).unwrap_err());
        assert!(unknown.contains("[agent.checkouts]") && unknown.contains("acme/api"), "{unknown}");
        assert!(!script.lines().iter().any(|line| line.starts_with("claude --bg")), "{:?}", script.lines());
        assert!(cache.read::<Kept>(&keys_session()).is_none());
    }

    #[test]
    fn the_session_just_started_is_the_one_printed_else_the_newest_of_its_name() {
        let active = sessions(&list(&[item("aaaa1111", "revu: !42 x", REPO, true, 1), item("bbbb2222", "revu: !42 x", REPO, true, 2)]));
        assert_eq!(started(&active, "session aaaa1111 is running", "revu: !42 x").unwrap().id, "aaaa1111");
        assert_eq!(started(&active, "started", "revu: !42 x").unwrap().id, "bbbb2222");
        assert_eq!(started(&active, "started", "another name"), None);
    }

    #[test]
    fn the_session_revu_opened_is_found_again_after_a_restart_in_its_tmux_window() {
        let jobs = tempfile::tempdir().unwrap();
        let (dir, cache) = cache();
        let (first, _script) = claude(starting("[]"), jobs.path(), true);
        assert!(matches!(first.open(&mr(), &cache, Choice::Find).unwrap(), Outcome::Shown(_)));
        drop((first, cache));
        let listed = list(&[item("5e5510n0", "revu: !42 charge cards at checkout", WORKTREE, false, 5)]);
        let panes = format!("%0\t@1\t\n%9\t@4\t{}\n", resume_id("5e5510n0"));
        let (again, script) = claude(Script::default().says(LIST, &listed).says("tmux list-panes", &panes), jobs.path(), true);
        let shown = again.open(&mr(), &crate::cache::Cache::in_dir(dir.path()), Choice::Find).unwrap();
        let text = "the session revu remembered for !42 · revu: !42 charge cards at checkout · already open in tmux";
        assert_eq!(shown, Outcome::Shown(text.into()));
        let resume = format!("claude --bg --resume {}", resume_id("5e5510n0"));
        let listing = "tmux list-panes -s -F #{pane_id}\t#{window_id}\t#{@revu_session}";
        assert_eq!(script.lines(), [LIST, resume.as_str(), listing, "tmux select-window -t @4", "tmux select-pane -t %9"]);
    }
}
