//! The palette, `ctrl-k` or `⌘k`: one box that finds MRs or runs commands, picked by the first
//! character as VS Code does. Nothing typed finds MRs (`@author`, `!42`, `~label` narrow them, see
//! `query`), `>` runs commands. `:` opens it on commands, with fuzzy tab completion, a ghost
//! suggestion and history.
use crate::act::{self, Act, Verb};
use crate::forge::MrKey;
use vtui::fuzzy;

/// What the palette searches, from the first character typed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Mode {
    #[default]
    Mrs,
    Commands,
    /// The answers Claude gave before on the open MR, `a h`.
    Answers,
    /// The agent sessions that could be an MR's, `a g`.
    Sessions,
}

impl vtui::palette::Mode for Mode {
    fn switched_by(self, first: char) -> Option<Self> {
        (self == Mode::Mrs && first == '>').then_some(Mode::Commands)
    }

    fn takes_commands(self) -> bool {
        self == Mode::Commands
    }
}

/// The palette over revu's modes.
pub(crate) type Palette = vtui::palette::Palette<Mode>;

impl Mode {
    /// The character that switches to this mode, and the word the prompt shows.
    pub(crate) fn prompt(self) -> (&'static str, &'static str) {
        match self {
            Mode::Mrs => ("", "MRs · @author !42 ~label draft:no"),
            Mode::Commands => (">", "commands"),
            Mode::Answers => ("", "answers kept for this MR"),
            Mode::Sessions => ("", "sessions of this checkout"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Mr(MrKey),
    /// A kept answer, by its index in the list `a h` found.
    Answer(usize),
    /// An agent session, by its index in the list `a g` found; `None` for a new one.
    Session(Option<usize>),
}

/// One row of the palette's list: what it shows, and where `enter` goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub(crate) label: String,
    /// Dim text after the label: the author of an MR.
    pub(crate) detail: String,
    pub(crate) target: Target,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    /// Open an MR: `!42`, `#42`, `42`, `group/project!42`, `owner/repo#42`.
    Go(String),
    Open,
    /// `:copy`: the link `:open` would show, on the clipboard.
    Copy,
    /// `:merge`: my approved MR, after a yes.
    Merge,
    /// `:ready`: my MR, ready for review when a draft, a draft when ready.
    Ready,
    Approve,
    Publish,
    /// `:threads`: every conversation of the MR in the right pane, as `T` does.
    Threads,
    /// `:pin`, `:unpin`: the MR kept on top of the queue, or let go.
    Pin,
    Unpin,
    /// This repo only, or every project.
    All,
    Set {
        key: String,
        value: String,
    },
    /// The file in the reader's program: nothing for the cursor's file, `old` for its base version, or `path[:line]`.
    View(String),
    /// `:outline`: the symbols the MR changes in the right pane, as `O` does.
    Outline,
    /// `:ai off`: no more AI calls this session.
    AiOff,
    /// `:ai on`: the providers the config switched on answer again.
    AiOn,
    /// `:ask <question>`: a free question to Claude about what is under the cursor.
    Ask(String),
    /// `:share [target]`: post the MR through a `[share]` command, after a preview.
    Share(Option<String>),
    /// `:session`: the agent session of the MR, as `a g` does.
    Session,
    /// `:news`: what changed in revu, version by version.
    News,
    Help,
    Quit,
}

impl Command {
    /// The action the command is counted and listed as.
    pub(crate) fn act(&self) -> Act {
        match self {
            Self::Go(_) => Act::GoCommand,
            Self::Open => Act::OpenCommand,
            Self::Copy => Act::CopyCommand,
            Self::Merge => Act::MergeCommand,
            Self::Ready => Act::ReadyCommand,
            Self::Approve => Act::ApproveCommand,
            Self::Publish => Act::PublishCommand,
            Self::Threads => Act::ThreadsCommand,
            Self::All => Act::AllCommand,
            Self::Pin => Act::PinCommand,
            Self::Unpin => Act::UnpinCommand,
            Self::Set { .. } => Act::SetCommand,
            Self::View(_) => Act::ViewCommand,
            Self::Outline => Act::OutlineCommand,
            Self::AiOff | Self::AiOn => Act::AiCommand,
            Self::Ask(_) => Act::AskCommand,
            Self::Share(_) => Act::ShareCommand,
            Self::Session => Act::SessionCommand,
            Self::News => Act::NewsCommand,
            Self::Help => Act::HelpCommand,
            Self::Quit => Act::QuitCommand,
        }
    }
}

/// revu's commands that match what is typed, best first.
pub(super) fn verbs_for(input: &str) -> Vec<Verb> {
    vtui::palette::verbs_for(input, act::verbs())
}

pub(crate) fn parse(line: &str) -> Result<Command, String> {
    let line = line.trim().trim_start_matches(':').trim();
    let (verb, rest) = line.split_once(char::is_whitespace).map_or((line, ""), |(v, r)| (v, r.trim()));
    match verb {
        "" => Err("type a command, tab completes".into()),
        "go" | "g" if rest.is_empty() => Err(":go needs an MR, like !42".into()),
        "go" | "g" => Ok(Command::Go(rest.to_owned())),
        "open" | "o" => Ok(Command::Open),
        "copy" => Ok(Command::Copy),
        "approve" | "a" => Ok(Command::Approve),
        "merge" => Ok(Command::Merge),
        "ready" => Ok(Command::Ready),
        "publish" | "p" => Ok(Command::Publish),
        "threads" => Ok(Command::Threads),
        "all" | "*" => Ok(Command::All),
        "pin" => Ok(Command::Pin),
        "unpin" => Ok(Command::Unpin),
        "set" => {
            let Some((key, value)) = rest.split_once('=') else { return Err(":set needs key=value, e.g. theme=nord".into()) };
            Ok(Command::Set { key: key.trim().to_owned(), value: value.trim().to_owned() })
        }
        "view" | "v" => Ok(Command::View(rest.to_owned())),
        "outline" => Ok(Command::Outline),
        "ai" if rest == "off" => Ok(Command::AiOff),
        "ai" if rest == "on" => Ok(Command::AiOn),
        "ai" => Err(":ai off or :ai on; the config says which providers exist".into()),
        "ask" if rest.is_empty() => Err(":ask needs a question".into()),
        "ask" => Ok(Command::Ask(rest.to_owned())),
        "share" => Ok(Command::Share((!rest.is_empty()).then(|| rest.to_owned()))),
        "session" => Ok(Command::Session),
        "news" => Ok(Command::News),
        "help" | "h" | "?" => Ok(Command::Help),
        "quit" | "q" | "exit" => Ok(Command::Quit),
        unknown => {
            let close = fuzzy::suggestions(unknown, act::verbs().map(|verb| verb.word), 3);
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
pub(super) enum Slot {
    Verb,
    Mr,
    Setting,
    Theme,
    /// `old` or a file of the open MR.
    File,
    Free,
}

pub(crate) fn slot(line: &str) -> Slot {
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

#[cfg(test)]
mod tests {
    use super::*;
    use vtui::toward::Toward;

    fn typed(text: &str) -> Palette {
        let mut p = Palette::new(Mode::Commands, vec![]);
        text.chars().for_each(|c| p.type_char(c));
        p
    }

    #[test]
    fn a_command_is_the_act_of_its_verb() {
        for verb in act::verbs() {
            let command = ["", " off", " x=y"].iter().find_map(|argument| parse(&format!("{}{argument}", verb.word)).ok()).unwrap();
            assert_eq!(command.act().row().verb, Some(verb), "{}", verb.word);
        }
    }

    #[test]
    fn parses_verbs_and_aliases() {
        assert_eq!(parse(":go !42"), Ok(Command::Go("!42".into())));
        assert_eq!(parse("g acme/widgets!42"), Ok(Command::Go("acme/widgets!42".into())));
        assert_eq!(parse("set theme=nord"), Ok(Command::Set { key: "theme".into(), value: "nord".into() }));
        assert_eq!(parse("*"), Ok(Command::All));
        assert_eq!(parse(":threads"), Ok(Command::Threads));
        assert_eq!(parse(":news"), Ok(Command::News));
        assert_eq!(parse(":session"), Ok(Command::Session));
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
        let verbs: Vec<String> = act::verbs().map(|verb| verb.word.to_owned()).collect();
        let mut p = typed("pu");
        assert_eq!(p.ghost(&verbs).as_deref(), Some("blish"));
        p.accept(&verbs);
        assert_eq!(p.input, "publish ");
        let mrs = vec!["!42".to_owned(), "!41".to_owned()];
        let mut p = typed("go 4");
        p.complete(&mrs, Toward::Next);
        assert_eq!(p.input, "go !42 ");
        assert!(p.hint().unwrap().starts_with("[!42]"));
        p.complete(&mrs, Toward::Next);
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
        assert_eq!((p.mode, p.input.as_str()), (Mode::Mrs, "/"), "a slash is typed like any letter");
        let mut p = Palette::default();
        p.type_char('@');
        assert_eq!((p.mode, p.input.as_str()), (Mode::Mrs, "@"), "@ ! # ~ narrow MRs, they are no modes");
    }

    #[test]
    fn the_command_list_follows_the_typed_verb() {
        assert_eq!(verbs_for("").len(), act::verbs().count().min(vtui::palette::MAX_SHOWN));
        assert_eq!(verbs_for("pub").first().map(|verb| verb.word), Some("publish"));
        assert_eq!(verbs_for("go !42").first().map(|verb| verb.word), Some("go"));
        assert!(typed("pu").naming_command() && !typed("go !4").naming_command());
    }
}
