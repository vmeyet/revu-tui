//! The palette, `ctrl-k` or `⌘k`: one box that finds MRs, the open MR's files, or runs commands,
//! picked by the first character as VS Code does. Nothing typed finds MRs (`@author`, `!42`,
//! `~label` narrow them, see `query`), `/` finds files, `>` runs commands. `:` opens it on
//! commands, with fuzzy tab completion, a ghost suggestion and history.
use super::complete::{self, Cycle};
use crate::forge::MrKey;
use crate::fuzzy;

/// What the palette searches, from the first character typed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Mrs,
    Files,
    Commands,
}

impl Mode {
    /// The character that switches to this mode, and the word the prompt shows.
    pub fn prompt(self) -> (&'static str, &'static str) {
        match self {
            Mode::Mrs => ("", "MRs · @author !42 ~label draft:no"),
            Mode::Files => ("/", "files of the open MR"),
            Mode::Commands => (">", "commands"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Mr(MrKey),
    /// A file of the open MR, by its index in the review.
    File(usize),
}

/// One row of the palette's list: what it shows, and where `enter` goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub label: String,
    /// Dim text after the label: the author of an MR.
    pub detail: String,
    pub target: Target,
}

pub const MAX_SHOWN: usize = 12;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Open an MR: `!42`, `#42`, `42`, `group/project!42`, `owner/repo#42`.
    Go(String),
    Open,
    Approve,
    Publish,
    /// This repo only, or every project.
    All,
    Set {
        key: String,
        value: String,
    },
    /// The file in the reader's program: nothing for the cursor's file, `old` for its base version, or `path[:line]`.
    View(String),
    /// `:ai off`: no more AI calls this session.
    AiOff,
    /// `:ai on`: the providers the config switched on answer again.
    AiOn,
    /// `:ask <question>`: a free question to Claude about what is under the cursor.
    Ask(String),
    Help,
    Quit,
}

pub const VERBS: [(&str, &str); 11] = [
    ("go", "open an MR: :go !42 · :go acme/widgets!42"),
    ("open", "open the MR, or the line, in the browser"),
    ("approve", "approve the open MR, or take the approval back"),
    ("publish", "publish every draft, in the publish modal"),
    ("all", "the queue: this repo only, or every project"),
    ("set", "change and save a setting: :set theme=nord"),
    ("view", "the file in your program: :view · :view old · :view src/a.rs:42"),
    ("ask", "ask Claude about the cursor's hunk, file or the MR: :ask is this thread-safe?"),
    ("ai", "switch AI off or back on for this session: :ai off, :ai on"),
    ("help", "show the keys"),
    ("quit", "leave"),
];

pub fn parse(line: &str) -> Result<Command, String> {
    let line = line.trim().trim_start_matches(':').trim();
    let (verb, rest) = line.split_once(char::is_whitespace).map_or((line, ""), |(v, r)| (v, r.trim()));
    match verb {
        "" => Err("type a command, tab completes".into()),
        "go" | "g" if rest.is_empty() => Err(":go needs an MR, like !42".into()),
        "go" | "g" => Ok(Command::Go(rest.to_owned())),
        "open" | "o" => Ok(Command::Open),
        "approve" | "a" => Ok(Command::Approve),
        "publish" | "p" => Ok(Command::Publish),
        "all" | "*" => Ok(Command::All),
        "set" => {
            let Some((key, value)) = rest.split_once('=') else { return Err(":set needs key=value, e.g. theme=nord".into()) };
            Ok(Command::Set { key: key.trim().to_owned(), value: value.trim().to_owned() })
        }
        "view" | "v" => Ok(Command::View(rest.to_owned())),
        "ai" if rest == "off" => Ok(Command::AiOff),
        "ai" if rest == "on" => Ok(Command::AiOn),
        "ai" => Err(":ai off or :ai on; the config says which providers exist".into()),
        "ask" if rest.is_empty() => Err(":ask needs a question".into()),
        "ask" => Ok(Command::Ask(rest.to_owned())),
        "help" | "h" | "?" => Ok(Command::Help),
        "quit" | "q" | "exit" => Ok(Command::Quit),
        unknown => {
            let close = fuzzy::suggestions(unknown, VERBS.iter().map(|(v, _)| *v), 3);
            if close.is_empty() {
                Err(format!("unknown command :{unknown}"))
            } else {
                Err(format!("unknown command :{unknown}, did you mean :{}?", close.join(", :")))
            }
        }
    }
}

/// What the token under the cursor can complete to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Verb,
    Mr,
    Setting,
    Theme,
    /// `old` or a file of the open MR.
    File,
    Free,
}

pub fn slot(line: &str) -> Slot {
    let line = line.trim_start_matches(':');
    let mut tokens = line.split_whitespace();
    let verb = tokens.next().unwrap_or("");
    let done = tokens.count();
    if !line.contains(char::is_whitespace) {
        return Slot::Verb;
    }
    let arg = if line.ends_with(char::is_whitespace) { done } else { done.saturating_sub(1) };
    match (verb, arg) {
        ("go" | "g", 0) => Slot::Mr,
        ("set", 0) if line.contains("theme=") => Slot::Theme,
        ("set", 0) => Slot::Setting,
        ("view" | "v", 0) => Slot::File,
        _ => Slot::Free,
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Palette {
    pub mode: Mode,
    /// What is typed after the mode's character.
    pub input: String,
    /// The highlighted row of the list, for MRs and files.
    pub selected: usize,
    pub history: Vec<String>,
    history_at: Option<usize>,
    cycle: Option<Cycling>,
}

/// A cycle over the last token, and the line before it that stays put.
#[derive(Clone, Debug, PartialEq)]
struct Cycling {
    prefix: String,
    cycle: Cycle,
}

impl Palette {
    pub fn new(mode: Mode, history: Vec<String>) -> Self {
        Self { mode, history, ..Self::default() }
    }

    /// `>` or `/` as the very first character switches the mode instead of being typed.
    pub fn type_char(&mut self, c: char) {
        match (self.mode, self.input.is_empty(), c) {
            (Mode::Mrs, true, '>') => self.mode = Mode::Commands,
            (Mode::Mrs, true, '/') => self.mode = Mode::Files,
            _ => self.input.push(c),
        }
        self.selected = 0;
        self.cycle = None;
    }

    /// Erases a character; on an empty line it steps back to MRs first. `false` once there is
    /// nothing left to erase, which closes the palette.
    pub fn backspace(&mut self) -> bool {
        self.selected = 0;
        self.cycle = None;
        if self.input.pop().is_some() {
            return true;
        }
        let stepped = self.mode != Mode::Mrs;
        self.mode = Mode::Mrs;
        stepped
    }

    /// Moves the highlight in a list of `len` rows, staying inside it.
    pub fn move_by(&mut self, delta: isize, len: usize) {
        self.selected = if len == 0 { 0 } else { self.selected.saturating_add_signed(delta).min(len - 1) };
    }

    /// Replaces the token being typed with the next candidate for its slot.
    pub fn complete(&mut self, candidates: &[String], backwards: bool) {
        let cycling = if let Some(mut cycling) = self.cycle.take() {
            cycling.cycle.advance(backwards);
            cycling
        } else {
            let (prefix, token) = split_last_token(&self.input);
            let Some(cycle) = Cycle::new(token, candidates) else { return };
            Cycling { prefix: prefix.to_owned(), cycle }
        };
        let chosen = cycling.cycle.current();
        let trailing = if chosen.ends_with('=') { "" } else { " " };
        self.input = format!("{}{chosen}{trailing}", cycling.prefix);
        self.cycle = Some(cycling);
    }

    /// The grey rest of the best completion for the token being typed, as a shell shows it.
    pub fn ghost(&self, candidates: &[String]) -> Option<String> {
        if self.cycle.is_some() {
            return None;
        }
        let (_, token) = split_last_token(&self.input);
        complete::ghost(token, candidates)
    }

    /// Accepts the ghost text, as `→` does in a shell.
    pub fn accept(&mut self, candidates: &[String]) {
        if let Some(rest) = self.ghost(candidates) {
            self.input.push_str(&rest);
            if !self.input.ends_with('=') {
                self.input.push(' ');
            }
        }
    }

    pub fn hint(&self) -> Option<String> {
        Some(self.cycle.as_ref()?.cycle.hint(str::to_owned))
    }

    pub fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let at = self.history_at.map_or(self.history.len() - 1, |i| i.saturating_sub(1));
        self.history_at = Some(at);
        self.input.clone_from(&self.history[at]);
        self.cycle = None;
    }

    pub fn history_down(&mut self) {
        let Some(at) = self.history_at else { return };
        if at + 1 >= self.history.len() {
            self.history_at = None;
            self.input.clear();
        } else {
            self.history_at = Some(at + 1);
            self.input.clone_from(&self.history[at + 1]);
        }
        self.cycle = None;
    }

    /// Takes the line, remembers it, and leaves the palette empty for the next command.
    pub fn submit(&mut self) -> String {
        let line = std::mem::take(&mut self.input).trim().to_owned();
        if !line.is_empty() && self.history.last() != Some(&line) {
            self.history.push(line.clone());
        }
        self.history_at = None;
        self.cycle = None;
        line
    }
}

fn split_last_token(input: &str) -> (&str, &str) {
    match input.rfind(char::is_whitespace) {
        Some(i) => (&input[..=i], &input[i + 1..]),
        None => ("", input),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn typed(text: &str) -> Palette {
        let mut p = Palette::new(Mode::Commands, vec![]);
        text.chars().for_each(|c| p.type_char(c));
        p
    }

    #[test]
    fn parses_verbs_and_aliases() {
        assert_eq!(parse(":go !42"), Ok(Command::Go("!42".into())));
        assert_eq!(parse("g acme/widgets!42"), Ok(Command::Go("acme/widgets!42".into())));
        assert_eq!(parse("set theme=nord"), Ok(Command::Set { key: "theme".into(), value: "nord".into() }));
        assert_eq!(parse("*"), Ok(Command::All));
        assert_eq!(parse("q"), Ok(Command::Quit));
        assert_eq!(parse("view old"), Ok(Command::View("old".into())));
        assert_eq!(parse("v src/a.rs:42"), Ok(Command::View("src/a.rs:42".into())));
        assert_eq!(parse("view"), Ok(Command::View(String::new())));
    }

    #[test]
    fn errors_are_helpful() {
        assert_eq!(parse("go"), Err(":go needs an MR, like !42".into()));
        assert!(parse("aprove").unwrap_err().contains("did you mean :approve"));
        assert_eq!(parse("set theme"), Err(":set needs key=value, e.g. theme=nord".into()));
    }

    #[test]
    fn slots_follow_the_verb() {
        assert_eq!(slot("pu"), Slot::Verb);
        assert_eq!(slot("go "), Slot::Mr);
        assert_eq!(slot("set "), Slot::Setting);
        assert_eq!(slot("set theme=to"), Slot::Theme);
        assert_eq!(slot("go !42 more"), Slot::Free);
    }

    #[test]
    fn tab_cycles_and_the_ghost_suggests() {
        let verbs: Vec<String> = VERBS.iter().map(|(v, _)| (*v).to_owned()).collect();
        let mut p = typed("pu");
        assert_eq!(p.ghost(&verbs).as_deref(), Some("blish"));
        p.accept(&verbs);
        assert_eq!(p.input, "publish ");
        let mrs = vec!["!42".to_owned(), "!41".to_owned()];
        let mut p = typed("go 4");
        p.complete(&mrs, false);
        assert_eq!(p.input, "go !42 ");
        assert!(p.hint().unwrap().starts_with("[!42]"));
        p.complete(&mrs, false);
        assert_eq!(p.input, "go !41 ");
    }

    #[test]
    fn the_first_character_picks_the_mode_and_backspace_steps_back() {
        let mut p = Palette::default();
        p.type_char('>');
        assert_eq!((p.mode, p.input.as_str()), (Mode::Commands, ""));
        p.type_char('>');
        assert_eq!(p.input, ">", "only the first character switches");
        assert!(p.backspace() && p.backspace());
        assert_eq!(p.mode, Mode::Mrs, "an empty line steps back to MRs");
        assert!(!p.backspace(), "nothing left: the palette closes");
        p.type_char('/');
        assert_eq!(p.mode, Mode::Files);
        let mut p = Palette::default();
        p.type_char('@');
        assert_eq!((p.mode, p.input.as_str()), (Mode::Mrs, "@"), "@ ! # ~ narrow MRs, they are no modes");
    }

    #[test]
    fn the_highlight_stays_inside_the_list() {
        let mut p = Palette::default();
        p.move_by(5, 3);
        assert_eq!(p.selected, 2);
        p.move_by(-9, 3);
        assert_eq!(p.selected, 0);
        p.move_by(1, 0);
        assert_eq!(p.selected, 0);
    }

    #[test]
    fn history_recall() {
        let mut p = typed("go !1");
        assert_eq!(p.submit(), "go !1");
        p.input = "all".into();
        p.submit();
        p.history_up();
        assert_eq!(p.input, "all");
        p.history_up();
        assert_eq!(p.input, "go !1");
        p.history_down();
        p.history_down();
        assert_eq!(p.input, "");
    }
}
