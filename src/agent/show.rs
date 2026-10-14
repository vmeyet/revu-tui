//! A session in tmux: a pane beside revu when revu is wide enough for both, else a window of its own.
//! With no tmux, the session's program goes back to the loop, which lends it the terminal.
use super::{Call, Handoff, Outcome, Ready, Run};
use crate::config::AgentShow;
use anyhow::Result;

/// The pane option revu sets on a session's pane, so asking again finds it whatever its name became.
const MARK: &str = "@revu_session";
/// One pane of revu's own tmux session per line: its id, its window's id, the session it shows.
const PANES: &str = "#{pane_id}\t#{window_id}\t#{@revu_session}";
/// What tmux prints for a pane it just made.
const NEW_PANE: &str = "#{pane_id}";

/// revu runs in tmux: where a session goes, and the width under which revu itself turns compact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Tmux {
    pub(super) show: AgentShow,
    pub(super) compact_width: u16,
}

/// Which session this is and how it was found, for the toast.
pub(super) fn said(ready: &Ready, reference: &str) -> String {
    let notice = ready.notice.as_ref().map(|notice| format!(" · {notice}")).unwrap_or_default();
    format!("{} · {}{notice}", ready.case.said(reference), ready.kept.name)
}

pub(super) fn show(run: &dyn Run, tmux: Option<Tmux>, ready: &Ready, reference: &str) -> Result<Outcome> {
    let said = said(ready, reference);
    let Some(tmux) = tmux else {
        return Ok(Outcome::Handoff(Handoff { call: ready.call.clone(), said }));
    };
    let mark = ready.kept.id.as_deref().unwrap_or(&ready.kept.name);
    let panes = run.output(&Call::new(&["tmux", "list-panes", "-s", "-F", PANES]))?;
    if let Some((pane, window)) = pane_marked(&panes, mark) {
        run.output(&Call::new(&["tmux", "select-window", "-t", window]))?;
        run.output(&Call::new(&["tmux", "select-pane", "-t", pane]))?;
        return Ok(Outcome::Shown(format!("{said} · already open in tmux")));
    }
    let beside = is_beside(run, tmux);
    let pane = run.output(&opening(ready, beside))?;
    run.output(&Call::new(&["tmux", "set-option", "-p", "-t", pane.trim(), MARK, mark]))?;
    Ok(Outcome::Shown(format!("{said} · in a tmux {}", if beside { "pane beside revu" } else { "window" })))
}

/// The pane and the window of the pane marked with `mark`.
fn pane_marked<'a>(panes: &'a str, mark: &str) -> Option<(&'a str, &'a str)> {
    let fields = panes.lines().map(|line| line.split('\t').collect::<Vec<_>>());
    fields
        .filter_map(|fields| <[&str; 3]>::try_from(fields).ok())
        .find(|[_, _, marked]| *marked == mark)
        .map(|[pane, window, _]| (pane, window))
}

/// A pane beside revu, or a window: what `[agent] show` says, and under `auto` whether revu's pane fits both.
fn is_beside(run: &dyn Run, tmux: Tmux) -> bool {
    match tmux.show {
        AgentShow::Split => true,
        AgentShow::Window => false,
        AgentShow::Auto => {
            let width = run.output(&Call::new(&["tmux", "display-message", "-p", "#{pane_width}"]));
            width.ok().and_then(|width| width.trim().parse().ok()).is_some_and(|width| fits_beside(width, tmux.compact_width))
        }
    }
}

/// A split leaves revu half of its `width`: enough when that half is no compact window.
pub(super) fn fits_beside(width: u16, compact_width: u16) -> bool {
    width / 2 >= compact_width
}

/// The program's words go to tmux one by one, so tmux runs them as they are, with no shell between.
fn opening(ready: &Ready, beside: bool) -> Call {
    let place: &[&str] = if beside { &["split-window", "-h"] } else { &["new-window", "-n", ready.kept.name.as_str()] };
    let folder = ready.call.dir.as_ref().map(|dir| dir.to_string_lossy().into_owned());
    let in_folder: Vec<&str> = folder.iter().flat_map(|dir| ["-c", dir.as_str()]).collect();
    let words = ready.call.words();
    let program: Vec<&str> = words.iter().map(String::as_str).collect();
    Call::new(&[&["tmux"], place, &["-P", "-F", NEW_PANE], in_folder.as_slice(), program.as_slice()].concat())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::super::tests::Script;
    use super::super::{Case, Kept};
    use super::*;

    const ID: &str = "ab12cd34-0000";
    const LISTING: &str = "tmux list-panes -s -F #{pane_id}\t#{window_id}\t#{@revu_session}";
    const WIDTH: &str = "tmux display-message -p #{pane_width}";
    const SPLIT: &str = "tmux split-window -h -P -F #{pane_id} claude attach ab12cd34";
    const WINDOW: &str = "tmux new-window -n revu: !42 charge cards -P -F #{pane_id} claude attach ab12cd34";

    fn ready(dir: Option<&str>) -> Ready {
        let call = Call::new(&["claude", "attach", "ab12cd34"]);
        Ready {
            kept: Kept { name: "revu: !42 charge cards".into(), dir: "/code/widgets".into(), id: Some(ID.into()), pane: None },
            call: Call { dir: dir.map(Into::into), ..call },
            case: Case::Remembered,
            notice: None,
        }
    }

    fn tmux(show: AgentShow) -> Tmux {
        Tmux { show, compact_width: 70 }
    }

    /// A tmux where revu's pane is `width` columns wide and no session is on show.
    fn wide(width: u16) -> Script {
        Script::default()
            .says("tmux display-message", &format!("{width}\n"))
            .says("tmux split-window", "%5\n")
            .says("tmux new-window", "%6\n")
    }

    #[test]
    fn outside_tmux_the_program_comes_back_for_the_terminal_and_nothing_runs() {
        let script = Script::default();
        let shown = show(&script, None, &ready(None), "!42").unwrap();
        let said = "the session revu remembered for !42 · revu: !42 charge cards".to_owned();
        assert_eq!(shown, Outcome::Handoff(Handoff { call: Call::new(&["claude", "attach", "ab12cd34"]), said }));
        assert_eq!(script.lines(), [] as [String; 0]);
    }

    #[test]
    fn wide_enough_the_session_opens_in_a_pane_beside_revu_marked_with_its_id() {
        let script = wide(200);
        let shown = show(&script, Some(tmux(AgentShow::Auto)), &ready(None), "!42").unwrap();
        let said = "the session revu remembered for !42 · revu: !42 charge cards · in a tmux pane beside revu";
        assert_eq!(shown, Outcome::Shown(said.into()));
        assert_eq!(script.lines(), [LISTING, WIDTH, SPLIT, "tmux set-option -p -t %5 @revu_session ab12cd34-0000"]);
        assert_eq!(script.calls()[2].argv[6..], ["claude", "attach", "ab12cd34"], "the program goes to tmux word by word");
    }

    #[test]
    fn too_narrow_for_both_it_opens_in_a_window_named_after_the_session() {
        let script = wide(139);
        let shown = show(&script, Some(tmux(AgentShow::Auto)), &ready(None), "!42").unwrap();
        assert_eq!(shown, Outcome::Shown("the session revu remembered for !42 · revu: !42 charge cards · in a tmux window".into()));
        assert_eq!(script.lines(), [LISTING, WIDTH, WINDOW, "tmux set-option -p -t %6 @revu_session ab12cd34-0000"]);
    }

    #[test]
    fn a_pane_fits_beside_revu_when_half_the_width_is_no_compact_window() {
        assert!(fits_beside(140, 70) && fits_beside(141, 70));
        assert!(!fits_beside(139, 70));
        assert!(fits_beside(100, 50), "the limit follows `compact_width`");
        assert!(!fits_beside(400, u16::MAX), "the compact layout forced at any width never splits");
        let unknown = Script::default().says("tmux new-window", "%6\n");
        show(&unknown, Some(tmux(AgentShow::Auto)), &ready(None), "!42").unwrap();
        assert_eq!(unknown.lines()[2], WINDOW, "a width tmux does not give opens a window");
    }

    #[test]
    fn the_setting_forces_a_pane_or_a_window_without_asking_the_width() {
        let narrow = wide(60);
        show(&narrow, Some(tmux(AgentShow::Split)), &ready(None), "!42").unwrap();
        assert_eq!(narrow.lines()[..2], [LISTING, SPLIT]);
        let large = wide(300);
        show(&large, Some(tmux(AgentShow::Window)), &ready(None), "!42").unwrap();
        assert_eq!(large.lines()[..2], [LISTING, WINDOW]);
    }

    #[test]
    fn a_program_with_a_folder_opens_there_in_a_pane_and_in_a_window() {
        let folder = "/code/widgets/.claude/worktrees/feat+checkout";
        let script = wide(200);
        show(&script, Some(tmux(AgentShow::Split)), &ready(Some(folder)), "!42").unwrap();
        assert_eq!(script.lines()[1], format!("tmux split-window -h -P -F #{{pane_id}} -c {folder} claude attach ab12cd34"));
        let script = wide(200);
        show(&script, Some(tmux(AgentShow::Window)), &ready(Some(folder)), "!42").unwrap();
        assert_eq!(
            script.lines()[1],
            format!("tmux new-window -n revu: !42 charge cards -P -F #{{pane_id}} -c {folder} claude attach ab12cd34")
        );
    }

    #[test]
    fn a_session_already_in_a_pane_of_revus_window_gets_the_keys_and_none_opens() {
        let panes = format!("%0\t@1\t\n%5\t@1\t{ID}\n%7\t@2\tanother-id\n");
        let script = wide(200).says("tmux list-panes", &panes);
        let shown = show(&script, Some(tmux(AgentShow::Auto)), &ready(None), "!42").unwrap();
        assert_eq!(shown, Outcome::Shown("the session revu remembered for !42 · revu: !42 charge cards · already open in tmux".into()));
        assert_eq!(script.lines(), [LISTING, "tmux select-window -t @1", "tmux select-pane -t %5"]);
    }

    #[test]
    fn a_session_already_in_a_window_of_its_own_is_gone_to_whatever_the_setting() {
        let panes = format!("%0\t@1\t\n%9\t@4\t{ID}\n");
        let script = wide(200).says("tmux list-panes", &panes);
        show(&script, Some(tmux(AgentShow::Split)), &ready(None), "!42").unwrap();
        assert_eq!(script.lines(), [LISTING, "tmux select-window -t @4", "tmux select-pane -t %9"]);
    }

    #[test]
    fn a_session_with_no_id_of_its_own_is_marked_with_its_name() {
        let nameless = Ready { kept: Kept { id: None, ..ready(None).kept }, ..ready(None) };
        let script = wide(200);
        show(&script, Some(tmux(AgentShow::Split)), &nameless, "!42").unwrap();
        assert_eq!(script.lines()[2], "tmux set-option -p -t %5 @revu_session revu: !42 charge cards");
        let open = wide(200).says("tmux list-panes", "%5\t@1\trevu: !42 charge cards\n");
        show(&open, Some(tmux(AgentShow::Split)), &nameless, "!42").unwrap();
        assert_eq!(open.lines()[1..], ["tmux select-window -t @1", "tmux select-pane -t %5"]);
    }
}
