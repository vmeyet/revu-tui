use super::pane::Threads;
use super::{Action, App, Focus};
use crate::review::Row;
use crate::tui::help::{self, Help};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;

const HALF_PAGE: isize = 10;
/// Rows of one page still on screen after a full page down, as vim keeps them for context.
const KEPT_ROWS: u16 = 2;

impl App {
    /// Any key but the one finishing a quit calls the pending quit off, then does its own job.
    pub fn handle_key(&mut self, key: KeyEvent) -> Vec<Action> {
        self.drag = None;
        let pending = self.quitting;
        let actions = self.route_key(key);
        if self.quitting == pending {
            self.quitting = None;
        }
        actions
    }

    fn route_key(&mut self, key: KeyEvent) -> Vec<Action> {
        self.news = None;
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return self.quit_key(super::quit::QuitKey::CtrlC);
        }
        if let Some(open) = self.help {
            self.help = self.help_key(open, paged(key));
            return vec![];
        }
        if let Some(answered) = self.answer_offer(key) {
            return answered;
        }
        if self.confirm.is_some() {
            return self.handle_confirm_key(key);
        }
        if self.react.is_some() {
            return self.handle_react_key(key);
        }
        if self.input.is_some() {
            return self.handle_input_key(key);
        }
        if self.sharing.is_some() {
            return self.handle_share_key(key);
        }
        if self.palette.is_some() {
            return self.handle_palette_key(key);
        }
        if self.publish.is_some() {
            return self.handle_publish_key(key);
        }
        if self.brief.is_some() {
            return self.handle_brief_key(paged(key));
        }
        if self.filtering {
            return self.handle_filter_key(key);
        }
        if self.search.as_ref().is_some_and(|s| s.typing) {
            self.handle_search_key(key);
            return vec![];
        }
        if key.code == KeyCode::Esc && self.search.is_some() && self.focus == Focus::Review {
            self.clear_search();
            return vec![];
        }
        match self.keymap.feed(self.held.take(), key) {
            crate::keymap::Feed::Hold(first) => {
                self.held = Some(first);
                vec![]
            }
            crate::keymap::Feed::Keys(keys) => keys.into_iter().flat_map(|key| self.dispatch(paged(key))).collect(),
        }
    }

    /// A key after the user's bindings turned it into revu's own, counted when `[usage]` is on.
    fn dispatch(&mut self, key: KeyEvent) -> Vec<Action> {
        let named = self.usage_name(key);
        self.repeat = Some(super::repeat::Repeat::after(self.repeat, key.code, self.now));
        let was_zen = self.zen;
        let actions = self.dispatch_key(key);
        self.count_key(named, was_zen);
        actions
    }

    /// Prefixes first, then the keys every pane shares, then the pane's own.
    fn dispatch_key(&mut self, key: KeyEvent) -> Vec<Action> {
        if let Some(prefix) = self.pending.take() {
            return self.handle_prefixed(prefix, key);
        }
        if let Some(mode) = palette_key(key) {
            self.open_palette(mode);
            return vec![];
        }
        match key.code {
            KeyCode::Char(':') => self.open_palette(crate::tui::palette::Mode::Commands),
            KeyCode::Char('q') if self.focus != Focus::Queue && self.open.as_ref().is_some_and(super::Open::side_open) => {
                return self.handle_side_key(KeyEvent::from(KeyCode::Esc));
            }
            KeyCode::Char('q') => return self.quit_key(super::quit::QuitKey::Q),
            KeyCode::Char('?') => self.help = Some(Help::default()),
            KeyCode::Char('Y') => self.start_share(None),
            KeyCode::Char('h') | KeyCode::Left => return self.focus_left(),
            KeyCode::Char('l') | KeyCode::Right => return self.focus_right(),
            KeyCode::Char('z' | '[' | ']') if self.focus != Focus::Side || self.tree_open() => self.pending = key.code.as_char(),
            KeyCode::Char('z') if self.outline_keys() => self.pending = Some('z'),
            KeyCode::Char('a') if self.focus != Focus::Queue && self.open.is_some() && !self.answer_open() && !self.outline_keys() => {
                self.pending = Some('a');
            }
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

    /// Left, toward the queue; from the diff it also leaves zen, which hides the queue.
    fn focus_left(&mut self) -> Vec<Action> {
        let released = if self.focus == Focus::Review { self.leave_zen() } else { vec![] };
        self.focus = match self.focus {
            Focus::Side => Focus::Review,
            _ => Focus::Queue,
        };
        released
    }

    /// From the queue, right opens the selected MR, as `enter` does, so the diff always matches the row.
    /// In the diff, a marked line (or a file's outdated threads) opens the pane on it, unless the pane
    /// lists every thread: the reader goes back to the list, which `enter` on the line would replace.
    fn focus_right(&mut self) -> Vec<Action> {
        let keeps_list = self.open.as_ref().is_some_and(super::Open::lists_every_thread);
        if self.focus == Focus::Review && !keeps_list && (self.open_pane_here() || self.open_outdated_here()) {
            return vec![];
        }
        self.focus = match (self.focus, &self.open) {
            (Focus::Queue, _) => return self.open_selected(),
            (Focus::Review, Some(open)) if open.side_open() => Focus::Side,
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

    pub(super) fn handle_queue_key(&mut self, key: KeyEvent) -> Vec<Action> {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.queue_move(self.step()),
            KeyCode::Char('k') | KeyCode::Up => self.queue_move(-self.step()),
            KeyCode::Char('g') => self.queue_first(),
            KeyCode::Char('G') => self.queue_last(),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => self.queue_move(HALF_PAGE),
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => self.queue_move(-HALF_PAGE),
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => self.queue_move(full_page(self.areas.queue)),
            KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => self.queue_move(-full_page(self.areas.queue)),
            KeyCode::Char('/') => self.filtering = true,
            KeyCode::Char('*') => return self.toggle_scope(),
            KeyCode::Char('s') => return self.sort_queue(),
            KeyCode::Char('\'') => self.pending = Some('\''),
            KeyCode::Char(c @ '1'..='9') => self.apply_view(c),
            KeyCode::Char('S') => return self.group_queue(),
            KeyCode::Char('b') => return self.toggle_pin(),
            KeyCode::Char('i') => self.open_brief_from_queue(),
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

    pub(super) fn refresh_queue(&mut self) -> Vec<Action> {
        if self.queue_loading {
            return vec![];
        }
        self.queue_loading = true;
        self.queue_failed = false;
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
        self.queue_failed = false;
        vec![Action::LoadQueue { scope: self.scope(), from_cache: true }]
    }

    pub(super) fn handle_review_key(&mut self, key: KeyEvent) -> Vec<Action> {
        if self.open.is_none() {
            if key.code == KeyCode::Esc {
                self.focus = Focus::Queue;
            }
            return vec![];
        }
        if self.prose_open() {
            return self.handle_prose_key(key);
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
            KeyCode::Char('j') | KeyCode::Down => self.review_move(self.step()),
            KeyCode::Char('k') | KeyCode::Up => self.review_move(-self.step()),
            KeyCode::Char('g') => self.review_first(),
            KeyCode::Char('G') => self.review_last(),
            KeyCode::Char('/') => self.start_search(),
            KeyCode::Char('n') if self.search.is_some() => self.search_step(true),
            KeyCode::Char('N') if self.search.is_some() => self.search_step(false),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => self.review_move(HALF_PAGE),
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => self.review_move(-HALF_PAGE),
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => self.review_move(full_page(self.areas.review)),
            KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => self.review_move(-full_page(self.areas.review)),
            KeyCode::Char('D') => return self.toggle_side_by_side(),
            KeyCode::Char('>') => return self.cycle_peek(true),
            KeyCode::Char('<') => return self.cycle_peek(false),
            KeyCode::Char('t') => self.toggle_tree(),
            KeyCode::Char('T') => self.toggle_every_thread(),
            KeyCode::Char('p') => return self.toggle_pipeline(),
            KeyCode::Char('O') => return self.toggle_outline(),
            KeyCode::Char('W') => self.toggle_whitespace(),
            KeyCode::Char('+') => {
                self.open_react_here();
            }
            KeyCode::Char('=') => return self.expand_context(),
            KeyCode::Char('w') => {
                self.wrap = !self.wrap;
                self.toast(if self.wrap { "long lines wrap" } else { "long lines end in …" });
            }
            KeyCode::Tab => self.review_jump(true, |r| matches!(r, Row::File { .. })),
            KeyCode::BackTab => self.review_jump(false, |r| matches!(r, Row::File { .. })),
            KeyCode::Enter => return self.enter_review_row(),
            KeyCode::Esc | KeyCode::Char('x') if self.answer_open() => self.close_answer(),
            KeyCode::Esc | KeyCode::Char('x') if self.open.as_ref().is_some_and(|o| o.pane.is_some()) => self.close_pane(),
            KeyCode::Esc if self.zen => return self.leave_zen(),
            KeyCode::Esc => self.focus = Focus::Queue,
            KeyCode::Char('r') => return self.refresh_open(),
            KeyCode::Char('i') => self.open_brief_from_review(),
            KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => return self.view_here(crate::review::Side::New),
            KeyCode::Char('v') => return self.open_prose(),
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
                ('[' | ']', 'r') => return self.walk_reviews(prefix == ']'),
                ('[' | ']', 'm') => return self.step_mr(prefix == ']'),
                ('z', 'o') => Some(true),
                ('z', 'c') => Some(false),
                ('z', 'a') => None,
                ('z', 'z') => return self.toggle_zen(),
                _ => return vec![],
            };
            if let Some(actions) = self.fold_stack(open) {
                return actions;
            }
            return self.fold_section(open);
        }
        if prefix == 'a' {
            return self.ask_key(c);
        }
        if prefix == 'z' && self.outline_keys() {
            self.fold_outline(c);
            return vec![];
        }
        if self.prose_open() {
            return self.prose_prefixed(prefix, c);
        }
        let forward = prefix == ']';
        match (prefix, c) {
            ('z', 'a') => return self.fold_at_cursor(None),
            ('z', 'o') => return self.fold_at_cursor(Some(true)),
            ('z', 'c') => return self.fold_at_cursor(Some(false)),
            ('z', 'h') => self.header_folded = !self.header_folded,
            ('z', 'z') => return self.toggle_zen(),
            ('z', 'v') => return self.toggle_viewed(),
            ('z', 'M') => return self.fold_all(true),
            ('z', 'R') => return self.fold_all(false),
            ('[' | ']', 'r') => return self.walk_reviews(forward),
            ('[' | ']', 'm') => return self.step_mr(forward),
            ('[' | ']', 'c') => self.review_jump(forward, |r| matches!(r, Row::Hunk { .. })),
            ('[' | ']', 'n') => return self.jump_to_marked(forward, Threads::Open),
            ('[' | ']', 'N') => return self.jump_to_marked(forward, Threads::Every),
            ('[' | ']', 'f') => {
                let wanted = self.files_with_unresolved();
                self.review_jump(forward, move |r| matches!(r, Row::File { index, .. } if wanted.contains(index)));
            }
            _ => {}
        }
        vec![]
    }

    /// The outline reads `a` itself while it has the keys.
    fn outline_keys(&self) -> bool {
        self.focus == Focus::Side && self.outline_open()
    }

    fn tree_open(&self) -> bool {
        self.open.as_ref().is_some_and(|o| o.tree.is_some())
    }

    pub(super) fn handle_side_key(&mut self, key: KeyEvent) -> Vec<Action> {
        if self.answer_open() {
            return self.handle_answer_key(key);
        }
        if self.pipeline_open() {
            return self.handle_pipeline_key(key);
        }
        if self.outline_open() {
            return self.handle_outline_key(key);
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

    /// `?`, or the user's key for it, widens the focused pane's keys to every key, then closes the list.
    fn help_key(&self, open: Help, key: KeyEvent) -> Option<Help> {
        if self.is_help_key(key) {
            return (!open.every_key).then_some(Help { scroll: 0, every_key: true });
        }
        let last = help::last_row(open.every_key, self.focus);
        scroll_key(open.scroll, last, key).map(|scroll| Help { scroll, ..open })
    }

    fn is_help_key(&self, key: KeyEvent) -> bool {
        let crate::keymap::Feed::Keys(keys) = self.keymap.feed(None, key) else { return false };
        keys.first().is_some_and(|k| k.code == KeyCode::Char('?'))
    }
}

/// The scroll a moving key gives, kept at or under `last`; `None` for any other key.
pub(super) fn scroll_key(scroll: usize, last: usize, key: KeyEvent) -> Option<usize> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => Some(scroll.saturating_add(1).min(last)),
        KeyCode::Char('k') | KeyCode::Up => Some(scroll.saturating_sub(1)),
        KeyCode::Char('d') if ctrl => Some(scroll.saturating_add(HALF_PAGE.unsigned_abs()).min(last)),
        KeyCode::Char('u') if ctrl => Some(scroll.saturating_sub(HALF_PAGE.unsigned_abs())),
        KeyCode::Char('f') if ctrl => Some((scroll + 2 * HALF_PAGE.unsigned_abs()).min(last)),
        KeyCode::Char('b') if ctrl => Some(scroll.saturating_sub(2 * HALF_PAGE.unsigned_abs())),
        KeyCode::Char('g') => Some(0),
        KeyCode::Char('G') => Some(last),
        _ => None,
    }
}

/// `PageDown` pages down as `ctrl-f` does and `PageUp` up as `ctrl-b`, as in a pager; space keeps
/// the half page of `ctrl-d`. Wherever no text is typed.
fn paged(key: KeyEvent) -> KeyEvent {
    match (key.code, key.modifiers) {
        (KeyCode::PageDown, _) => KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
        (KeyCode::PageUp, _) => KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
        (KeyCode::Char(' '), KeyModifiers::NONE) => KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
        _ => key,
    }
}

/// The rows a full page moves in a pane drawn on `area`: those inside its border but the kept ones,
/// or twice a half page before the pane was ever drawn.
pub(super) fn full_page(area: Rect) -> isize {
    if area.is_empty() {
        return 2 * HALF_PAGE;
    }
    area.height.saturating_sub(2 + KEPT_ROWS).max(1) as isize
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crate::tui::app::test_support::*;
    use ratatui::layout::Rect;

    #[test]
    fn zh_folds_the_review_header_to_one_row() {
        let mut app = with_review();
        press(&mut app, "zh");
        assert!(app.header_folded);
        let screen = render(&mut app, 120, 24);
        assert!(screen.contains("▸ "), "{screen}");
        press(&mut app, "zh");
        assert!(!app.header_folded);
    }

    #[test]
    fn r_refreshes_once_and_o_y_take_the_mr_url() {
        let mut app = with_queue();
        assert_eq!(press(&mut app, "r"), vec![Action::LoadQueue { scope: None, from_cache: false }]);
        assert_eq!(press(&mut app, "r"), vec![], "not while one is in flight");
        app.apply(Incoming::Queue { scope: None, me: "nina".into(), sections: sections(), opened: HashMap::new(), cached: false });
        let url = "https://gitlab.com/acme/widgets/-/merge_requests/42".to_owned();
        assert_eq!(press(&mut app, "o"), vec![Action::OpenUrl(url.clone())]);
        assert_eq!(press(&mut app, "y"), vec![Action::Yank(url)]);
    }

    #[test]
    fn h_l_and_esc_move_the_focus() {
        let mut app = with_review();
        press(&mut app, "h");
        assert_eq!(app.focus, Focus::Queue);
        press(&mut app, "l");
        assert_eq!(app.focus, Focus::Review);
        press(&mut app, "l");
        assert_eq!(app.focus, Focus::Side, "the file row opens its outdated threads");
        app.handle_key(code(KeyCode::Esc));
        assert_eq!(app.focus, Focus::Review, "esc closes the pane first");
        app.handle_key(code(KeyCode::Esc));
        assert_eq!(app.focus, Focus::Queue);
    }

    #[test]
    fn help_and_quit() {
        let mut app = app();
        press(&mut app, "?");
        assert_eq!(app.help, Some(Help::default()));
        press(&mut app, "jjk");
        assert_eq!(app.help, Some(Help { scroll: 1, every_key: false }), "moving keys scroll the list");
        press(&mut app, "G");
        assert_eq!(app.help, Some(Help { scroll: crate::tui::help::last_row(false, Focus::Queue), every_key: false }));
        press(&mut app, "?");
        assert_eq!(app.help, Some(Help { scroll: 0, every_key: true }), "a second `?` lists every key from the top");
        press(&mut app, "G");
        assert_eq!(app.help.map(|h| h.scroll), Some(crate::tui::help::last_row(true, Focus::Queue)));
        press(&mut app, "?");
        assert_eq!(app.help, None, "`?` on every key closes it");
        press(&mut app, "?x");
        assert_eq!(app.help, None, "any other key closes it");
        app.handle_key(ctrl('c'));
        app.handle_key(ctrl('c'));
        assert!(app.should_quit);
    }

    /// The group titles the open key list shows, as its uppercase headers.
    fn help_titles(app: &mut App) -> Vec<&'static str> {
        let screen = render(app, 160, 60);
        crate::tui::help::GROUPS.iter().map(|g| g.title).filter(|t| screen.contains(&format!("  {} ", t.to_uppercase()))).collect()
    }

    #[test]
    fn the_first_question_mark_lists_the_keys_of_the_focused_pane_and_the_second_every_key() {
        let mut queue = with_queue();
        press(&mut queue, "?");
        assert_eq!(help_titles(&mut queue), ["move", "queue", "search & app"]);
        let mut diff = with_review();
        assert_eq!(diff.focus, Focus::Review);
        press(&mut diff, "?");
        assert_eq!(help_titles(&mut diff), ["move", "view", "comment & publish", "ask claude", "search & app"]);
        let mut thread = with_review();
        press(&mut thread, "]N");
        thread.handle_key(code(KeyCode::Enter));
        assert_eq!(thread.focus, Focus::Side);
        press(&mut thread, "?");
        assert_eq!(help_titles(&mut thread), ["comment & publish", "thread pane", "outline pane", "ask claude", "search & app"]);
        press(&mut thread, "?");
        assert_eq!(help_titles(&mut thread), crate::tui::help::GROUPS.map(|g| g.title));
        thread.handle_key(code(KeyCode::Esc));
        assert_eq!(thread.help, None);
    }

    #[test]
    fn right_from_the_queue_opens_the_selected_mr_like_enter() {
        let mut app = with_queue();
        assert_eq!(press(&mut app, "l"), vec![Action::Open(mr_key())]);
        assert_eq!(app.focus, Focus::Review);
        app.apply(Incoming::Review { key: mr_key(), review: Box::new(review()), cached: None });
        press(&mut app, "hj");
        let next = app.selected_mr().unwrap().key();
        assert_ne!(next, mr_key());
        assert_eq!(press(&mut app, "l"), vec![Action::Open(next)], "the diff must follow the queue row, never show the previous MR");
        assert_eq!(app.focus, Focus::Review);
    }

    fn with_keys(toml: &str) -> App {
        let mut app = with_review();
        app.keymap = keymap(toml);
        app
    }

    #[test]
    fn with_the_azerty_preset_parentheses_jump_like_brackets() {
        let mut plain = with_review();
        let before = plain.open.as_ref().unwrap().row().cloned();
        press(&mut plain, ")n");
        assert_eq!(plain.open.as_ref().unwrap().row().cloned(), before, "without the preset `)n` does nothing");
        let mut app = with_keys(r#"layout = "azerty""#);
        press(&mut app, ")c)c)n");
        assert_eq!(app.open.as_ref().unwrap().row(), Some(&Row::Header), "`)c` twice then `)n`, as `]c]c]n` would");
        press(&mut app, ")N");
        assert_eq!(app.open.as_ref().unwrap().row(), Some(&Row::Line { file: 0, hunk: 0, index: 1 }), "`)N` stops on the resolved thread");
        press(&mut app, "(N");
        assert_eq!(app.open.as_ref().unwrap().row(), Some(&Row::Header), "`(N` goes back");
        press(&mut app, "]N");
        assert_eq!(app.open.as_ref().unwrap().row(), Some(&Row::Line { file: 0, hunk: 0, index: 1 }), "brackets keep working");
    }

    #[test]
    fn a_bound_key_does_what_its_action_does_and_a_two_key_one_waits() {
        let mut app = with_keys(r#"bind = { next_hunk = "F", next_any_thread = ["ft", "ctrl-e"] }"#);
        press(&mut app, "F");
        assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::Hunk { index: 0, .. })));
        press(&mut app, "f");
        assert!(app.held.is_some(), "`f` waits for its second key");
        press(&mut app, "t");
        assert!(app.held.is_none());
        assert_eq!(
            app.open.as_ref().unwrap().row(),
            Some(&Row::Line { file: 0, hunk: 0, index: 1 }),
            "`ft` is `]N`: the marked line after the hunk"
        );
        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
        assert_eq!(app.open.as_ref().unwrap().row(), Some(&Row::Header), "`ctrl-e` is `]N` too, and wraps to the MR's thread");
    }

    #[test]
    fn a_key_bound_to_help_widens_and_closes_the_list_like_the_question_mark() {
        let mut app = with_queue();
        app.keymap = keymap("[bind]\nhelp = \"F\"");
        press(&mut app, "F");
        assert_eq!(app.help, Some(Help::default()));
        press(&mut app, "F");
        assert_eq!(app.help, Some(Help { scroll: 0, every_key: true }));
        press(&mut app, "F");
        assert_eq!(app.help, None);
    }

    #[test]
    fn page_keys_page_like_their_ctrl_key_and_count_as_it() {
        let selected = |app: &App| app.open.as_ref().unwrap().selected;
        let after = |key: KeyEvent| {
            let mut app = with_long_review();
            app.handle_key(key);
            selected(&app)
        };
        assert_eq!((after(code(KeyCode::PageDown)), after(key(' '))), (after(ctrl('f')), after(ctrl('d'))));
        let mut paged = counting();
        paged.handle_key(code(KeyCode::PageDown));
        paged.handle_key(code(KeyCode::PageUp));
        assert_eq!(selected(&paged), selected(&with_long_review()));
        let counts = paged.take_usage().unwrap();
        assert_eq!((counts.actions.get("full_page_down"), counts.actions.get("full_page_up")), (Some(&1), Some(&1)));
    }

    #[test]
    fn ctrl_f_and_ctrl_b_move_the_diff_cursor_by_the_rows_the_pane_shows() {
        let mut app = with_long_review();
        render(&mut app, 120, 30);
        let top = app.open.as_ref().unwrap().selected;
        let page = usize::from(app.areas.review.height) - 4;
        app.handle_key(ctrl('f'));
        assert_eq!(app.open.as_ref().unwrap().selected, top + page);
        app.handle_key(ctrl('b'));
        assert_eq!(app.open.as_ref().unwrap().selected, top);
    }

    #[test]
    fn a_full_page_keeps_two_rows_inside_the_border_and_is_twice_a_half_page_before_any_frame() {
        assert_eq!(super::full_page(Rect::new(0, 0, 80, 24)), 24 - 2 - 2);
        assert_eq!(super::full_page(Rect::new(0, 0, 80, 3)), 1);
        assert_eq!(super::full_page(Rect::default()), 2 * super::HALF_PAGE);
    }
}
