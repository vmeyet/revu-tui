use super::{Action, App, Brief, Focus};
use crate::review::Row;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const HALF_PAGE: isize = 10;

impl App {
    pub fn handle_key(&mut self, key: KeyEvent) -> Vec<Action> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return vec![];
        }
        if self.help {
            self.help = false;
            return vec![];
        }
        if self.input.is_some() {
            return self.handle_input_key(key);
        }
        if self.publish.is_some() {
            return self.handle_publish_key(key);
        }
        if self.brief.is_some() {
            return self.handle_brief_key(key);
        }
        if self.filtering {
            return self.handle_filter_key(key);
        }
        if let Some(prefix) = self.pending.take() {
            return self.handle_prefixed(prefix, key);
        }
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.help = true,
            KeyCode::Char('h') | KeyCode::Left => self.focus_left(),
            KeyCode::Char('l') | KeyCode::Right => return self.focus_right(),
            KeyCode::Char('z') | KeyCode::Char('[') | KeyCode::Char(']') if self.focus != Focus::Side => self.pending = key.code.as_char(),
            _ => {
                return match self.focus {
                    Focus::Queue => self.handle_queue_key(key),
                    Focus::Review => self.handle_review_key(key),
                    Focus::Side => self.handle_side_key(key),
                };
            }
        }
        vec![]
    }

    fn focus_left(&mut self) {
        self.focus = match self.focus {
            Focus::Side => Focus::Review,
            _ => Focus::Queue,
        };
    }

    /// From the queue, right opens the selected MR, as `enter` does, so the diff always matches the row.
    fn focus_right(&mut self) -> Vec<Action> {
        self.focus = match (self.focus, &self.open) {
            (Focus::Queue, _) => return self.open_selected(),
            (Focus::Review, Some(open)) if open.thread.is_some() => Focus::Side,
            (Focus::Side, _) => Focus::Side,
            (focus, _) => focus,
        };
        vec![]
    }

    fn handle_filter_key(&mut self, key: KeyEvent) -> Vec<Action> {
        match key.code {
            KeyCode::Esc => {
                self.filter.clear();
                self.filtering = false;
            }
            KeyCode::Enter => self.filtering = false,
            KeyCode::Backspace => {
                self.filter.pop();
            }
            KeyCode::Char(c) => self.filter.push(c),
            _ => {}
        }
        self.queue_settle();
        vec![]
    }

    fn handle_queue_key(&mut self, key: KeyEvent) -> Vec<Action> {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.queue_move(1),
            KeyCode::Char('k') | KeyCode::Up => self.queue_move(-1),
            KeyCode::Char('g') => self.queue_first(),
            KeyCode::Char('G') => self.queue_last(),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => self.queue_move(HALF_PAGE),
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => self.queue_move(-HALF_PAGE),
            KeyCode::Char('/') => self.filtering = true,
            KeyCode::Char('*') => return self.toggle_scope(),
            KeyCode::Char('i') => self.brief = self.selected_mr().map(Brief::of_queue),
            KeyCode::Enter => return self.open_selected(),
            KeyCode::Char('r') => return self.refresh_queue(),
            KeyCode::Char('o') => return self.selected_mr().map(|mr| vec![Action::OpenUrl(mr.web_url.clone())]).unwrap_or_default(),
            KeyCode::Char('y') => return self.selected_mr().map(|mr| vec![Action::Yank(mr.web_url.clone())]).unwrap_or_default(),
            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                self.queue_settle();
            }
            _ => {}
        }
        vec![]
    }

    fn refresh_queue(&mut self) -> Vec<Action> {
        if self.queue_loading {
            return vec![];
        }
        self.queue_loading = true;
        vec![Action::LoadQueue { scope: self.scope(), from_cache: false }]
    }

    /// Between the checkout's project and every project; the other list paints from its cache.
    fn toggle_scope(&mut self) -> Vec<Action> {
        if self.project.is_none() {
            self.toast("not in a GitLab checkout: the queue already shows every project");
            return vec![];
        }
        self.everywhere = !self.everywhere;
        self.sections = None;
        self.queue_selected = 0;
        self.queue_scroll = 0;
        self.queue_loading = true;
        vec![Action::LoadQueue { scope: self.scope(), from_cache: true }]
    }

    fn handle_review_key(&mut self, key: KeyEvent) -> Vec<Action> {
        if self.open.is_none() {
            if key.code == KeyCode::Esc {
                self.focus = Focus::Queue;
            }
            return vec![];
        }
        if !key.modifiers.contains(KeyModifiers::CONTROL)
            && let Some(actions) = self.handle_write_key(key)
        {
            return actions;
        }
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.review_move(1),
            KeyCode::Char('k') | KeyCode::Up => self.review_move(-1),
            KeyCode::Char('g') => self.review_first(),
            KeyCode::Char('G') => self.review_last(),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => self.review_move(HALF_PAGE),
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => self.review_move(-HALF_PAGE),
            KeyCode::Tab => self.review_jump(true, |r| matches!(r, Row::File { .. })),
            KeyCode::BackTab => self.review_jump(false, |r| matches!(r, Row::File { .. })),
            KeyCode::Enter => return self.enter_review_row(),
            KeyCode::Esc => self.focus = Focus::Queue,
            KeyCode::Char('r') => return self.refresh_open(),
            KeyCode::Char('i') => self.brief = self.open.as_ref().map(|o| Brief::of_review(&o.review)),
            KeyCode::Char('o') => return self.open.as_ref().map(|o| vec![Action::OpenUrl(o.line_url())]).unwrap_or_default(),
            KeyCode::Char('y') => return self.open.as_ref().map(|o| vec![Action::Yank(o.line_url())]).unwrap_or_default(),
            _ => {}
        }
        vec![]
    }

    /// A refresh also posts again every draft GitLab does not hold yet.
    fn refresh_open(&mut self) -> Vec<Action> {
        let Some(key) = self.open.as_ref().map(|o| o.key) else { return vec![] };
        let mut actions = vec![Action::RefreshMr(key)];
        actions.extend(self.retry_unsaved());
        actions
    }

    fn handle_prefixed(&mut self, prefix: char, key: KeyEvent) -> Vec<Action> {
        let KeyCode::Char(c) = key.code else { return vec![] };
        if self.focus == Focus::Queue {
            match (prefix, c) {
                ('z', 'o') => self.done_open = true,
                ('z', 'c') => self.done_open = false,
                ('z', 'a') => self.done_open = !self.done_open,
                _ => {}
            }
            self.queue_settle();
            return vec![];
        }
        let forward = prefix == ']';
        match (prefix, c) {
            ('z', 'a') => return self.fold_at_cursor(None),
            ('z', 'o') => return self.fold_at_cursor(Some(true)),
            ('z', 'c') => return self.fold_at_cursor(Some(false)),
            ('z', 'M') => return self.fold_all(true),
            ('z', 'R') => return self.fold_all(false),
            ('[' | ']', 'c') => self.review_jump(forward, |r| matches!(r, Row::Hunk { .. })),
            ('[' | ']', 'n') => self.review_jump(forward, |r| matches!(r, Row::Thread { .. } | Row::Outdated { .. })),
            ('[' | ']', 'f') => {
                let wanted = self.files_with_unresolved();
                self.review_jump(forward, move |r| matches!(r, Row::File { index, .. } if wanted.contains(index)));
            }
            _ => {}
        }
        vec![]
    }

    fn handle_side_key(&mut self, key: KeyEvent) -> Vec<Action> {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.thread_scroll(1),
            KeyCode::Char('k') | KeyCode::Up => self.thread_scroll(-1),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => self.thread_scroll(HALF_PAGE),
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => self.thread_scroll(-HALF_PAGE),
            KeyCode::Char('u') => {
                return match self.thread_link() {
                    Some(url) => vec![Action::OpenUrl(url)],
                    None => {
                        self.toast("no link in this thread");
                        vec![]
                    }
                };
            }
            KeyCode::Char('o') => {
                let url = self.open.as_ref().and_then(|o| o.thread.as_ref().map(|id| note_url(&o.review.mr.web_url, &o.review, id)));
                return url.map(|u| vec![Action::OpenUrl(u)]).unwrap_or_default();
            }
            KeyCode::Char('r') => self.reply_here(),
            KeyCode::Char('R') => return self.toggle_resolved(),
            KeyCode::Char('P') => self.open_publish(),
            KeyCode::Esc => self.close_thread(),
            _ => {}
        }
        vec![]
    }
}

fn note_url(web_url: &str, review: &crate::review::Review, id: &str) -> String {
    match review.thread(id) {
        Some(thread) => format!("{web_url}#note_{}", thread.first().id),
        None => web_url.to_owned(),
    }
}
