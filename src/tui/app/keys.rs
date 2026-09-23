use super::{Action, App, Brief, Focus};
use crate::review::Row;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const HALF_PAGE: isize = 10;

impl App {
    pub fn handle_key(&mut self, key: KeyEvent) -> Vec<Action> {
        self.news = None;
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return vec![];
        }
        if let Some(scroll) = self.help {
            self.help = help_scroll(scroll, key);
            return vec![];
        }
        if self.confirm.is_some() {
            return self.handle_confirm_key(key);
        }
        if self.input.is_some() {
            return self.handle_input_key(key);
        }
        if self.palette.is_some() {
            return self.handle_palette_key(key);
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
        match self.keymap.feed(self.held.take(), key) {
            crate::keymap::Feed::Hold(first) => {
                self.held = Some(first);
                vec![]
            }
            crate::keymap::Feed::Keys(keys) => keys.into_iter().flat_map(|key| self.dispatch(key)).collect(),
        }
    }

    /// A key after the user's bindings turned it into revu's own: prefixes, then the pane's keys.
    fn dispatch(&mut self, key: KeyEvent) -> Vec<Action> {
        if let Some(prefix) = self.pending.take() {
            return self.handle_prefixed(prefix, key);
        }
        if let Some(mode) = palette_key(key) {
            self.open_palette(mode);
            return vec![];
        }
        match key.code {
            KeyCode::Char(':') => self.open_palette(crate::tui::palette::Mode::Commands),
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.help = Some(0),
            KeyCode::Char('h') | KeyCode::Left => self.focus_left(),
            KeyCode::Char('l') | KeyCode::Right => return self.focus_right(),
            KeyCode::Char('z' | '[' | ']') if self.focus != Focus::Side || self.tree_open() => self.pending = key.code.as_char(),
            KeyCode::Char('a') if self.focus != Focus::Queue && self.open.is_some() && !self.answer_open() => self.pending = Some('a'),
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
        if self.focus == Focus::Review {
            self.reading = false;
        }
        self.focus = match self.focus {
            Focus::Side => Focus::Review,
            _ => Focus::Queue,
        };
    }

    /// From the queue, right opens the selected MR, as `enter` does, so the diff always matches the row.
    /// In the diff, a marked line (or a file's outdated threads) opens the pane on it.
    fn focus_right(&mut self) -> Vec<Action> {
        if self.focus == Focus::Review && (self.open_pane_here() || self.open_outdated_here()) {
            return vec![];
        }
        self.focus = match (self.focus, &self.open) {
            (Focus::Queue, _) => return self.open_selected(),
            (Focus::Review, Some(open))
                if open.pane.is_some() || open.tree.is_some() || open.answer.is_some() || open.pipeline.is_some() =>
            {
                Focus::Side
            }
            (Focus::Side, _) => Focus::Side,
            (focus, _) => focus,
        };
        vec![]
    }

    fn handle_filter_key(&mut self, key: KeyEvent) -> Vec<Action> {
        match key.code {
            KeyCode::Esc => {
                self.filter.clear();
                self.view = None;
                self.filtering = false;
            }
            KeyCode::Enter => self.filtering = false,
            KeyCode::Backspace => {
                self.filter.pop();
                self.view = None;
            }
            KeyCode::Char(c) => {
                self.filter.push(c);
                self.view = None;
            }
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
            KeyCode::Char('s') => return self.sort_queue(),
            KeyCode::Char('\'') => self.pending = Some('\''),
            KeyCode::Char(c @ '1'..='9') => self.apply_view(c),
            KeyCode::Char('S') => return self.group_queue(),
            KeyCode::Char('i') => self.brief = self.selected_mr().map(|mr| Brief::of_queue(mr, self.hosts.kind_of(&mr.key()).sigil())),
            KeyCode::Enter => return self.open_selected(),
            KeyCode::Char('r') => return self.refresh_queue(),
            KeyCode::Char('o') => return self.selected_mr().map(|mr| vec![Action::OpenUrl(mr.web_url.clone())]).unwrap_or_default(),
            KeyCode::Char('y') => return self.selected_mr().map(|mr| vec![Action::Yank(mr.web_url.clone())]).unwrap_or_default(),
            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                self.view = None;
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
    pub(super) fn toggle_scope(&mut self) -> Vec<Action> {
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
        let actions = self.move_in_review(key);
        self.follow_cursor();
        actions
    }

    fn move_in_review(&mut self, key: KeyEvent) -> Vec<Action> {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.review_move(1),
            KeyCode::Char('k') | KeyCode::Up => self.review_move(-1),
            KeyCode::Char('g') => self.review_first(),
            KeyCode::Char('G') => self.review_last(),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => self.review_move(HALF_PAGE),
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => self.review_move(-HALF_PAGE),
            KeyCode::Char('D') => return self.toggle_split(),
            KeyCode::Char('t') => self.toggle_tree(),
            KeyCode::Char('p') => return self.toggle_pipeline(),
            KeyCode::Char('W') => self.toggle_whitespace(),
            KeyCode::Char('+') => return self.expand_context(),
            KeyCode::Char('w') => {
                self.wrap = !self.wrap;
                self.toast(if self.wrap { "long lines wrap" } else { "long lines end in …" });
            }
            KeyCode::Tab => self.review_jump(true, |r| matches!(r, Row::File { .. })),
            KeyCode::BackTab => self.review_jump(false, |r| matches!(r, Row::File { .. })),
            KeyCode::Enter => return self.enter_review_row(),
            KeyCode::Esc | KeyCode::Char('x') if self.answer_open() => self.close_answer(),
            KeyCode::Esc | KeyCode::Char('x') if self.open.as_ref().is_some_and(|o| o.pane.is_some()) => self.close_pane(),
            KeyCode::Esc => self.focus = Focus::Queue,
            KeyCode::Char('r') => return self.refresh_open(),
            KeyCode::Char('i') => self.brief = self.open.as_ref().map(|o| Brief::of_review(&o.review, self.hosts.kind_of(&o.key).sigil())),
            KeyCode::Char('v') => return self.view_here(crate::review::Side::New),
            KeyCode::Char('o') => {
                return self.open.as_ref().map(|o| vec![Action::OpenUrl(o.line_url(self.hosts.kind_of(&o.key)))]).unwrap_or_default();
            }
            KeyCode::Char('y') => {
                return self.open.as_ref().map(|o| vec![Action::Yank(o.line_url(self.hosts.kind_of(&o.key)))]).unwrap_or_default();
            }
            _ => {}
        }
        vec![]
    }

    /// A refresh also posts again every draft GitLab does not hold yet.
    fn refresh_open(&mut self) -> Vec<Action> {
        let Some(key) = self.open.as_ref().map(|o| o.key.clone()) else { return vec![] };
        let mut actions = vec![Action::RefreshMr(key)];
        actions.extend(self.retry_unsaved());
        actions
    }

    fn handle_prefixed(&mut self, prefix: char, key: KeyEvent) -> Vec<Action> {
        let KeyCode::Char(c) = key.code else { return vec![] };
        if prefix == '\'' {
            self.apply_view(c);
            return vec![];
        }
        if self.focus == Focus::Queue {
            let open = match (prefix, c) {
                ('z', 'o') => Some(true),
                ('z', 'c') => Some(false),
                ('z', 'a') => None,
                ('z', 'z') => {
                    self.toggle_reading();
                    return vec![];
                }
                _ => return vec![],
            };
            if let Some(actions) = self.fold_stack(open) {
                return actions;
            }
            self.fold_section(open);
            return vec![];
        }
        if prefix == 'a' {
            return self.ask_key(c);
        }
        let forward = prefix == ']';
        match (prefix, c) {
            ('z', 'a') => return self.fold_at_cursor(None),
            ('z', 'o') => return self.fold_at_cursor(Some(true)),
            ('z', 'c') => return self.fold_at_cursor(Some(false)),
            ('z', 'h') => self.header_folded = !self.header_folded,
            ('z', 'z') => self.toggle_reading(),
            ('z', 'v') => return self.toggle_viewed(),
            ('z', 'M') => return self.fold_all(true),
            ('z', 'R') => return self.fold_all(false),
            ('[' | ']', 'c') => self.review_jump(forward, |r| matches!(r, Row::Hunk { .. })),
            ('[' | ']', 'n') => self.jump_to_marked(forward),
            ('[' | ']', 'f') => {
                let wanted = self.files_with_unresolved();
                self.review_jump(forward, move |r| matches!(r, Row::File { index, .. } if wanted.contains(index)));
            }
            _ => {}
        }
        vec![]
    }

    /// `zz`: the diff alone and centered; the queue comes back with `zz` or `h`.
    fn toggle_reading(&mut self) {
        if self.open.is_none() {
            self.toast("open an MR first");
            return;
        }
        self.reading = !self.reading;
        self.focus = Focus::Review;
    }

    fn tree_open(&self) -> bool {
        self.open.as_ref().is_some_and(|o| o.tree.is_some())
    }

    fn handle_side_key(&mut self, key: KeyEvent) -> Vec<Action> {
        if self.answer_open() {
            return self.handle_answer_key(key);
        }
        if self.pipeline_open() {
            return self.handle_pipeline_key(key);
        }
        if self.tree_open() {
            return self.handle_tree_key(key);
        }
        self.handle_pane_key(key)
    }
}

impl App {
    /// Claude's answer holds the right pane.
    pub(super) fn answer_open(&self) -> bool {
        self.open.as_ref().is_some_and(|o| o.answer.is_some())
    }
}

/// Moving keys scroll the key list; any other key closes it.
fn help_scroll(scroll: usize, key: KeyEvent) -> Option<usize> {
    let last = crate::tui::help::last_row();
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => Some((scroll + 1).min(last)),
        KeyCode::Char('k') | KeyCode::Up => Some(scroll.saturating_sub(1)),
        KeyCode::Char('d') if ctrl => Some((scroll + HALF_PAGE.unsigned_abs()).min(last)),
        KeyCode::Char('u') if ctrl => Some(scroll.saturating_sub(HALF_PAGE.unsigned_abs())),
        KeyCode::Char('g') => Some(0),
        KeyCode::Char('G') => Some(last),
        _ => None,
    }
}

/// `ctrl-k` and `⌘k` open the palette on MRs; `⌘⇧k`, where the terminal tells it apart, on commands.
fn palette_key(key: KeyEvent) -> Option<crate::tui::palette::Mode> {
    use crate::tui::palette::Mode;
    let k = matches!(key.code, KeyCode::Char('k' | 'K'));
    let command = key.modifiers.contains(KeyModifiers::SUPER);
    match (k, command, key.modifiers.contains(KeyModifiers::SHIFT) || key.code == KeyCode::Char('K')) {
        (true, true, true) => Some(Mode::Commands),
        (true, true, false) => Some(Mode::Mrs),
        _ if key.code == KeyCode::Char('k') && key.modifiers.contains(KeyModifiers::CONTROL) => Some(Mode::Mrs),
        _ => None,
    }
}
