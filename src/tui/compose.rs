//! Writing a comment in `$EDITOR`: the draft goes to a private temp file, what comes back is kept.
use anyhow::{Context, Result};
use std::path::Path;
use std::process::{Command, ExitStatus};

const FALLBACK: &str = "vi";

/// What the editor gave back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edited {
    /// The file came back untouched or empty: the user backed out.
    Cancelled,
    Done(String),
    /// The text changed but the editor exited with an error, as vim does on `:cq` or after any error in the session.
    Errored(String),
}

/// Opens the draft in the user's editor and gives back what came of it. The temp file is gone either way.
pub fn edit(draft: &str) -> Result<Edited> {
    let file = tempfile::Builder::new().prefix("mr-").suffix(".md").tempfile()?;
    std::fs::write(file.path(), draft)?;
    let status = Editor::from_env().open(file.path())?;
    let edited = std::fs::read_to_string(file.path())?;
    Ok(to_send(draft, &edited, status.success()))
}

fn to_send(draft: &str, edited: &str, editor_ok: bool) -> Edited {
    let text = edited.trim();
    let is_cancelled = text.is_empty() || text == draft.trim();
    match (is_cancelled, editor_ok) {
        (true, _) => Edited::Cancelled,
        (false, true) => Edited::Done(text.to_owned()),
        (false, false) => Edited::Errored(text.to_owned()),
    }
}

/// The editor to open, and the arguments it was configured with, as in `EDITOR="code -w"`.
#[derive(Debug, PartialEq, Eq)]
struct Editor {
    program: String,
    args: Vec<String>,
}

impl Editor {
    fn from_env() -> Self {
        Self::pick(var("VISUAL"), var("EDITOR"))
    }

    fn pick(visual: Option<String>, editor: Option<String>) -> Self {
        let command = visual.or(editor).unwrap_or_default();
        let mut words = command.split_whitespace();
        let program = words.next().unwrap_or(FALLBACK).to_owned();
        Self { program, args: words.map(str::to_owned).collect() }
    }

    fn open(&self, path: &Path) -> Result<ExitStatus> {
        Command::new(&self.program).args(&self.args).arg(path).status().with_context(|| format!("could not run {}", self.program))
    }
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(program: &str, args: &[&str]) -> Editor {
        Editor { program: program.into(), args: args.iter().map(|a| (*a).to_owned()).collect() }
    }

    #[test]
    fn visual_comes_first_then_editor_then_vi() {
        assert_eq!(Editor::pick(Some("emacs".into()), Some("nano".into())), editor("emacs", &[]));
        assert_eq!(Editor::pick(None, Some("nano".into())), editor("nano", &[]));
        assert_eq!(Editor::pick(None, None), editor("vi", &[]));
        assert_eq!(Editor::pick(Some("  ".into()), None), editor("vi", &[]));
    }

    #[test]
    fn an_editor_keeps_the_arguments_it_was_given() {
        assert_eq!(Editor::pick(None, Some("code -w --new-window".into())), editor("code", &["-w", "--new-window"]));
    }

    #[test]
    fn a_changed_file_is_sent_without_its_surrounding_blank() {
        assert_eq!(to_send("", "hello\n", true), Edited::Done("hello".into()));
        assert_eq!(to_send("hi", "\nhi there\n\n", true), Edited::Done("hi there".into()));
        assert_eq!(to_send("a", "one\ntwo", true), Edited::Done("one\ntwo".into()));
    }

    #[test]
    fn a_changed_file_is_kept_when_the_editor_exits_with_an_error() {
        assert_eq!(to_send("", "hello\n", false), Edited::Errored("hello".into()));
    }

    #[test]
    fn an_empty_or_untouched_file_cancels_whatever_the_editor_exit() {
        for editor_ok in [true, false] {
            assert_eq!(to_send("", "  \n ", editor_ok), Edited::Cancelled);
            assert_eq!(to_send("hi", "", editor_ok), Edited::Cancelled);
            assert_eq!(to_send("hi", "hi\n", editor_ok), Edited::Cancelled);
        }
    }
}
