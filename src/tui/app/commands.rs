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
            KeyCode::Char('p') if ctrl && palette.mode == Mode::Commands => palette.history_up(),
            KeyCode::Char('n') if ctrl && palette.mode == Mode::Commands => palette.history_down(),
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

    /// ↑ ↓ pick a command from the list, then walk the options of its argument as tab does;
    /// → takes the ghost; ^p ^n walk the history, as in a shell.
    fn command_key(&self, palette: &mut Palette, code: KeyCode) {
        let candidates = self.completions_for(&palette.input);
        let listed = palette::verbs_for(&palette.input).len();
        match code {
            KeyCode::Up | KeyCode::Down if palette.naming_command() => palette.move_by(if code == KeyCode::Up { -1 } else { 1 }, listed),
            KeyCode::Up | KeyCode::Down => palette.complete(&candidates, code == KeyCode::Up),
            KeyCode::Tab | KeyCode::BackTab => palette.complete(&candidates, code == KeyCode::BackTab),
            KeyCode::Right | KeyCode::End => palette.accept(&candidates),
            _ => {}
        }
    }

    fn palette_enter(&mut self, mut palette: Palette) -> Vec<Action> {
        if palette.naming_command() {
            let picked = palette::verbs_for(&palette.input).get(palette.selected).map(|(verb, _)| (*verb).to_owned());
            if let Some(verb) = picked {
                palette.input.clone_from(&verb);
                if palette::parse(&verb).is_err() {
                    palette.input.push(' ');
                    self.palette = Some(palette);
                    return vec![];
                }
            }
        }
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
            Some(Target::Answer(index)) => {
                self.show_past_answer(index);
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
            Mode::Answers => self.answer_candidates(&palette.input),
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

    /// The kept answers, newest first, each with its age and a word when the MR moved on since.
    fn answer_candidates(&self, typed: &str) -> Vec<Candidate> {
        let head = self.open.as_ref().map(|o| o.review.mr.refs.head.clone()).unwrap_or_default();
        let answers = self.past_answers.iter().enumerate().map(|(i, past)| {
            let age = crate::tui::ui::short_age((self.today - past.asked_at).to_std().unwrap_or_default());
            let detail = if past.head == head { age } else { format!("{age} · older push") };
            (past.label.clone(), Candidate { label: past.label.clone(), detail, target: Target::Answer(i) })
        });
        fuzzy::rank(typed, answers).into_iter().map(|(_, c)| c).take(MAX_SHOWN).collect()
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
            Command::Threads => {
                self.toggle_every_thread();
                vec![]
            }
            Command::All => self.toggle_scope(),
            Command::Set { key, value } => self.set(&key, &value),
            Command::View(argument) => self.view_command(&argument),
            Command::Outline => self.show_outline(),
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crate::tui::app::test_support::*;

    fn run_line(app: &mut App, line: &str) -> Vec<Action> {
        press(app, ":");
        press(app, line);
        app.handle_key(code(KeyCode::Enter))
    }

    #[test]
    fn go_opens_an_mr_by_number_or_full_reference() {
        let mut app = with_queue();
        let actions = run_line(&mut app, "go !41");
        assert!(matches!(actions.as_slice(), [Action::Open(key)] if key.number == 41), "{actions:?}");
        let actions = run_line(&mut app, "go other/thing!7");
        assert!(matches!(actions.as_slice(), [Action::Open(key)] if key.project == "other/thing" && key.number == 7), "{actions:?}");
        assert_eq!(run_line(&mut app, "go !999"), vec![]);
        assert!(app.live_toast().unwrap().text.contains("no MR !999"));
    }

    #[test]
    fn set_theme_changes_it_now_and_asks_to_save_it() {
        let mut app = with_queue();
        assert_eq!(run_line(&mut app, "set theme=nord"), vec![Action::SaveTheme("nord".into())]);
        assert_eq!(app.theme.name, "nord");
        assert_eq!(run_line(&mut app, "set theme=nope"), vec![]);
        assert!(app.live_toast().unwrap().danger);
    }

    #[test]
    fn tab_completes_verbs_mrs_and_themes_and_up_recalls() {
        let mut app = with_queue();
        press(&mut app, ":pu");
        app.handle_key(code(KeyCode::Tab));
        assert_eq!(app.palette.as_ref().unwrap().input, "publish ");
        app.handle_key(code(KeyCode::Esc));
        assert!(app.palette.is_none());
        assert!(app.completions_for("go ").contains(&"!42".to_owned()));
        assert!(app.completions_for("set theme=").contains(&"theme=tokyonight".to_owned()));
        run_line(&mut app, "help");
        assert_eq!(app.help, Some(Help::default()));
        press(&mut app, "x:");
        app.handle_key(ctrl('p'));
        assert_eq!(app.palette.as_ref().unwrap().input, "help", "^p recalls, as in a shell");
    }

    #[test]
    fn arrows_pick_a_command_from_the_list_and_enter_runs_it_or_waits_for_its_argument() {
        let mut app = with_queue();
        press(&mut app, ":");
        for _ in 0..crate::tui::palette::VERBS.iter().position(|(v, _)| *v == "all").unwrap() {
            app.handle_key(code(KeyCode::Down));
        }
        app.handle_key(code(KeyCode::Up));
        app.handle_key(code(KeyCode::Down));
        assert!(render(&mut app, 120, 30).contains("▸ all"), "the picked command stands out");
        app.handle_key(code(KeyCode::Enter));
        assert!(app.palette.is_none() && app.everywhere, "all runs at once");
        press(&mut app, ":g");
        app.handle_key(code(KeyCode::Enter));
        assert_eq!(app.palette.as_ref().map(|p| p.input.as_str()), Some("go "), "go needs an MR: the line waits for it");
        press(&mut app, "4");
        app.handle_key(code(KeyCode::Down));
        assert_eq!(app.palette.as_ref().unwrap().input, "go !42 ", "past the verb, ↓ walks the argument's options");
    }

    fn palette_labels(app: &App) -> Vec<String> {
        app.palette_candidates(app.palette.as_ref().unwrap()).into_iter().map(|c| c.label).collect()
    }

    #[test]
    fn ctrl_k_finds_mrs_and_a_slash_finds_the_open_mrs_files() {
        let mut app = with_review();
        app.handle_key(ctrl_k());
        assert_eq!(app.palette.as_ref().unwrap().mode, crate::tui::palette::Mode::Mrs);
        press(&mut app, "!41");
        assert_eq!(palette_labels(&app), ["!41 fix: flaky cache test"]);
        let actions = app.handle_key(code(KeyCode::Enter));
        assert!(matches!(actions.as_slice(), [Action::Open(key)] if key.number == 41));
        let mut app = with_review();
        app.handle_key(ctrl_k());
        press(&mut app, "/");
        assert_eq!(app.palette.as_ref().unwrap().mode, crate::tui::palette::Mode::Files);
        assert!(matches!(app.palette_candidates(app.palette.as_ref().unwrap())[0].target, crate::tui::palette::Target::File(0)));
        app.handle_key(code(KeyCode::Enter));
        assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::File { index: 0, .. })));
    }

    #[test]
    fn the_mr_search_combines_author_label_number_and_words() {
        let mut app = with_queue();
        app.handle_key(ctrl_k());
        press(&mut app, "@omar");
        assert_eq!(palette_labels(&app), ["!42 feat: charge cards at checkout", "!35 infra: new runner"]);
        press(&mut app, " run");
        assert_eq!(palette_labels(&app), ["!35 infra: new runner"], "words rank what the terms keep");
        app.handle_key(code(KeyCode::Esc));
        app.handle_key(ctrl_k());
        press(&mut app, "@nobody");
        assert_eq!(palette_labels(&app), [] as [String; 0]);
    }

    #[test]
    fn greater_than_switches_to_commands_and_backspace_comes_back_to_mrs() {
        let mut app = with_queue();
        app.handle_key(ctrl_k());
        press(&mut app, ">he");
        assert_eq!(app.palette.as_ref().unwrap().mode, crate::tui::palette::Mode::Commands);
        app.handle_key(code(KeyCode::Tab));
        assert_eq!(app.palette.as_ref().unwrap().input, "help ");
        app.handle_key(code(KeyCode::Enter));
        assert_eq!(app.help, Some(Help::default()));
        press(&mut app, "x");
        app.handle_key(ctrl_k());
        press(&mut app, ">");
        app.handle_key(code(KeyCode::Backspace));
        assert_eq!(app.palette.as_ref().unwrap().mode, crate::tui::palette::Mode::Mrs);
        app.handle_key(code(KeyCode::Backspace));
        assert!(app.palette.is_none(), "a backspace on nothing closes it");
    }

    #[test]
    fn command_k_opens_the_palette_on_mrs_and_command_shift_k_on_commands() {
        use crate::tui::palette::Mode;
        let mut app = with_queue();
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::SUPER));
        assert_eq!(app.palette.as_ref().map(|p| p.mode), Some(Mode::Mrs));
        app.handle_key(code(KeyCode::Esc));
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::SUPER | KeyModifiers::SHIFT));
        assert_eq!(app.palette.as_ref().map(|p| p.mode), Some(Mode::Commands));
        app.handle_key(code(KeyCode::Esc));
        press(&mut app, ":");
        assert_eq!(app.palette.as_ref().map(|p| p.mode), Some(Mode::Commands), "`:` keeps its muscle memory");
    }
}
