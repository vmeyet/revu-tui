//! The `:` command line and the `ctrl-k` jump: typing a verb or a name instead of walking to it.
use super::{Action, App, Focus};
use crate::forge::MrKey;
use crate::review::Row;
use crate::tui::jump::{Candidate, Jump, Target};
use crate::tui::palette::{self, Command, Palette, Slot};
use crate::tui::theme::Theme;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

impl App {
    pub(super) fn open_palette(&mut self) {
        self.palette = Some(Palette::with_history(self.palette_history.clone()));
    }

    pub(super) fn handle_palette_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let Some(mut palette) = self.palette.take() else { return vec![] };
        let candidates = self.completions_for(&palette.input);
        match key.code {
            KeyCode::Esc => return vec![],
            KeyCode::Backspace if palette.input.is_empty() => return vec![],
            KeyCode::Backspace => palette.backspace(),
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => palette.type_char(c),
            KeyCode::Tab | KeyCode::BackTab => palette.complete(&candidates, key.code == KeyCode::BackTab),
            KeyCode::Right | KeyCode::End => palette.accept(&candidates),
            KeyCode::Up => palette.history_up(),
            KeyCode::Down => palette.history_down(),
            KeyCode::Enter => {
                let line = palette.submit();
                self.palette_history.clone_from(&palette.history);
                return match palette::parse(&line) {
                    Ok(command) => self.run_command(command),
                    Err(reason) => {
                        self.warn(reason);
                        vec![]
                    }
                };
            }
            _ => {}
        }
        self.palette = Some(palette);
        vec![]
    }

    /// What the token under the cursor completes to, for tab and the ghost text.
    pub fn completions_for(&self, line: &str) -> Vec<String> {
        match palette::slot(line) {
            Slot::Verb => palette::VERBS.iter().map(|(verb, _)| (*verb).to_owned()).collect(),
            Slot::Mr => self.queue_mrs().iter().map(|mr| format!("{}{}", self.kind.sigil(), mr.number)).collect(),
            Slot::Setting => vec!["theme=".to_owned()],
            Slot::Theme => Theme::NAMES.iter().map(|name| format!("theme={name}")).collect(),
            Slot::Free => vec![],
        }
    }

    fn run_command(&mut self, command: Command) -> Vec<Action> {
        match command {
            Command::Go(reference) => self.go(&reference),
            Command::Open => self.open_in_browser(),
            Command::Approve => self.toggle_approval(),
            Command::Publish => {
                self.open_publish();
                vec![]
            }
            Command::All => self.toggle_scope(),
            Command::Set { key, value } => self.set(&key, &value),
            Command::Help => {
                self.help = Some(0);
                vec![]
            }
            Command::Quit => {
                self.should_quit = true;
                vec![]
            }
        }
    }

    /// `!42`, `#42` or `42` pick the MR of that number in the queue; `group/project!42` names one anywhere.
    fn go(&mut self, reference: &str) -> Vec<Action> {
        let Some(key) = self.resolve(reference) else {
            self.warn(format!("no MR {reference} in the queue · name it in full: group/project!42"));
            return vec![];
        };
        self.open_key(key)
    }

    fn resolve(&self, reference: &str) -> Option<MrKey> {
        let reference = reference.trim();
        if let Some((project, number)) = reference.rsplit_once(['!', '#']).filter(|(p, _)| !p.is_empty()) {
            return Some(MrKey { project: project.to_owned(), number: number.parse().ok()? });
        }
        let number: u64 = reference.trim_start_matches(['!', '#']).parse().ok()?;
        self.queue_mrs().into_iter().find(|mr| mr.number == number).map(crate::forge::QueueMr::key)
    }

    fn open_in_browser(&self) -> Vec<Action> {
        match (&self.open, self.focus) {
            (Some(open), Focus::Review | Focus::Side) => vec![Action::OpenUrl(open.line_url(self.kind))],
            _ => self.selected_mr().map(|mr| vec![Action::OpenUrl(mr.web_url.clone())]).unwrap_or_default(),
        }
    }

    fn set(&mut self, key: &str, value: &str) -> Vec<Action> {
        if key != "theme" {
            self.warn(format!("unknown setting `{key}` · try :set theme=nord"));
            return vec![];
        }
        let Some(theme) = Theme::named(value) else {
            self.warn(format!("no theme `{value}` · try {}", Theme::NAMES.join(", ")));
            return vec![];
        };
        self.theme = self.ground.map_or(theme, |ground| theme.with_ground(ground));
        self.toast(format!("theme {}, saved", theme.name));
        vec![Action::SaveTheme(theme.name.to_owned())]
    }

    pub(super) fn open_jump(&mut self) {
        let files = self.open.iter().flat_map(|open| {
            open.review.files.iter().enumerate().map(|(i, file)| Candidate { label: file.new_path.clone(), target: Target::File(i) })
        });
        let sigil = self.kind.sigil();
        let mrs = self
            .queue_mrs()
            .into_iter()
            .map(|mr| Candidate { label: format!("{sigil}{} {}", mr.number, mr.title), target: Target::Mr(mr.key()) });
        self.jump = Some(Jump::new(files.chain(mrs).collect()));
    }

    pub(super) fn handle_jump_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let Some(mut jump) = self.jump.take() else { return vec![] };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => return vec![],
            KeyCode::Down | KeyCode::Tab => jump.move_by(1),
            KeyCode::Up | KeyCode::BackTab => jump.move_by(-1),
            KeyCode::Char('n') if ctrl => jump.move_by(1),
            KeyCode::Char('p') if ctrl => jump.move_by(-1),
            KeyCode::Backspace => jump.backspace(),
            KeyCode::Char(c) if !ctrl => jump.type_char(c),
            KeyCode::Enter => {
                return match jump.chosen().map(|c| c.target) {
                    Some(Target::Mr(key)) => self.open_key(key),
                    Some(Target::File(index)) => {
                        self.focus = Focus::Review;
                        self.review_jump_to(|row| matches!(row, Row::File { index: i, .. } if *i == index));
                        vec![]
                    }
                    None => vec![],
                };
            }
            _ => {}
        }
        self.jump = Some(jump);
        vec![]
    }

    /// Every MR the queue holds, whatever the filter and the folded sections.
    fn queue_mrs(&self) -> Vec<&crate::forge::QueueMr> {
        let Some(s) = &self.sections else { return vec![] };
        s.to_review.iter().chain(&s.mine).chain(&s.watching).chain(&s.open).chain(&s.done).collect()
    }
}
