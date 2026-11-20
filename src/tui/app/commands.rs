//! The palette: MRs, the open MR's files, or commands, typed instead of walked to.
use super::{Action, App, Focus};
use crate::forge::MrKey;
use crate::query::Query;
use crate::tui::palette::{self, Candidate, Command, Mode, Palette, Slot, Target};
use crate::tui::theme::Theme;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use vtui::fuzzy;
use vtui::palette::MAX_SHOWN;
use vtui::toward::Toward;

impl App {
    /// `ctrl-k` and `⌘k` open it on MRs, `:` and `⌘⇧k` on commands.
    pub(super) fn open_palette(&mut self, mode: Mode) {
        self.palette = Some(Palette::new(mode, self.palette_history.clone()));
    }

    pub(super) fn handle_palette_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let Some(mut palette) = self.palette.take() else { return vec![] };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let plain = !ctrl && !key.modifiers.contains(KeyModifiers::ALT);
        let commands = palette.mode == Mode::Commands;
        match key.code {
            KeyCode::Esc => return vec![],
            KeyCode::Enter => return self.palette_enter(palette),
            KeyCode::Backspace if plain && !palette.backspace() => return vec![],
            KeyCode::Backspace if plain => {}
            // `ctrl-k` opens the palette, so in it the key deletes nothing.
            KeyCode::Char('k') if ctrl => {}
            KeyCode::Char('p') if ctrl && commands => palette.history_up(),
            KeyCode::Char('n') if ctrl && commands => palette.history_down(),
            KeyCode::Char('n') if ctrl => palette.move_by(1, self.palette_candidates(&palette).len()),
            KeyCode::Char('p') if ctrl => palette.move_by(-1, self.palette_candidates(&palette).len()),
            KeyCode::Up | KeyCode::Down | KeyCode::Tab | KeyCode::BackTab if commands => self.command_key(&mut palette, key.code),
            KeyCode::Right | KeyCode::End if commands && palette.field().at_end() => {
                palette.accept(&self.completions_for(palette.input()));
            }
            KeyCode::Down | KeyCode::Tab => palette.move_by(1, self.palette_candidates(&palette).len()),
            KeyCode::Up | KeyCode::BackTab => palette.move_by(-1, self.palette_candidates(&palette).len()),
            _ => palette.press(key),
        }
        self.palette = Some(palette);
        vec![]
    }

    /// ↑ ↓ pick a command from the list, ↑ on an empty line at the top of it recalls the last line, then they walk
    /// the options of the argument as tab does; ^p ^n walk the history from any line, as in a shell.
    fn command_key(&self, palette: &mut Palette, code: KeyCode) {
        let candidates = self.completions_for(palette.input());
        let listed = palette::verbs_for(palette.input(), self.kind()).len();
        match code {
            KeyCode::Up | KeyCode::Down if palette.naming_command() || palette.walking_history() => {
                palette.arrow(Toward::next_if(code == KeyCode::Down), listed);
            }
            KeyCode::Up | KeyCode::Down => palette.complete(&candidates, Toward::next_if(code == KeyCode::Down)),
            KeyCode::Tab | KeyCode::BackTab => palette.complete(&candidates, Toward::next_if(code == KeyCode::Tab)),
            _ => {}
        }
    }

    fn palette_enter(&mut self, mut palette: Palette) -> Vec<Action> {
        if palette.naming_command() {
            let picked = palette::verbs_for(palette.input(), self.kind()).get(palette.selected).map(|verb| verb.word);
            if let Some(verb) = picked {
                palette.set_input(verb);
                if palette::parse(verb).is_err() {
                    palette.type_char(' ');
                    self.palette = Some(palette);
                    return vec![];
                }
            }
        }
        if palette.mode == Mode::Commands {
            let line = palette.submit();
            self.palette_history.clone_from(&palette.history);
            return match palette::parse(&line) {
                Ok(command) if !command.act().row().reach.serves(self.kind()) => {
                    self.warn(format!("`{line}` does nothing here"));
                    vec![]
                }
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
        match self.palette_candidates(&palette).into_iter().nth(palette.selected).map(|c| c.target) {
            Some(Target::Mr(key)) => {
                self.count("palette_mr");
                self.open_key(key)
            }
            Some(Target::Session(index)) => self.pick_session(index),
            None => vec![],
        }
    }

    /// The rows the palette lists for what is typed: MRs the query keeps, ranked by its free
    /// words; nothing for commands, which complete in place.
    pub(crate) fn palette_candidates(&self, palette: &Palette) -> Vec<Candidate> {
        match palette.mode {
            Mode::Mrs => self.mr_candidates(palette.input()),
            Mode::Sessions => self.session_candidates(palette.input()),
            Mode::Commands => vec![],
        }
    }

    fn mr_candidates(&self, typed: &str) -> Vec<Candidate> {
        let query = Query::lenient(typed);
        let kept = self.queue_mrs().into_iter().filter(|mr| query.keeps(mr, self.me.on_row(mr))).map(|mr| {
            let candidate = Candidate { label: self.row_label(mr), detail: mr.author.clone(), target: Target::Mr(mr.key()) };
            (format!("{} {} {}", candidate.label, mr.author, mr.source_branch), candidate)
        });
        let words = query.words.join(" ");
        let ranked: Vec<Candidate> =
            if words.is_empty() { kept.map(|(_, c)| c).collect() } else { fuzzy::rank(&words, kept).into_iter().map(|(_, c)| c).collect() };
        ranked.into_iter().take(MAX_SHOWN).collect()
    }

    /// What the token under the cursor completes to, for tab and the ghost text.
    pub(crate) fn completions_for(&self, line: &str) -> Vec<String> {
        match palette::slot(line) {
            Slot::Verb => crate::act::verbs_on(self.kind()).map(|verb| verb.word.to_owned()).collect(),
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
            Command::Go(reference) => self.open_reference(&reference),
            Command::Copy => self.link_here().map(|url| vec![Action::copy_link(url)]).unwrap_or_default(),
            Command::Approve => self.toggle_approval(),
            Command::Publish => {
                self.open_publish();
                vec![]
            }
            Command::Threads => {
                self.toggle_every_thread();
                vec![]
            }
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
            Command::Share(target) => {
                self.start_share(target.as_deref());
                vec![]
            }
            Command::Session => self.open_session(),
            Command::News => {
                self.open_whats_new();
                vec![]
            }
            Command::Target(target) => self.change_local(crate::forge::local::Change::Target(target)),
            Command::Title(title) => self.set_title(title),
            Command::Quit => {
                self.should_quit = true;
                vec![]
            }
        }
    }

    /// `!42`, `#42` or `42` pick the MR of that number in the queue; `group/project!42` names one anywhere.
    fn open_reference(&mut self, reference: &str) -> Vec<Action> {
        let Some(key) = self.resolve(reference) else {
            self.warn(format!("no MR {reference} in the queue · name it in full: group/project!42"));
            return vec![];
        };
        self.open_key(key)
    }

    fn resolve(&self, reference: &str) -> Option<MrKey> {
        let reference = reference.trim();
        if let Some((project, number)) = reference.rsplit_once(['!', '#']).filter(|(p, _)| !p.is_empty()) {
            return Some(MrKey::new(crate::forge::ProjectPath::parse(project)?, number.parse().ok()?));
        }
        let number: u64 = reference.trim_start_matches(['!', '#']).parse().ok()?;
        self.queue_mrs().into_iter().find(|mr| mr.number == number).map(crate::forge::QueueMr::key)
    }

    /// The page of the line under the cursor in an open MR, else of the MR selected in the queue.
    fn link_here(&self) -> Option<String> {
        match (&self.open, self.focus) {
            (Some(open), Focus::Review | Focus::Side) => Some(open.line_url(self.hosts.kind_of(&open.key))),
            _ => self.selected_mr().map(|mr| mr.web_url.clone()),
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
    use crate::tui::app::test_support::*;
    use crate::tui::palette::{Mode, Palette};

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
        assert!(
            matches!(actions.as_slice(), [Action::Open(key)] if key.project.as_str() == "other/thing" && key.number == 7),
            "{actions:?}"
        );
        assert_eq!(run_line(&mut app, "go !999"), vec![]);
        assert!(app.live_toast().unwrap().text.contains("no MR !999"));
    }

    #[test]
    fn copy_takes_the_link_o_would_open_in_the_browser() {
        let copied = |url: &str| vec![Action::Copy { text: url.into(), done: format!("copied {url}") }];
        let mut app = with_queue();
        assert_eq!(run_line(&mut app, "copy"), copied("https://gitlab.com/acme/widgets/-/merge_requests/42"));
        let mut app = with_review();
        press(&mut app, "]cj");
        let opened = press(&mut app, "o");
        let [Action::OpenUrl(line)] = opened.as_slice() else { panic!("the line in the browser: {opened:?}") };
        assert!(line.contains("/diffs#"), "{line}");
        assert_eq!(run_line(&mut app, "copy"), copied(line));
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
        assert_eq!(app.palette.as_ref().unwrap().input(), "publish ");
        app.handle_key(code(KeyCode::Esc));
        assert!(app.palette.is_none());
        assert!(app.completions_for("go ").contains(&"!42".to_owned()));
        assert!(app.completions_for("set theme=").contains(&"theme=tokyonight".to_owned()));
        run_line(&mut app, "copy");
        press(&mut app, ":");
        app.handle_key(ctrl('p'));
        assert_eq!(app.palette.as_ref().unwrap().input(), "copy", "^p recalls, as in a shell");
    }

    #[test]
    fn up_on_an_empty_line_recalls_the_last_command_and_the_arrows_walk_on() {
        let mut app = with_queue();
        run_line(&mut app, "copy");
        run_line(&mut app, "set theme=nord");
        press(&mut app, ":");
        app.handle_key(code(KeyCode::Up));
        assert_eq!(app.palette.as_ref().unwrap().input(), "set theme=nord");
        app.handle_key(code(KeyCode::Up));
        assert_eq!(app.palette.as_ref().unwrap().input(), "copy");
        app.handle_key(code(KeyCode::Down));
        app.handle_key(code(KeyCode::Down));
        assert_eq!(app.palette.as_ref().unwrap().input(), "");
    }

    #[test]
    fn down_on_an_empty_line_picks_from_the_list_with_history_to_recall() {
        let mut app = with_queue();
        run_line(&mut app, "copy");
        press(&mut app, ":");
        app.handle_key(code(KeyCode::Down));
        let palette = app.palette.as_ref().unwrap();
        assert_eq!((palette.input(), palette.selected), ("", 1));
    }

    #[test]
    fn up_finding_mrs_never_recalls_a_command() {
        let mut app = with_queue();
        run_line(&mut app, "copy");
        app.handle_key(ctrl_k());
        app.handle_key(code(KeyCode::Up));
        assert_eq!(app.palette.as_ref().unwrap().input(), "");
    }

    #[test]
    fn arrows_pick_a_command_from_the_list_and_enter_runs_it_or_waits_for_its_argument() {
        let mut app = with_queue();
        press(&mut app, ":");
        for _ in 0..crate::act::verbs().position(|verb| verb.word == "copy").unwrap() {
            app.handle_key(code(KeyCode::Down));
        }
        app.handle_key(code(KeyCode::Up));
        app.handle_key(code(KeyCode::Down));
        assert!(render(&mut app, 120, 30).contains("▸ copy"), "the picked command stands out");
        let actions = app.handle_key(code(KeyCode::Enter));
        assert!(app.palette.is_none() && matches!(actions.as_slice(), [Action::Copy { .. }]), "copy runs at once");
        press(&mut app, ":g");
        app.handle_key(code(KeyCode::Enter));
        assert_eq!(app.palette.as_ref().map(Palette::input), Some("go "), "go needs an MR: the line waits for it");
        press(&mut app, "4");
        app.handle_key(code(KeyCode::Down));
        assert_eq!(app.palette.as_ref().unwrap().input(), "go !42 ", "past the verb, ↓ walks the argument's options");
    }

    /// The palette after `line` is typed on commands, then `keys` pressed.
    fn command_line_after(line: &str, keys: &[KeyEvent]) -> App {
        let mut app = with_queue();
        press(&mut app, ":");
        press(&mut app, line);
        press_keys(&mut app, keys);
        app
    }

    /// The palette's line, with `|` where its cursor is.
    fn line_with_cursor(app: &App) -> String {
        let (before, under, after) = app.palette.as_ref().unwrap().field().split();
        format!("{before}|{under}{after}")
    }

    #[test]
    fn left_and_right_move_the_cursor_of_the_palette_by_character() {
        let left = code(KeyCode::Left);
        assert_eq!(line_with_cursor(&command_line_after("go !42", &[left, left, code(KeyCode::Right)])), "go !4|2");
    }

    #[test]
    fn alt_left_and_alt_right_move_the_cursor_of_the_palette_by_word() {
        let back = alt(KeyCode::Left);
        assert_eq!(line_with_cursor(&command_line_after("go !42 now", &[back, back, back, alt(KeyCode::Right)])), "go| !42 now");
    }

    #[test]
    fn alt_b_and_alt_f_move_by_word_in_the_palette_and_type_no_letter() {
        let app = command_line_after("go !42", &[alt(KeyCode::Char('b')), alt(KeyCode::Char('b')), alt(KeyCode::Char('f'))]);
        assert_eq!(line_with_cursor(&app), "go| !42");
    }

    #[test]
    fn a_character_is_typed_where_the_cursor_of_the_palette_is() {
        assert_eq!(line_with_cursor(&command_line_after("go !2", &[code(KeyCode::Left), key('4')])), "go !4|2");
    }

    #[test]
    fn alt_backspace_in_the_palette_deletes_the_word_before_the_cursor() {
        assert_eq!(line_with_cursor(&command_line_after("go !42 now", &[alt(KeyCode::Left), alt(KeyCode::Backspace)])), "go |now");
    }

    #[test]
    fn ctrl_u_in_the_palette_deletes_back_to_the_start_of_the_line() {
        assert_eq!(line_with_cursor(&command_line_after("go !42", &[alt(KeyCode::Left), ctrl('u')])), "|!42");
    }

    #[test]
    fn ctrl_k_in_the_open_palette_keeps_the_line_and_the_mode() {
        let app = command_line_after("go !42", &[alt(KeyCode::Left), ctrl_k()]);
        assert_eq!((line_with_cursor(&app).as_str(), app.palette.as_ref().unwrap().mode), ("go |!42", Mode::Commands));
    }

    #[test]
    fn right_at_the_end_of_the_line_takes_the_ghost() {
        assert_eq!(line_with_cursor(&command_line_after("pu", &[code(KeyCode::Right)])), "publish |");
    }

    #[test]
    fn right_inside_the_line_moves_the_cursor_and_takes_no_ghost() {
        assert_eq!(line_with_cursor(&command_line_after("pu", &[code(KeyCode::Left), code(KeyCode::Right)])), "pu|");
    }

    #[test]
    fn end_inside_the_line_goes_to_its_end_and_takes_no_ghost() {
        assert_eq!(line_with_cursor(&command_line_after("pu", &[code(KeyCode::Home), code(KeyCode::End)])), "pu|");
    }

    #[test]
    fn backspace_at_the_start_of_a_typed_line_keeps_the_palette_open() {
        assert_eq!(line_with_cursor(&command_line_after("go", &[code(KeyCode::Home), code(KeyCode::Backspace)])), "|go");
    }

    #[test]
    fn ctrl_n_and_ctrl_p_move_the_highlight_in_the_list_of_mrs() {
        let mut app = with_queue();
        press_keys(&mut app, &[ctrl_k(), ctrl('n'), ctrl('n'), ctrl('p')]);
        assert_eq!(app.palette.as_ref().unwrap().selected, 1);
    }

    fn palette_labels(app: &App) -> Vec<String> {
        app.palette_candidates(app.palette.as_ref().unwrap()).into_iter().map(|c| c.label).collect()
    }

    #[test]
    fn ctrl_k_finds_mrs() {
        let mut app = with_review();
        app.handle_key(ctrl_k());
        assert_eq!(app.palette.as_ref().unwrap().mode, Mode::Mrs);
        press(&mut app, "!41");
        assert_eq!(palette_labels(&app), ["!41 fix: flaky cache test"]);
        let actions = app.handle_key(code(KeyCode::Enter));
        assert!(matches!(actions.as_slice(), [Action::Open(key)] if key.number == 41));
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
        press(&mut app, ">co");
        assert_eq!(app.palette.as_ref().unwrap().mode, Mode::Commands);
        app.handle_key(code(KeyCode::Tab));
        assert_eq!(app.palette.as_ref().unwrap().input(), "copy ");
        app.handle_key(code(KeyCode::Enter));
        assert!(app.palette.is_none());
        app.handle_key(ctrl_k());
        press(&mut app, ">");
        app.handle_key(code(KeyCode::Backspace));
        assert_eq!(app.palette.as_ref().unwrap().mode, Mode::Mrs);
        app.handle_key(code(KeyCode::Backspace));
        assert!(app.palette.is_none(), "a backspace on nothing closes it");
    }

    #[test]
    fn command_k_opens_the_palette_on_mrs_and_command_shift_k_on_commands() {
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
