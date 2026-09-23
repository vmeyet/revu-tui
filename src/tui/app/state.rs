use super::{Action, Brief, Focus, Input, MrKey, Open, Publish, Toast};
use crate::forge::{Kind, Sections};
use crate::tui::field::Field;
use crate::tui::theme::Theme;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const QUEUE_EVERY: Duration = Duration::from_secs(60);
const MR_EVERY: Duration = Duration::from_secs(60);
const DISCUSSIONS_EVERY: Duration = Duration::from_secs(30);
const BACKOFF: Duration = Duration::from_secs(300);

/// What the app is started with; everything else it learns from `Incoming`.
#[derive(Clone, Debug)]
pub struct Settings {
    pub theme: Theme,
    pub host: String,
    /// Which forge `host` runs: how an MR is named and how a line is linked.
    pub kind: Kind,
    pub me: String,
    /// The project of the checkout `revu` runs in; `None` outside one or with `--all`.
    pub project: Option<String>,
    /// The terminal's own background, when it said: `:set theme=` tints the new theme from it.
    pub ground: Option<u32>,
}

/// When each background refresh is due; `None` until the first answer arrived.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Poll {
    pub queue_due: Option<Instant>,
    pub mr_due: Option<Instant>,
    pub discussions_due: Option<Instant>,
}

#[derive(Debug)]
pub struct App {
    pub theme: Theme,
    pub host: String,
    pub kind: Kind,
    pub me: String,
    pub focus: Focus,
    /// The checkout's project; the queue shows only it unless `everywhere`.
    pub project: Option<String>,
    pub everywhere: bool,
    pub sections: Option<Sections>,
    /// When each MR was last opened here, so the queue can mark what moved since.
    pub opened: HashMap<MrKey, DateTime<Utc>>,
    pub queue_selected: usize,
    pub queue_scroll: usize,
    pub filter: String,
    /// The filter row is taking keys.
    pub filtering: bool,
    /// Queue sections folded to their header; Done starts folded.
    /// The review header on one row instead of two.
    pub header_folded: bool,
    pub closed_sections: std::collections::BTreeSet<&'static str>,
    pub queue_loading: bool,
    pub open: Option<Open>,
    /// The MR being fetched for the first time; the review pane shows a spinner until it lands.
    pub opening: Option<MrKey>,
    /// First half of `z`, `[` or `]`.
    pub pending: Option<char>,
    /// The input row is open for this; `buffer` holds what is typed.
    pub input: Option<Input>,
    pub buffer: Field,
    pub publish: Option<Publish>,
    /// The MR description modal.
    pub brief: Option<Brief>,
    /// Where the `!iid`s were drawn this frame, so the loop can make them clickable.
    pub links: Vec<crate::tui::ui::Link>,
    /// What the editor's text turned into; the loop drains it after `apply`.
    pub composed: Vec<Action>,
    /// The key list, open, with how far it is scrolled.
    pub help: Option<usize>,
    pub palette: Option<crate::tui::palette::Palette>,
    pub palette_history: Vec<String>,
    pub jump: Option<crate::tui::jump::Jump>,
    pub ground: Option<u32>,
    pub toast: Option<Toast>,
    /// Since when refreshes fail while a cached view is shown.
    pub offline: Option<Instant>,
    pub poll: Poll,
    pub should_quit: bool,
    pub started: Instant,
    /// Set by the event loop each time it wakes, so nothing below reads the clock.
    pub now: Instant,
    /// Wall clock, for ages next to notes and MRs; set with `now`.
    pub today: DateTime<Utc>,
}

impl App {
    pub fn new(settings: Settings) -> Self {
        let now = Instant::now();
        Self {
            theme: settings.theme,
            host: settings.host,
            kind: settings.kind,
            me: settings.me,
            focus: Focus::default(),
            everywhere: settings.project.is_none(),
            project: settings.project,
            sections: None,
            opened: HashMap::new(),
            queue_selected: 0,
            queue_scroll: 0,
            filter: String::new(),
            filtering: false,
            header_folded: false,
            closed_sections: std::collections::BTreeSet::from(["DONE"]),
            queue_loading: true,
            open: None,
            opening: None,
            pending: None,
            input: None,
            buffer: Field::default(),
            publish: None,
            brief: None,
            links: vec![],
            composed: vec![],
            help: None,
            palette: None,
            palette_history: vec![],
            jump: None,
            ground: settings.ground,
            toast: None,
            offline: None,
            poll: Poll::default(),
            should_quit: false,
            started: now,
            now,
            today: Utc::now(),
        }
    }

    pub fn start(&self) -> Vec<Action> {
        vec![Action::LoadQueue { scope: self.scope(), from_cache: true }]
    }

    /// The project the queue is limited to, `None` for every project.
    pub fn scope(&self) -> Option<String> {
        self.project.clone().filter(|_| !self.everywhere)
    }

    /// Called on every tick: what the clock says is due, at most once per due date.
    pub fn tick(&mut self) -> Vec<Action> {
        let mut actions = vec![];
        if self.poll.queue_due.is_some_and(|due| self.now >= due) && !self.queue_loading {
            self.queue_loading = true;
            self.poll.queue_due = None;
            actions.push(Action::LoadQueue { scope: self.scope(), from_cache: false });
        }
        let Some(key) = self.open.as_ref().map(|o| o.key.clone()) else { return actions };
        if self.poll.mr_due.is_some_and(|due| self.now >= due) {
            self.poll.mr_due = None;
            actions.push(Action::RefreshMr(key.clone()));
        }
        if self.poll.discussions_due.is_some_and(|due| self.now >= due) {
            self.poll.discussions_due = None;
            actions.push(Action::RefreshDiscussions(key));
        }
        actions
    }

    pub(super) fn schedule_queue(&mut self) {
        self.poll.queue_due = Some(self.now + QUEUE_EVERY);
    }

    pub(super) fn schedule_review(&mut self) {
        self.poll.mr_due = Some(self.now + MR_EVERY);
        self.poll.discussions_due = Some(self.now + DISCUSSIONS_EVERY);
    }

    pub(super) fn schedule_discussions(&mut self) {
        self.poll.discussions_due = Some(self.now + DISCUSSIONS_EVERY);
    }

    pub(super) fn back_off(&mut self) {
        let later = Some(self.now + BACKOFF);
        self.poll = Poll { queue_due: later, mr_due: later, discussions_due: later };
    }

    pub fn loading(&self) -> bool {
        self.queue_loading || self.opening.is_some()
    }
}
