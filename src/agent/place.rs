//! Where a session goes, in this order: tmux when revu runs in it, else the terminal's own split when
//! revu knows the terminal, else the reader's split command, else the terminal itself, lent.
use super::show::{self, Tmux, fits_beside};
use super::terminal::Terminal;
use super::{Call, Handoff, Outcome, Ready, Run};
use crate::config::AgentShow;
use anyhow::{Context, Result, bail};
use std::path::Path;

/// The word of `[agent] split` that becomes the session's program, word by word.
const COMMAND: &str = "{command}";
const PLACEHOLDERS: [&str; 2] = ["command", "dir"];

/// What can put a session beside revu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Screen {
    Tmux,
    Terminal(Terminal),
    /// `[agent] split`: a command of the reader's that opens a pane.
    Command(String),
    /// Nothing splits: the session takes the terminal.
    Plain,
}

/// The screen revu runs on, and what `[agent] show` and the compact width say about splitting it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Place {
    pub(super) screen: Screen,
    pub(super) show: AgentShow,
    pub(super) compact_width: u16,
}

/// A session on show, or on its way to the terminal, and the pane to remember it by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Placed {
    pub(super) outcome: Outcome,
    pub(super) pane: Option<String>,
}

impl Placed {
    fn unmarked(outcome: Outcome) -> Self {
        Self { outcome, pane: None }
    }

    fn lent(ready: &Ready, said: String) -> Self {
        Self::unmarked(Outcome::Handoff(Handoff { call: ready.call.clone(), said }))
    }
}

/// `[agent] split` reads as words, starts with a program, and has `{command}` as a word of its own.
pub fn check_split(command: &str, key: &str) -> Result<()> {
    crate::program::check(command, key)?;
    let mut words = command.split_whitespace();
    if words.next().is_some_and(|program| crate::share::placeholders(program).next().is_some()) {
        bail!("`{key}` starts with a placeholder: its first word is the program to run");
    }
    if !words.any(|word| word == COMMAND) {
        bail!("`{key}` needs `{COMMAND}` as a word of its own: the session's program goes there, word by word");
    }
    crate::share::check_placeholders(command, key, &PLACEHOLDERS)
}

impl Place {
    /// revu's own pane, for a terminal that must be asked, and only when a split may follow.
    pub(super) fn origin(&self, run: &dyn Run) -> Option<String> {
        match &self.screen {
            Screen::Terminal(terminal) if self.show != AgentShow::Window => terminal.origin(run),
            _ => None,
        }
    }

    /// Shows the session of `ready`; `pane` is where revu showed it last, if it did.
    pub(super) fn put(&self, run: &dyn Run, origin: Option<&str>, ready: &Ready, pane: Option<&str>, reference: &str) -> Result<Placed> {
        if self.screen == Screen::Plain {
            return show::show(run, None, ready, reference).map(Placed::unmarked);
        }
        let ready = &Ready { call: pane_call(run, &ready.call)?, ..ready.clone() };
        let tmux = Tmux { show: self.show, compact_width: self.compact_width };
        match &self.screen {
            Screen::Tmux | Screen::Plain => show::show(run, Some(tmux), ready, reference).map(Placed::unmarked),
            Screen::Terminal(terminal) => Ok(self.in_terminal(run, terminal, origin, ready, pane, reference)),
            Screen::Command(command) => Ok(self.by_command(run, command, ready, reference)),
        }
    }

    /// The remembered pane when the terminal still has it, else a new one; a terminal that refuses lends itself.
    fn in_terminal(
        &self,
        run: &dyn Run,
        terminal: &Terminal,
        origin: Option<&str>,
        ready: &Ready,
        pane: Option<&str>,
        reference: &str,
    ) -> Placed {
        let said = show::said(ready, reference);
        if let Some(pane) = pane.filter(|pane| terminal.focus(run, pane)) {
            return Placed { outcome: Outcome::Shown(format!("{said} · already open beside revu")), pane: Some(pane.to_owned()) };
        }
        if !self.is_beside(run) {
            return Placed::lent(ready, said);
        }
        match terminal.split(run, origin, &ready.call, &ready.kept.dir) {
            Ok(pane) => Placed { outcome: Outcome::Shown(format!("{said} · in a pane beside revu")), pane: Some(pane) },
            Err(why) => Placed::lent(ready, format!("{said} · {} did not split: {why:#}", terminal.name())),
        }
    }

    /// The reader's command opens the pane; revu cannot tell later whether it is still there.
    fn by_command(&self, run: &dyn Run, command: &str, ready: &Ready, reference: &str) -> Placed {
        let said = show::said(ready, reference);
        if !self.is_beside(run) {
            return Placed::lent(ready, said);
        }
        match split_call(command, &ready.call, &ready.kept.dir).and_then(|call| run.output(&call)) {
            Ok(_) => Placed::unmarked(Outcome::Shown(format!("{said} · in a pane beside revu"))),
            Err(why) => Placed::lent(ready, format!("{said} · the split command failed: {why:#}")),
        }
    }

    /// What `[agent] show` says, and under `auto` whether revu's terminal fits both.
    fn is_beside(&self, run: &dyn Run) -> bool {
        match self.show {
            AgentShow::Split => true,
            AgentShow::Window => false,
            AgentShow::Auto => run.width().is_some_and(|width| fits_beside(width, self.compact_width)),
        }
    }
}

/// The program as a pane runs it. A pane starts with the terminal's own environment, not revu's: the
/// program is named by its full path, and it carries revu's `PATH` so what it starts in turn is found too.
fn pane_call(run: &dyn Run, call: &Call) -> Result<Call> {
    let program = call.program();
    let located = run.locate(program).with_context(|| format!("no session shown: `{program}` is not on revu's PATH"))?;
    let argv = std::iter::once(located).chain(call.argv.iter().skip(1).cloned()).collect();
    Ok(Call { argv, path: run.search_path(), ..call.clone() })
}

/// The split command's words, `{command}` replaced by the program's own words and `{dir}` by its folder.
fn split_call(command: &str, program: &Call, dir: &Path) -> Result<Call> {
    let words = shell_words::split(command).with_context(|| format!("`{command}` does not split into words"))?;
    let folder = dir.to_string_lossy();
    let filled = |word: &String| if word == COMMAND { program.words() } else { vec![word.replace("{dir}", &folder)] };
    Ok(Call::new(&words.iter().flat_map(filled).collect::<Vec<_>>()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::super::terminal::CLEAR;
    use super::super::tests::Script;
    use super::super::{Case, Kept};
    use super::*;

    const SAID: &str = "a new session for !42 · revu: !42 charge cards";
    const KITTY: &str = "kitty @ launch --location=vsplit --cwd {dir} {command}";

    fn ready() -> Ready {
        Ready {
            kept: Kept {
                name: "revu: !42 charge cards".into(),
                dir: "/code/my widgets".into(),
                id: Some("ab12cd34-0000".into()),
                pane: None,
            },
            call: Call::new(&["claude", "attach", "ab12cd34"]),
            case: Case::New,
            notice: None,
        }
    }

    fn place(screen: Screen, show: AgentShow) -> Place {
        Place { screen, show, compact_width: 70 }
    }

    fn ghostty(show: AgentShow) -> Place {
        place(Screen::Terminal(Terminal::Ghostty), show)
    }

    fn lent(said: &str) -> Placed {
        Placed::lent(&ready(), said.to_owned())
    }

    /// A Ghostty that says revu is in terminal `A1` and makes `B2` on a split.
    fn splitting(width: u16) -> Script {
        Script::default()
            .wide(width)
            .says("osascript -e tell application", "A1\n")
            .says("osascript -e on run argv\n  tell application \"Ghostty\"\n    set revu", "B2\n")
    }

    #[test]
    fn tmux_comes_first_and_a_plain_terminal_is_lent() {
        let script = Script::default().says("tmux split-window", "%5\n");
        let placed = place(Screen::Tmux, AgentShow::Split).put(&script, None, &ready(), None, "!42").unwrap();
        assert_eq!(placed, Placed::unmarked(Outcome::Shown(format!("{SAID} · in a tmux pane beside revu"))));
        assert!(script.lines()[1].starts_with("tmux split-window -h"));
        let script = Script::default().wide(300);
        let plain = place(Screen::Plain, AgentShow::Split);
        assert_eq!(plain.origin(&script), None);
        assert_eq!(plain.put(&script, None, &ready(), None, "!42").unwrap(), lent(SAID));
        assert_eq!(script.lines(), [] as [String; 0]);
    }

    #[test]
    fn a_known_terminal_wide_enough_splits_and_gives_the_pane_to_remember() {
        let script = splitting(200);
        let place = ghostty(AgentShow::Auto);
        let origin = place.origin(&script);
        assert_eq!(origin.as_deref(), Some("A1"));
        let placed = place.put(&script, origin.as_deref(), &ready(), None, "!42").unwrap();
        assert_eq!(placed, Placed { outcome: Outcome::Shown(format!("{SAID} · in a pane beside revu")), pane: Some("B2".into()) });
        let typed = format!(" {CLEAR}; exec claude attach ab12cd34");
        assert_eq!(script.calls()[1].argv[3..], ["A1", typed.as_str(), "/code/my widgets"]);
    }

    #[test]
    fn too_narrow_or_told_window_the_terminal_is_lent_and_split_ignores_the_width() {
        let narrow = splitting(139);
        assert_eq!(ghostty(AgentShow::Auto).put(&narrow, Some("A1"), &ready(), None, "!42").unwrap(), lent(SAID));
        let unknown = Script::default();
        assert_eq!(ghostty(AgentShow::Auto).put(&unknown, Some("A1"), &ready(), None, "!42").unwrap(), lent(SAID), "no width, no split");
        let wide = splitting(300);
        let window = ghostty(AgentShow::Window);
        assert_eq!(window.origin(&wide), None, "nothing is asked when nothing will split");
        assert_eq!(window.put(&wide, None, &ready(), None, "!42").unwrap(), lent(SAID));
        assert_eq!(wide.lines(), [] as [String; 0]);
        let forced = ghostty(AgentShow::Split).put(&splitting(60), Some("A1"), &ready(), None, "!42").unwrap();
        assert_eq!(forced.pane.as_deref(), Some("B2"));
    }

    #[test]
    fn a_remembered_pane_still_there_gets_the_keys_and_nothing_opens() {
        let script = Script::default().wide(200).says("osascript", "focused\n");
        let placed = ghostty(AgentShow::Auto).put(&script, Some("A1"), &ready(), Some("B2"), "!42").unwrap();
        assert_eq!(placed, Placed { outcome: Outcome::Shown(format!("{SAID} · already open beside revu")), pane: Some("B2".into()) });
        assert_eq!(script.calls().len(), 1);
        let even_as_window = ghostty(AgentShow::Window).put(&script, None, &ready(), Some("B2"), "!42").unwrap();
        assert_eq!(even_as_window.pane.as_deref(), Some("B2"), "a session on show is gone to whatever the setting");
    }

    #[test]
    fn a_remembered_pane_that_is_gone_is_replaced_by_a_new_split() {
        let script = Script::default()
            .wide(200)
            .says("osascript -e on run argv\n  tell application \"Ghostty\"\n    set found", "gone\n")
            .says("osascript", "C3\n");
        let placed = ghostty(AgentShow::Auto).put(&script, Some("A1"), &ready(), Some("B2"), "!42").unwrap();
        assert_eq!(placed.pane.as_deref(), Some("C3"));
        assert_eq!(script.calls().len(), 2, "one look for the old pane, one split");
    }

    #[test]
    fn a_terminal_that_refuses_lends_itself_and_the_toast_says_why() {
        let refused = "`osascript -e` failed: execution error: Not authorized to send Apple events to Ghostty. (-1743)";
        let script = Script::default().wide(200).fails("osascript", refused);
        let place = ghostty(AgentShow::Auto);
        let origin = place.origin(&script);
        let placed = place.put(&script, origin.as_deref(), &ready(), None, "!42").unwrap();
        assert_eq!(placed, lent(&format!("{SAID} · Ghostty did not split: Ghostty did not say which terminal revu runs in")));
        let placed = place.put(&script, Some("A1"), &ready(), None, "!42").unwrap();
        assert_eq!(placed, lent(&format!("{SAID} · Ghostty did not split: {refused}")));
    }

    #[test]
    fn the_split_command_gets_the_program_word_by_word_and_the_folder() {
        let script = Script::default().wide(200);
        let custom = place(Screen::Command(KITTY.into()), AgentShow::Auto);
        let placed = custom.put(&script, None, &ready(), Some("ignored"), "!42").unwrap();
        assert_eq!(placed, Placed::unmarked(Outcome::Shown(format!("{SAID} · in a pane beside revu"))));
        let wanted = ["kitty", "@", "launch", "--location=vsplit", "--cwd", "/code/my widgets", "claude", "attach", "ab12cd34"];
        assert_eq!(script.calls()[0].argv, wanted);
        custom.put(&script, None, &ready(), None, "!42").unwrap();
        assert_eq!(script.calls().len(), 2, "asking again runs it again: revu cannot see the pane");
    }

    #[test]
    fn the_split_command_follows_show_and_a_failure_lends_the_terminal() {
        let custom = |show| place(Screen::Command(KITTY.into()), show);
        let narrow = Script::default().wide(100);
        assert_eq!(custom(AgentShow::Auto).put(&narrow, None, &ready(), None, "!42").unwrap(), lent(SAID));
        assert_eq!(custom(AgentShow::Window).put(&narrow, None, &ready(), None, "!42").unwrap(), lent(SAID));
        assert_eq!(narrow.lines(), [] as [String; 0]);
        let missing = Script::default().fails("kitty", "cannot run `kitty`: No such file or directory");
        let placed = custom(AgentShow::Split).put(&missing, None, &ready(), None, "!42").unwrap();
        assert_eq!(placed, lent(&format!("{SAID} · the split command failed: cannot run `kitty`: No such file or directory")));
    }

    const PATH: &str = "/Users/nina/.local/bin:/usr/bin";

    #[test]
    fn a_pane_gets_the_program_by_its_full_path_and_revus_own_path() {
        let script = Script::default().with_path(PATH);
        let call = pane_call(&script, &Call::new(&["claude", "attach", "ab12cd34"]).in_dir(Path::new("/code/widgets"))).unwrap();
        assert_eq!(call.argv, ["/Users/nina/.local/bin/claude", "attach", "ab12cd34"]);
        assert_eq!(call.path.as_deref(), Some(PATH));
        assert_eq!(
            call.words(),
            ["/usr/bin/env", "PATH=/Users/nina/.local/bin:/usr/bin", "/Users/nina/.local/bin/claude", "attach", "ab12cd34"]
        );
        assert_eq!(call.dir.as_deref(), Some(Path::new("/code/widgets")));
        let no_path = pane_call(&Script::default(), &Call::new(&["/opt/bin/aider", "--restore"])).unwrap();
        assert_eq!(no_path.words(), ["/opt/bin/aider", "--restore"]);
    }

    #[test]
    fn every_screen_that_opens_a_pane_runs_the_located_program_with_the_path() {
        let program = ["/usr/bin/env", "PATH=/Users/nina/.local/bin:/usr/bin", "/Users/nina/.local/bin/claude", "attach", "ab12cd34"];
        let quoted = "/usr/bin/env 'PATH=/Users/nina/.local/bin:/usr/bin' /Users/nina/.local/bin/claude attach ab12cd34";
        let tmux = Script::default().with_path(PATH).says("tmux split-window", "%5\n");
        place(Screen::Tmux, AgentShow::Split).put(&tmux, None, &ready(), None, "!42").unwrap();
        assert_eq!(tmux.calls()[1].argv[6..], program);
        let ghostty = Script::default().with_path(PATH).says("osascript", "B2\n");
        self::ghostty(AgentShow::Split).put(&ghostty, Some("A1"), &ready(), None, "!42").unwrap();
        let typed = format!(" {CLEAR}; exec /Users/nina/.local/bin/claude attach ab12cd34");
        assert_eq!(ghostty.calls()[0].argv[3..], ["A1", typed.as_str(), "/code/my widgets"], "the shell of the split has its own PATH");
        let iterm = Script::default().with_path(PATH).says("osascript", "9F\n");
        place(Screen::Terminal(Terminal::ITerm { session: "5A".into() }), AgentShow::Split)
            .put(&iterm, None, &ready(), None, "!42")
            .unwrap();
        assert_eq!(iterm.calls()[0].argv[3..], ["5A", quoted]);
        let wezterm = Script::default().with_path(PATH).says("wezterm", "7\n");
        let screen = Screen::Terminal(Terminal::WezTerm { program: "wezterm".into() });
        place(screen, AgentShow::Split).put(&wezterm, None, &ready(), None, "!42").unwrap();
        assert_eq!(wezterm.calls()[0].argv[7..], program);
        let custom = Script::default().with_path(PATH);
        place(Screen::Command(KITTY.into()), AgentShow::Split).put(&custom, None, &ready(), None, "!42").unwrap();
        assert_eq!(custom.calls()[0].argv[6..], program);
    }

    #[test]
    fn a_program_that_is_not_on_the_path_opens_no_pane_and_is_named() {
        for screen in [Screen::Tmux, Screen::Terminal(Terminal::Ghostty), Screen::Command(KITTY.into())] {
            let script = Script::default().with_path(PATH).without("claude");
            let err = place(screen, AgentShow::Split).put(&script, Some("A1"), &ready(), None, "!42").unwrap_err().to_string();
            assert_eq!(err, "no session shown: `claude` is not on revu's PATH");
            assert_eq!(script.lines(), [] as [String; 0]);
        }
        let plain = Script::default().with_path(PATH).without("claude");
        assert_eq!(
            place(Screen::Plain, AgentShow::Split).put(&plain, None, &ready(), None, "!42").unwrap(),
            lent(SAID),
            "lent, the terminal is revu's own"
        );
    }

    #[test]
    fn a_split_command_is_checked_when_the_config_loads() {
        assert!(check_split(KITTY, "agent.split").is_ok());
        assert!(check_split("zellij run --direction right --cwd {dir} -- {command}", "agent.split").is_ok());
        let glued = check_split("kitty @ launch --cmd={command}", "agent.split").unwrap_err().to_string();
        assert!(glued.contains("agent.split") && glued.contains("word of its own"), "{glued}");
        let typo = check_split("kitty @ launch --cwd {folder} {command}", "agent.split").unwrap_err().to_string();
        assert!(typo.contains("{folder}") && typo.contains("{dir}"), "{typo}");
        assert!(check_split("{command}", "agent.split").unwrap_err().to_string().contains("program"));
        assert!(check_split("kitty 'open {command}", "agent.split").is_err());
    }
}
