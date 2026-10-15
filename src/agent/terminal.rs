//! Terminals that split by themselves: Ghostty and iTerm2 through AppleScript, WezTerm through its own
//! command. Every value goes to `osascript` as an argument, never into the script, so a title or a
//! folder cannot run AppleScript.
use super::{Call, Run};
use anyhow::{Context, Result, bail};
use std::path::Path;

/// The terminal Ghostty has in front: revu's own at the moment `a g` is pressed.
const GHOSTTY_FRONT: &str = r#"tell application "Ghostty" to return id of focused terminal of selected tab of front window"#;

/// Arguments: the terminal to split, the line its shell types, its folder. Prints the new terminal's id.
/// The split starts the reader's shell, which becomes the session: given as `command`, Ghostty would
/// keep the split open after the session exits, until one more key.
const GHOSTTY_SPLIT: &str = r#"on run argv
  tell application "Ghostty"
    set revu to first terminal whose id is (item 1 of argv)
    set settings to new surface configuration
    set initial input of settings to (item 2 of argv) & linefeed
    set initial working directory of settings to (item 3 of argv)
    set made to split revu direction right with configuration settings
    focus made
    return id of made
  end tell
end run"#;

/// Argument: a terminal's id. Prints `focused`, or `gone` when Ghostty no longer has it.
const GHOSTTY_FOCUS: &str = r#"on run argv
  tell application "Ghostty"
    set found to every terminal whose id is (item 1 of argv)
    if (count of found) is 0 then return "gone"
    focus (item 1 of found)
    return "focused"
  end tell
end run"#;

/// Arguments: revu's own session, the command. Prints the new session's id.
const ITERM_SPLIT: &str = r#"on run argv
  tell application "iTerm2"
    repeat with aWindow in windows
      repeat with aTab in tabs of aWindow
        repeat with aSession in sessions of aTab
          if id of aSession is (item 1 of argv) then
            tell aSession to set made to (split vertically with same profile command (item 2 of argv))
            tell made to select
            return id of made
          end if
        end repeat
      end repeat
    end repeat
    error "the session revu runs in was not found"
  end tell
end run"#;

/// Argument: a session's id. Prints `focused`, or `gone` when iTerm2 no longer has it.
const ITERM_FOCUS: &str = r#"on run argv
  tell application "iTerm2"
    repeat with aWindow in windows
      repeat with aTab in tabs of aWindow
        repeat with aSession in sessions of aTab
          if id of aSession is (item 1 of argv) then
            tell aSession to select
            return "focused"
          end if
        end repeat
      end repeat
    end repeat
    return "gone"
  end tell
end run"#;

const FOCUSED: &str = "focused";

/// Wipes the screen, then the scrollback, so the line typed into the split and its echoes leave nothing
/// behind. In that order: some terminals push a cleared screen into the scrollback.
pub(super) const CLEAR: &str = r"printf '\033[H\033[2J\033[3J'";

/// A terminal revu knows how to split, with what it needs to find revu's own pane in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Terminal {
    /// Its panes carry no id revu can read from inside: revu's own is the one in front when `a g` is pressed.
    Ghostty,
    /// `session` is the id of the session revu runs in.
    ITerm { session: String },
    /// `program` is the `wezterm` command; it finds revu's own pane from `WEZTERM_PANE` by itself.
    WezTerm { program: String },
}

impl Terminal {
    /// The terminal revu runs in, from what it wrote in the environment. `TERM_PROGRAM` decides: a
    /// terminal started from another one inherits that one's other variables, never this one.
    pub fn detect(env: impl Fn(&str) -> Option<String>) -> Option<Self> {
        match env("TERM_PROGRAM")?.as_str() {
            "ghostty" => Some(Terminal::Ghostty),
            "iTerm.app" => {
                let named = env("ITERM_SESSION_ID")?;
                Some(Terminal::ITerm { session: named.rsplit(':').next()?.to_owned() })
            }
            "WezTerm" => {
                env("WEZTERM_PANE")?;
                let program = env("WEZTERM_EXECUTABLE_DIR").map_or_else(|| "wezterm".to_owned(), |folder| format!("{folder}/wezterm"));
                Some(Terminal::WezTerm { program })
            }
            _ => None,
        }
    }

    pub(super) fn name(&self) -> &'static str {
        match self {
            Terminal::Ghostty => "Ghostty",
            Terminal::ITerm { .. } => "iTerm2",
            Terminal::WezTerm { .. } => "WezTerm",
        }
    }

    /// revu's own pane, asked before the slow work so the reader has no time to look elsewhere; only
    /// Ghostty needs asking.
    pub(super) fn origin(&self, run: &dyn Run) -> Option<String> {
        match self {
            Terminal::Ghostty => run.output(&osascript(GHOSTTY_FRONT, &[])).ok().map(|id| id.trim().to_owned()),
            Terminal::ITerm { .. } | Terminal::WezTerm { .. } => None,
        }
    }

    /// Runs `call` in a new pane on the right of revu's, in `dir`, and gives the pane's id.
    pub(super) fn split(&self, run: &dyn Run, origin: Option<&str>, call: &Call, dir: &Path) -> Result<String> {
        let folder = dir.to_string_lossy();
        let words = call.words();
        let made = match self {
            Terminal::Ghostty => {
                let revu = origin.context("Ghostty did not say which terminal revu runs in")?;
                run.output(&osascript(GHOSTTY_SPLIT, &[revu, &typed_line(call), &folder]))?
            }
            Terminal::ITerm { session } => run.output(&osascript(ITERM_SPLIT, &[session, &in_folder(call, &shell_words::join(&words))]))?,
            Terminal::WezTerm { program } => {
                let words: Vec<&str> = words.iter().map(String::as_str).collect();
                run.output(&Call::new(
                    &[&[program.as_str(), "cli", "split-pane", "--right", "--cwd", &folder, "--"], words.as_slice()].concat(),
                ))?
            }
        };
        let pane = made.trim();
        if pane.is_empty() {
            bail!("{} gave no pane", self.name());
        }
        Ok(pane.to_owned())
    }

    /// Gives the keys to `pane`; `false` when the terminal no longer has it, or does not answer.
    pub(super) fn focus(&self, run: &dyn Run, pane: &str) -> bool {
        match self {
            Terminal::Ghostty => run.output(&osascript(GHOSTTY_FOCUS, &[pane])).is_ok_and(|said| said.trim() == FOCUSED),
            Terminal::ITerm { .. } => run.output(&osascript(ITERM_FOCUS, &[pane])).is_ok_and(|said| said.trim() == FOCUSED),
            Terminal::WezTerm { program } => {
                let listed = run.output(&Call::new(&[program.as_str(), "cli", "list", "--format", "json"])).unwrap_or_default();
                has_pane(&listed, pane) && run.output(&Call::new(&[program.as_str(), "cli", "activate-pane", "--pane-id", pane])).is_ok()
            }
        }
    }
}

fn osascript(script: &str, arguments: &[&str]) -> Call {
    Call::new(&[&["osascript", "-e", script], arguments].concat())
}

/// What the shell of a Ghostty split types: a leading space, kept out of the history by a shell that
/// ignores such lines, then the screen wiped and the shell replaced by the program. The shell is the
/// reader's own login shell, which sets its own `PATH`; the program keeps its full path.
fn typed_line(call: &Call) -> String {
    format!(" {CLEAR}; exec {}", shell_words::join(&call.argv))
}

/// iTerm2 has no folder to give a split: a program that needs one is started from a shell that goes there first.
fn in_folder(call: &Call, command: &str) -> String {
    let Some(dir) = &call.dir else { return command.to_owned() };
    let script = format!("cd {} && exec {command}", shell_words::quote(&dir.to_string_lossy()));
    shell_words::join(["/bin/sh", "-c", script.as_str()])
}

/// Whether `pane` is one of the panes `wezterm cli list --format json` printed.
fn has_pane(listed: &str, pane: &str) -> bool {
    let panes: Vec<serde_json::Value> = serde_json::from_str(listed).unwrap_or_default();
    panes.iter().filter_map(|listed| listed.get("pane_id")?.as_u64()).any(|id| id.to_string() == pane)
}

#[cfg(test)]
mod tests {
    use super::super::tests::Script;
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| pairs.iter().find(|(key, _)| *key == name).map(|(_, value)| (*value).to_owned())
    }

    fn attach() -> Call {
        Call::new(&["claude", "attach", "ab12cd34"])
    }

    #[test]
    fn the_terminal_is_read_from_term_program_with_the_id_each_one_needs() {
        assert_eq!(Terminal::detect(env(&[("TERM_PROGRAM", "ghostty")])), Some(Terminal::Ghostty));
        let iterm = [("TERM_PROGRAM", "iTerm.app"), ("ITERM_SESSION_ID", "w0t1p0:5A1B-GUID")];
        assert_eq!(Terminal::detect(env(&iterm)), Some(Terminal::ITerm { session: "5A1B-GUID".into() }));
        let wezterm =
            [("TERM_PROGRAM", "WezTerm"), ("WEZTERM_PANE", "3"), ("WEZTERM_EXECUTABLE_DIR", "/Applications/WezTerm.app/Contents/MacOS")];
        let program = "/Applications/WezTerm.app/Contents/MacOS/wezterm".to_owned();
        assert_eq!(Terminal::detect(env(&wezterm)), Some(Terminal::WezTerm { program }));
        assert_eq!(Terminal::detect(env(&wezterm[..2])), Some(Terminal::WezTerm { program: "wezterm".into() }));
    }

    #[test]
    fn an_unknown_terminal_or_one_without_its_id_is_not_recognised() {
        assert_eq!(Terminal::detect(env(&[])), None);
        assert_eq!(Terminal::detect(env(&[("TERM_PROGRAM", "Apple_Terminal")])), None);
        assert_eq!(Terminal::detect(env(&[("TERM_PROGRAM", "iTerm.app")])), None);
        assert_eq!(Terminal::detect(env(&[("TERM_PROGRAM", "WezTerm")])), None);
        let inherited = [("TERM_PROGRAM", "Apple_Terminal"), ("GHOSTTY_RESOURCES_DIR", "/Applications/Ghostty.app"), ("WEZTERM_PANE", "3")];
        assert_eq!(Terminal::detect(env(&inherited)), None, "variables another terminal left behind do not count");
    }

    #[test]
    fn ghostty_splits_the_terminal_that_was_in_front_and_takes_every_value_as_an_argument() {
        let script =
            Script::default().says("osascript -e tell application \"Ghostty\" to return id", "A1-FRONT\n").says("osascript", "B2-NEW\n");
        let origin = Terminal::Ghostty.origin(&script).unwrap();
        assert_eq!(origin, "A1-FRONT");
        let call = Call::new(&["aider", "--message", "it's \"quoted\" $HOME"]);
        let pane = Terminal::Ghostty.split(&script, Some(&origin), &call, Path::new("/code/my widgets")).unwrap();
        assert_eq!(pane, "B2-NEW");
        let calls = script.calls();
        assert_eq!(calls[0].argv, ["osascript", "-e", GHOSTTY_FRONT]);
        let typed = r#" printf '\033[H\033[2J\033[3J'; exec aider --message 'it'\''s "quoted" $HOME'"#;
        assert_eq!(calls[1].argv, ["osascript", "-e", GHOSTTY_SPLIT, "A1-FRONT", typed, "/code/my widgets"]);
        assert!(!GHOSTTY_SPLIT.contains("aider") && GHOSTTY_SPLIT.contains("split revu direction right with configuration settings"));
        assert!(
            GHOSTTY_SPLIT.contains("\n    set initial input of settings to (item 2 of argv) & linefeed\n"),
            "the shell becomes the session"
        );
        assert!(!GHOSTTY_SPLIT.contains("set command") && !GHOSTTY_SPLIT.contains("wait after command"));
        let with_path =
            Call { path: Some("/a very/long:/path".into()), ..Call::new(&["/Users/nina/.local/bin/claude", "attach", "ab12cd34"]) };
        Terminal::Ghostty.split(&script, Some(&origin), &with_path, Path::new("/code")).unwrap();
        assert_eq!(
            script.calls()[2].argv[4],
            r" printf '\033[H\033[2J\033[3J'; exec /Users/nina/.local/bin/claude attach ab12cd34",
            "no PATH typed"
        );
    }

    #[test]
    fn ghostty_without_its_own_terminal_or_with_no_answer_fails_with_a_reason() {
        let script = Script::default();
        let unknown = Terminal::Ghostty.split(&script, None, &attach(), Path::new("/code/widgets")).unwrap_err().to_string();
        assert!(unknown.contains("which terminal revu runs in"), "{unknown}");
        assert_eq!(script.lines(), [] as [String; 0]);
        let silent = Terminal::Ghostty.split(&script, Some("A1"), &attach(), Path::new("/code/widgets")).unwrap_err().to_string();
        assert_eq!(silent, "Ghostty gave no pane");
        let refused =
            Script::default().fails("osascript", "`osascript -e` failed: Not authorized to send Apple events to Ghostty. (-1743)");
        assert_eq!(Terminal::Ghostty.origin(&refused), None);
    }

    #[test]
    fn iterm_splits_its_own_session_and_goes_to_the_folder_through_a_shell() {
        let iterm = Terminal::ITerm { session: "5A1B-GUID".into() };
        let script = Script::default().says("osascript", "9F-NEW\n");
        assert_eq!(iterm.origin(&script), None);
        assert_eq!(iterm.split(&script, None, &attach(), Path::new("/code/widgets")).unwrap(), "9F-NEW");
        assert_eq!(script.calls()[0].argv, ["osascript", "-e", ITERM_SPLIT, "5A1B-GUID", "claude attach ab12cd34"]);
        let started = Call::new(&["aider", "--message", "it's here"]).in_dir(Path::new("/code/my widgets"));
        iterm.split(&script, None, &started, Path::new("/code/my widgets")).unwrap();
        let words = shell_words::split(&script.calls()[1].argv[4]).unwrap();
        assert_eq!(words, ["/bin/sh", "-c", r"cd '/code/my widgets' && exec aider --message 'it'\''s here'"]);
    }

    #[test]
    fn wezterm_splits_to_the_right_in_the_folder_with_the_program_word_by_word() {
        let wezterm = Terminal::WezTerm { program: "wezterm".into() };
        let script = Script::default().says("wezterm cli split-pane", "7\n");
        let call = Call::new(&["aider", "--message", "it's here"]);
        assert_eq!(wezterm.split(&script, None, &call, Path::new("/code/my widgets")).unwrap(), "7");
        let wanted = ["wezterm", "cli", "split-pane", "--right", "--cwd", "/code/my widgets", "--", "aider", "--message", "it's here"];
        assert_eq!(script.calls()[0].argv, wanted);
    }

    #[test]
    fn a_remembered_pane_gets_the_keys_and_one_that_is_gone_says_so() {
        let here = Script::default().says("osascript", "focused\n");
        assert!(Terminal::Ghostty.focus(&here, "B2"));
        assert_eq!(here.calls()[0].argv, ["osascript", "-e", GHOSTTY_FOCUS, "B2"]);
        let iterm = Terminal::ITerm { session: "5A1B".into() };
        assert!(iterm.focus(&here, "9F"));
        assert_eq!(here.calls()[1].argv, ["osascript", "-e", ITERM_FOCUS, "9F"]);
        let gone = Script::default().says("osascript", "gone\n");
        assert!(!Terminal::Ghostty.focus(&gone, "B2") && !iterm.focus(&gone, "9F"));
        let refused = Script::default().fails("osascript", "not authorized");
        assert!(!Terminal::Ghostty.focus(&refused, "B2"));
    }

    #[test]
    fn wezterm_focuses_a_pane_it_still_lists_and_only_that() {
        let wezterm = Terminal::WezTerm { program: "wezterm".into() };
        let script = Script::default().says("wezterm cli list", r#"[{"window_id": 0, "pane_id": 3}, {"pane_id": 7, "title": "claude"}]"#);
        assert!(wezterm.focus(&script, "7"));
        assert_eq!(script.lines(), ["wezterm cli list --format json", "wezterm cli activate-pane --pane-id 7"]);
        let script = Script::default().says("wezterm cli list", r#"[{"pane_id": 3}, {"pane_id": 70}]"#);
        assert!(!wezterm.focus(&script, "7"));
        assert_eq!(script.lines(), ["wezterm cli list --format json"]);
        assert!(!wezterm.focus(&Script::default().fails("wezterm cli list", "no server"), "7"));
    }
}
