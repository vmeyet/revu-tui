//! Putting a session on screen: a tmux window named after it when revu runs in tmux, else the terminal itself.
use super::{Call, Handoff, Outcome, Ready, Run};
use anyhow::Result;

/// One window of revu's own tmux session per line: its id, a tab, its name.
const WINDOWS: &str = "#{window_id}\t#{window_name}";

pub(super) fn show(run: &dyn Run, tmux: bool, ready: &Ready, reference: &str) -> Result<Outcome> {
    let said = format!("{} · {}", ready.case.said(reference), ready.kept.name);
    if !tmux {
        return Ok(Outcome::Handoff(Handoff { call: ready.call.clone(), said }));
    }
    let windows = run.output(&Call::new(&["tmux", "list-windows", "-F", WINDOWS]))?;
    let call = match window_named(&windows, &ready.kept.name) {
        Some(window) => Call::new(&["tmux", "select-window", "-t", window]),
        None => new_window(ready),
    };
    run.output(&call)?;
    Ok(Outcome::Window(format!("{said} · in its tmux window")))
}

fn window_named<'a>(windows: &'a str, name: &str) -> Option<&'a str> {
    windows.lines().filter_map(|line| line.split_once('\t')).find(|(_, window)| *window == name).map(|(id, _)| id)
}

/// The program's words go to tmux one by one, so tmux runs them as they are, with no shell between.
fn new_window(ready: &Ready) -> Call {
    let folder = ready.call.dir.as_ref().map(|dir| dir.to_string_lossy().into_owned());
    let in_folder: Vec<&str> = folder.iter().flat_map(|dir| ["-c", dir.as_str()]).collect();
    let program: Vec<&str> = ready.call.argv.iter().map(String::as_str).collect();
    Call::new(&[&["tmux", "new-window", "-n", ready.kept.name.as_str()], in_folder.as_slice(), program.as_slice()].concat())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::super::tests::Script;
    use super::super::{Case, Kept};
    use super::*;

    fn ready(dir: Option<&str>) -> Ready {
        let call = Call::new(&["claude", "attach", "ab12cd34"]);
        Ready {
            kept: Kept { name: "revu: !42 charge cards".into(), dir: "/code/widgets".into(), id: Some("ab12cd34-0000".into()) },
            call: Call { dir: dir.map(Into::into), ..call },
            case: Case::Remembered,
        }
    }

    #[test]
    fn outside_tmux_the_program_comes_back_for_the_terminal_and_nothing_runs() {
        let script = Script::default();
        let shown = show(&script, false, &ready(None), "!42").unwrap();
        let said = "the session revu remembered for !42 · revu: !42 charge cards".to_owned();
        assert_eq!(shown, Outcome::Handoff(Handoff { call: Call::new(&["claude", "attach", "ab12cd34"]), said }));
        assert_eq!(script.lines(), [] as [String; 0]);
    }

    #[test]
    fn in_tmux_a_new_window_named_after_the_session_runs_the_program_word_by_word() {
        let script = Script::default().says("tmux list-windows", "@1\trevu\n@2\tzsh\n");
        let shown = show(&script, true, &ready(None), "!42").unwrap();
        assert_eq!(shown, Outcome::Window("the session revu remembered for !42 · revu: !42 charge cards · in its tmux window".into()));
        let calls = script.calls();
        assert_eq!(calls[0].argv, ["tmux", "list-windows", "-F", "#{window_id}\t#{window_name}"]);
        assert_eq!(calls[1].argv, ["tmux", "new-window", "-n", "revu: !42 charge cards", "claude", "attach", "ab12cd34"]);
    }

    #[test]
    fn a_program_with_a_folder_opens_its_window_there() {
        let script = Script::default();
        show(&script, true, &ready(Some("/code/widgets/.claude/worktrees/feat+checkout")), "!42").unwrap();
        let wanted = "tmux new-window -n revu: !42 charge cards -c /code/widgets/.claude/worktrees/feat+checkout claude attach ab12cd34";
        assert_eq!(script.lines()[1], wanted);
    }

    #[test]
    fn a_window_already_open_for_the_session_is_selected_instead_of_opened_twice() {
        let script = Script::default().says("tmux list-windows", "@1\trevu\n@7\trevu: !42 charge cards\n");
        show(&script, true, &ready(None), "!42").unwrap();
        assert_eq!(script.lines()[1], "tmux select-window -t @7");
        assert_eq!(script.calls().len(), 2);
    }
}
