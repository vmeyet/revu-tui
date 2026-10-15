//! Another agent than Claude Code: two commands of the reader's, with the MR filled in.
//! revu cannot ask such an agent what it runs, so a session is either the one revu remembers or a new one.
use super::{Agent, Call, Case, Kept, Mr, Ready, Step, worktree};
use anyhow::{Context, Result, bail};
use std::path::Path;

const PLACEHOLDERS: [&str; 10] = ["ref", "iid", "url", "author", "project", "branch", "target", "dir", "name", "title"];

/// The command splits into words, starts with a program and names only placeholders revu fills; checked
/// when the config loads. A program that is a placeholder would run text the forge wrote.
pub fn check(command: &str, key: &str) -> Result<()> {
    crate::program::check(command, key)?;
    let program = command.split_whitespace().next().unwrap_or_default();
    if crate::share::placeholders(program).next().is_some() {
        bail!("`{key}` starts with a placeholder: its first word is the program to run");
    }
    crate::share::check_placeholders(command, key, &PLACEHOLDERS)
}

pub(super) fn session(agent: &Agent, start: &str, open: Option<&str>, mr: &Mr, kept: Option<&Kept>) -> Result<Step> {
    if let (Some(kept), Some(open)) = (kept, open) {
        let call = call(open, mr, &kept.name, &kept.dir)?;
        return Ok(Step::Ready(Ready { kept: kept.clone(), call, case: Case::Remembered, notice: None }));
    }
    let folder = worktree::on_branch(agent.run.as_ref(), agent.checkout(&mr.fields.project)?, &mr.fields.branch)?;
    let name = mr.session_name();
    let call = call(start, mr, &name, &folder.dir)?;
    let kept = Kept { name, dir: folder.dir, id: None, pane: None };
    Ok(Step::Ready(Ready { kept, call, case: Case::New, notice: folder.notice }))
}

/// The command's words with the MR filled in, word by word after the split, so a title with a quote stays one word.
fn call(command: &str, mr: &Mr, name: &str, dir: &Path) -> Result<Call> {
    let words = shell_words::split(command).with_context(|| format!("`{command}` does not split into words"))?;
    let fields = &mr.fields;
    let values = [
        fields.reference.clone(),
        fields.number.to_string(),
        fields.url.clone(),
        fields.author.clone(),
        fields.project.clone(),
        fields.branch.clone(),
        mr.target.clone(),
        dir.to_string_lossy().into_owned(),
        name.to_owned(),
        fields.title.clone(),
    ];
    let fill = |word: &String| {
        PLACEHOLDERS.iter().zip(&values).fold(word.clone(), |word, (name, value)| word.replace(&format!("{{{name}}}"), value))
    };
    Ok(Call::new(&words.iter().map(fill).collect::<Vec<_>>()).in_dir(dir))
}

#[cfg(test)]
mod tests {
    use super::super::tests::{Script, agent, cache, mr};
    use super::super::{Choice, Handoff, Harness, Outcome};
    use super::*;
    use std::path::PathBuf;

    const START: &str = "aider --message 'Review {ref} \"{title}\" by {author}: {url} ({branch} into {target})' --name {name}";
    const WORKTREE: &str = "/code/widgets/.claude/worktrees/feat+checkout";

    fn custom(open: Option<&str>) -> Harness {
        Harness::Custom { start: START.into(), open: open.map(str::to_owned) }
    }

    #[test]
    fn commands_name_only_placeholders_revu_fills() {
        assert!(check(START, "agent.start").is_ok());
        assert!(check("aider --config '{\"a\": 1}' {dir} {iid} {project}", "agent.start").is_ok(), "JSON braces are not placeholders");
        let typo = check("aider {titel}", "agent.start").unwrap_err().to_string();
        assert!(typo.contains("agent.start") && typo.contains("{titel}") && typo.contains("{title}"), "{typo}");
        assert!(check("aider 'open", "agent.open").unwrap_err().to_string().contains("agent.open"));
        assert!(check(" ", "agent.start").is_err());
        let program = check("{title} --review", "agent.start").unwrap_err().to_string();
        assert!(program.contains("agent.start") && program.contains("program"), "{program}");
    }

    #[test]
    fn the_first_time_the_start_command_runs_in_the_worktree_with_the_mr_filled_in() {
        let (agent, script) = agent(custom(Some("aider --restore {dir}")), Script::default(), false);
        let (_dir, cache) = cache();
        let Outcome::Handoff(Handoff { call, said }) = agent.open(&mr(), &cache, Choice::Find).unwrap() else { panic!("no hand-off") };
        assert_eq!(
            call.argv,
            [
                "aider",
                "--message",
                "Review !42 \"charge cards at checkout\" by omar: https://gitlab.com/acme/widgets/-/merge_requests/42 (feat/checkout into main)",
                "--name",
                "revu: !42 charge cards at checkout",
            ]
        );
        assert_eq!(call.dir, Some(PathBuf::from(WORKTREE)));
        assert_eq!(said, "a new session for !42 · revu: !42 charge cards at checkout");
        assert!(script.lines().iter().all(|line| line.starts_with("git ")), "only git ran: {:?}", script.lines());
    }

    #[test]
    fn the_next_time_the_open_command_runs_in_the_folder_revu_remembered() {
        let (agent, script) = agent(custom(Some("aider --restore {dir}")), Script::default(), false);
        let (_dir, cache) = cache();
        agent.open(&mr(), &cache, Choice::Find).unwrap();
        let before = script.calls().len();
        let Outcome::Handoff(Handoff { call, said }) = agent.open(&mr(), &cache, Choice::Find).unwrap() else { panic!("no hand-off") };
        assert_eq!(call, Call::new(&["aider", "--restore", WORKTREE]).in_dir(Path::new(WORKTREE)));
        assert!(said.starts_with("the session revu remembered for !42"), "{said}");
        assert_eq!(script.calls().len(), before, "nothing is looked for: no git, no list");
    }

    #[test]
    fn without_an_open_command_start_runs_again() {
        let (agent, _script) = agent(custom(None), Script::default(), false);
        let (_dir, cache) = cache();
        agent.open(&mr(), &cache, Choice::Find).unwrap();
        let Outcome::Handoff(Handoff { call, said }) = agent.open(&mr(), &cache, Choice::Find).unwrap() else { panic!("no hand-off") };
        assert_eq!(call.argv[0], "aider");
        assert!(said.starts_with("a new session for !42"), "{said}");
    }

    #[test]
    fn in_tmux_the_command_gets_a_pane_in_the_worktree_marked_with_the_session_name() {
        let wide = Script::default().says("tmux display-message", "200\n").says("tmux split-window", "%5\n");
        let (agent, script) = agent(custom(None), wide, true);
        let (_dir, cache) = cache();
        assert!(matches!(agent.open(&mr(), &cache, Choice::Find).unwrap(), Outcome::Shown(_)));
        let lines = script.lines();
        let pane = lines.iter().find(|line| line.starts_with("tmux split-window")).unwrap();
        assert!(pane.starts_with(&format!("tmux split-window -h -P -F #{{pane_id}} -c {WORKTREE} aider --message")), "{pane}");
        assert_eq!(lines.last().unwrap(), "tmux set-option -p -t %5 @revu_session revu: !42 charge cards at checkout");
    }
}
