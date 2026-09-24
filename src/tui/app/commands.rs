//! The palette: MRs, the open MR's files, or commands, typed instead of walked to.
use super::{Action, App, Focus};
use crate::forge::MrKey;
use crate::fuzzy;
use crate::query::Query;
use crate::review::Row;
use crate::tui::help::Help;
use crate::tui::palette::{self, Candidate, Command, MAX_SHOWN, Mode, Palette, Slot, Target};
use crate::tui::theme::Theme;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

impl App {
    /// `ctrl-k` and `⌘k` open it on MRs, `:` and `⌘⇧k` on commands.
    pub(super) fn open_palette(&mut self, mode: Mode) {
        self.palette = Some(Palette::new(mode, self.palette_history.clone()));
    }

    pub(super) fn handle_palette_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let Some(mut palette) = self.palette.take() else { return vec![] };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => return vec![],
            KeyCode::Backspace if !palette.backspace() => return vec![],
            KeyCode::Backspace => {}
            KeyCode::Enter => return self.palette_enter(palette),
            KeyCode::Char('n') if ctrl => palette.move_by(1, self.palette_candidates(&palette).len()),
            KeyCode::Char('p') if ctrl => palette.move_by(-1, self.palette_candidates(&palette).len()),
            KeyCode::Char(c) if !ctrl => palette.type_char(c),
            _ if palette.mode == Mode::Commands => self.command_key(&mut palette, key.code),
            KeyCode::Down | KeyCode::Tab => palette.move_by(1, self.palette_candidates(&palette).len()),
            KeyCode::Up | KeyCode::BackTab => palette.move_by(-1, self.palette_candidates(&palette).len()),
            _ => {}
        }
        self.palette = Some(palette);
        vec![]
    }

    /// Tab completes, → takes the ghost, ↑ ↓ walk the history: a shell's command line.
    fn command_key(&self, palette: &mut Palette, code: KeyCode) {
        let candidates = self.completions_for(&palette.input);
        match code {
            KeyCode::Tab | KeyCode::BackTab => palette.complete(&candidates, code == KeyCode::BackTab),
            KeyCode::Right | KeyCode::End => palette.accept(&candidates),
            KeyCode::Up => palette.history_up(),
            KeyCode::Down => palette.history_down(),
            _ => {}
        }
    }

    fn palette_enter(&mut self, mut palette: Palette) -> Vec<Action> {
        if palette.mode == Mode::Commands {
            let line = palette.submit();
            self.palette_history.clone_from(&palette.history);
            return match palette::parse(&line) {
                Ok(command) => {
                    self.count_command(&command);
                    self.run_command(command)
                }
                Err(reason) => {
                    self.warn(reason);
                    vec![]
                }
            };
        }
        if !palette.input.is_empty() && palette.input.trim().chars().all(|c| c.is_ascii_digit()) {
            self.count_hint("palette_number");
        }
        match self.palette_candidates(&palette).into_iter().nth(palette.selected).map(|c| c.target) {
            Some(Target::Mr(key)) => {
                self.count("palette_mr");
                self.open_key(key)
            }
            Some(Target::File(index)) => {
                self.count("palette_file");
                self.focus = Focus::Review;
                self.review_jump_to(|row| matches!(row, Row::File { index: i, .. } if *i == index));
                vec![]
            }
            None => vec![],
        }
    }

    /// The rows the palette lists for what is typed: MRs the query keeps, ranked by its free
    /// words; the open MR's files by fuzzy path; nothing for commands, which complete in place.
    pub fn palette_candidates(&self, palette: &Palette) -> Vec<Candidate> {
        match palette.mode {
            Mode::Mrs => self.mr_candidates(&palette.input),
            Mode::Files => self.file_candidates(&palette.input),
            Mode::Commands => vec![],
        }
    }

    fn mr_candidates(&self, typed: &str) -> Vec<Candidate> {
        let query = Query::lenient(typed);
        let kept = self.queue_mrs().into_iter().filter(|mr| query.keeps(mr, &self.me)).map(|mr| {
            let sigil = self.hosts.kind_of(&mr.key()).sigil();
            let candidate =
                Candidate { label: format!("{sigil}{} {}", mr.number, mr.title), detail: mr.author.clone(), target: Target::Mr(mr.key()) };
            (format!("{} {} {}", candidate.label, mr.author, mr.source_branch), candidate)
        });
        let words = query.words.join(" ");
        let ranked: Vec<Candidate> =
            if words.is_empty() { kept.map(|(_, c)| c).collect() } else { fuzzy::rank(&words, kept).into_iter().map(|(_, c)| c).collect() };
        ranked.into_iter().take(MAX_SHOWN).collect()
    }

    fn file_candidates(&self, typed: &str) -> Vec<Candidate> {
        let files = self.open.iter().flat_map(|open| {
            open.review.files.iter().enumerate().map(|(i, file)| {
                (file.new_path.clone(), Candidate { label: file.new_path.clone(), detail: String::new(), target: Target::File(i) })
            })
        });
        fuzzy::rank(typed, files).into_iter().map(|(_, c)| c).take(MAX_SHOWN).collect()
    }

    /// What the token under the cursor completes to, for tab and the ghost text.
    pub fn completions_for(&self, line: &str) -> Vec<String> {
        match palette::slot(line) {
            Slot::Verb => palette::VERBS.iter().map(|(verb, _)| (*verb).to_owned()).collect(),
            Slot::Mr => self.queue_mrs().iter().map(|mr| format!("{}{}", self.hosts.kind_of(&mr.key()).sigil(), mr.number)).collect(),
            Slot::Setting => vec!["theme=".to_owned()],
            Slot::Theme => Theme::NAMES.iter().map(|name| format!("theme={name}")).collect(),
            Slot::File => {
                let files = self.open.iter().flat_map(|o| o.review.files.iter()).map(|f| f.new_path.clone());
                std::iter::once("old".to_owned()).chain(files).collect()
            }
            Slot::Free => vec![],
        }
    }

    fn run_command(&mut self, command: Command) -> Vec<Action> {
        match command {
            Command::Go(reference) => self.go(&reference),
            Command::Open => self.open_in_browser(),
            Command::Approve => self.toggle_approval(),
            Command::Merge => self.merge_here(),
            Command::Ready => self.toggle_draft(),
            Command::Publish => {
                self.open_publish();
                vec![]
            }
            Command::All => self.toggle_scope(),
            Command::Set { key, value } => self.set(&key, &value),
            Command::View(argument) => self.view_command(&argument),
            Command::AiOff => {
                self.ai_off();
                vec![]
            }
            Command::AiOn => {
                self.ai_on();
                vec![]
            }
            Command::Ask(question) => self.ask_free(question),
            Command::Share(target) => {
                self.start_share(target.as_deref());
                vec![]
            }
            Command::Help => {
                self.help = Some(Help::default());
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
            return Some(MrKey::new(project, number.parse().ok()?));
        }
        let number: u64 = reference.trim_start_matches(['!', '#']).parse().ok()?;
        self.queue_mrs().into_iter().find(|mr| mr.number == number).map(crate::forge::QueueMr::key)
    }

    fn open_in_browser(&self) -> Vec<Action> {
        match (&self.open, self.focus) {
            (Some(open), Focus::Review | Focus::Side) => vec![Action::OpenUrl(open.line_url(self.hosts.kind_of(&open.key)))],
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

    /// Every MR the queue holds, whatever the filter and the folded sections.
    fn queue_mrs(&self) -> Vec<&crate::forge::QueueMr> {
        self.sections.iter().flat_map(crate::forge::Sections::all).collect()
    }
}
