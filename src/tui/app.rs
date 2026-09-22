//! The pure state machine: keys in, actions out, incoming answers applied. No clock, no network.
use super::theme::Theme;
use crate::api::User;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::time::Instant;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Focus {
    #[default]
    Queue,
    Review,
    Side,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    LoadMe,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Incoming {
    Me(User),
    Failed(String),
}

#[derive(Debug)]
pub struct App {
    pub theme: Theme,
    pub host: String,
    pub me: String,
    pub focus: Focus,
    pub help: bool,
    pub error: Option<String>,
    pub loading: bool,
    pub should_quit: bool,
    pub started: Instant,
    pub now: Instant,
}

impl App {
    pub fn new(theme: Theme, host: String, me: String) -> Self {
        let now = Instant::now();
        Self { theme, host, me, focus: Focus::default(), help: false, error: None, loading: true, should_quit: false, started: now, now }
    }

    pub fn start(&self) -> Vec<Action> {
        vec![Action::LoadMe]
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Vec<Action> {
        self.error = None;
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return vec![];
        }
        if self.help {
            self.help = false;
            return vec![];
        }
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.help = true,
            KeyCode::Char('h') | KeyCode::Left => self.focus = left_of(self.focus),
            KeyCode::Char('l') | KeyCode::Right => self.focus = right_of(self.focus),
            KeyCode::Char('r') => {
                self.loading = true;
                return vec![Action::LoadMe];
            }
            _ => {}
        }
        vec![]
    }

    pub fn apply(&mut self, incoming: Incoming) {
        self.loading = false;
        match incoming {
            Incoming::Me(me) => self.me = me.username,
            Incoming::Failed(message) => self.error = Some(message),
        }
    }
}

fn left_of(focus: Focus) -> Focus {
    match focus {
        Focus::Queue => Focus::Queue,
        Focus::Review => Focus::Queue,
        Focus::Side => Focus::Review,
    }
}

fn right_of(focus: Focus) -> Focus {
    match focus {
        Focus::Queue => Focus::Review,
        Focus::Review | Focus::Side => Focus::Side,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn app() -> App {
        App::new(Theme::default(), "gitlab.com".into(), String::new())
    }

    #[test]
    fn starts_by_loading_who_i_am() {
        assert_eq!(app().start(), vec![Action::LoadMe]);
    }

    #[test]
    fn q_quits_and_question_mark_opens_help() {
        let mut a = app();
        assert!(a.handle_key(key('?')).is_empty());
        assert!(a.help);
        a.handle_key(key('x'));
        assert!(!a.help, "any key closes help");
        a.handle_key(key('q'));
        assert!(a.should_quit);
    }

    #[test]
    fn focus_moves_between_the_three_panes_and_stops_at_the_edges() {
        let mut a = app();
        a.handle_key(key('h'));
        assert_eq!(a.focus, Focus::Queue);
        a.handle_key(key('l'));
        assert_eq!(a.focus, Focus::Review);
        a.handle_key(key('l'));
        a.handle_key(key('l'));
        assert_eq!(a.focus, Focus::Side);
    }

    #[test]
    fn me_arrives_and_errors_clear_on_the_next_key() {
        let mut a = app();
        a.apply(Incoming::Me(User { id: 1, username: "nina".into(), name: "Nina".into(), avatar_url: None }));
        assert_eq!(a.me, "nina");
        assert!(!a.loading);
        a.apply(Incoming::Failed("offline".into()));
        assert_eq!(a.error.as_deref(), Some("offline"));
        a.handle_key(key('l'));
        assert_eq!(a.error, None);
    }
}
