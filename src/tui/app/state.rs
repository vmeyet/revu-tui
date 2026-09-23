use super::{Action, Brief, Focus, Input, MrKey, Open, Publish, Toast};
use crate::forge::Sections;
use crate::tui::field::Field;
use crate::tui::theme::Theme;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const QUEUE_EVERY: Duration = Duration::from_secs(60);
const MR_EVERY: Duration = Duration::from_secs(60);
const DISCUSSIONS_EVERY: Duration = Duration::from_secs(30);
const BACKOFF: Duration = Duration::from_secs(300);
const SLOW_DOWN: u32 = 5;

/// What the app is started with; everything else it learns from `Incoming`.
#[derive(Clone, Debug)]
pub struct Settings {
    pub theme: Theme,
    pub host: String,
    pub me: String,
    /// The project of the checkout `revu` runs in; `None` outside one or with `--all`.
    pub project: Option<String>,
    /// The terminal's own background, when it said: `:set theme=` tints the new theme from it.
    pub ground: Option<u32>,
    /// Jev is switched on and holds a key: the queue and the files get its marks.
    pub triage: bool,
    /// Claude's model, when it is switched on and holds a key: `a` asks it.
    pub ask: Option<String>,
    /// `[notify] enabled`: a macOS notification when an MR lands in To review.
    pub notify: bool,
    /// The hosts the queue draws from: which forge each MR is on, and its row's tag.
    pub hosts: crate::forge::Hosts,
    /// `[keys]`: the user's keys, each standing for revu's default ones.
    pub keymap: crate::keymap::Keymap,
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
    /// Long diff lines wrap under their text instead of ending in `…`, `w`.
    pub wrap: bool,
    /// Reading mode, `zz`: the queue hides and the diff sits centered.
    pub reading: bool,
    /// What the forge's rate limit says, copied in by the loop each tick.
    pub rate: crate::forge::RateLimit,
    /// What changed in the open MR since the reader last pressed a key: `● 2 new notes`.
    pub news: Option<String>,
    /// A commit waiting for `y`; every key answers it first.
    pub confirm: Option<super::Confirm>,
    pub notify: bool,
    pub hosts: crate::forge::Hosts,
    /// What To review held at the last fresh answer, so only newcomers are announced.
    pub seen: Option<super::notify::Seen>,
    pub closed_sections: std::collections::BTreeSet<&'static str>,
    pub queue_loading: bool,
    pub open: Option<Open>,
    /// The MR being fetched for the first time; the review pane shows a spinner until it lands.
    pub opening: Option<MrKey>,
    /// First half of `z`, `[` or `]`.
    /// The first key of a two-key binding of the user's, waiting for its second.
    pub held: Option<crossterm::event::KeyEvent>,
    pub keymap: crate::keymap::Keymap,
    pub pending: Option<char>,
    /// The input row is open for this; `buffer` holds what is typed.
    pub input: Option<Input>,
    pub buffer: Field,
    /// Text typed for a target and left with `esc`, by target: it comes back when the box opens there again.
    pub unsent: HashMap<String, String>,
    /// Where the compose box was opened from: the keys go back there once it closes.
    pub compose_from: Focus,
    pub publish: Option<Publish>,
    /// The MR description modal.
    pub brief: Option<Brief>,
    /// Where the `!iid`s were drawn this frame, so the loop can make them clickable.
    pub links: Vec<crate::tui::ui::Link>,
    /// A file ready for the reader's program; the loop takes it and hands over the terminal.
    pub viewing: Option<crate::open::View>,
    /// What the editor's text turned into; the loop drains it after `apply`.
    pub composed: Vec<Action>,
    /// The key list, open, with how far it is scrolled.
    pub help: Option<usize>,
    pub palette: Option<crate::tui::palette::Palette>,
    pub palette_history: Vec<String>,
    pub jump: Option<crate::tui::jump::Jump>,
    pub ground: Option<u32>,
    /// Jev ranks the queue and the files: on with `[ai.typesafe]` and a key, off for good once it fails.
    pub triage: bool,
    /// Claude's model while `a` may ask it; `:ai off` clears it for the session.
    pub ask_model: Option<String>,
    /// A question already went out this session: the first one says where the MR goes.
    pub asked: bool,
    /// The id of the last answer started.
    pub next_answer: u64,
    /// What Jev said about each queue MR.
    pub verdicts: HashMap<MrKey, crate::ai::triage::Verdict>,
    /// MRs Jev is being asked about right now, so a new queue does not ask twice.
    pub triage_asked: std::collections::HashSet<MrKey>,
    /// What Jev read in each opened MR, with the head commit it read.
    pub readings: HashMap<MrKey, (String, crate::ai::triage::Reading)>,
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
            wrap: false,
            reading: false,
            rate: crate::forge::RateLimit::default(),
            news: None,
            confirm: None,
            notify: settings.notify,
            hosts: settings.hosts,
            seen: None,
            closed_sections: std::collections::BTreeSet::from(["DONE"]),
            queue_loading: true,
            open: None,
            opening: None,
            held: None,
            keymap: settings.keymap,
            pending: None,
            input: None,
            buffer: Field::default(),
            unsent: HashMap::new(),
            compose_from: Focus::default(),
            publish: None,
            brief: None,
            links: vec![],
            viewing: None,
            composed: vec![],
            help: None,
            palette: None,
            palette_history: vec![],
            jump: None,
            ground: settings.ground,
            triage: settings.triage,
            ask_model: settings.ask,
            asked: false,
            next_answer: 0,
            verdicts: HashMap::new(),
            triage_asked: std::collections::HashSet::new(),
            readings: HashMap::new(),
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
        actions.extend(self.pipeline_tick());
        actions
    }

    pub(super) fn schedule_queue(&mut self) {
        self.poll.queue_due = Some(self.now + self.paced(QUEUE_EVERY));
    }

    pub(super) fn schedule_review(&mut self) {
        self.poll.mr_due = Some(self.now + self.paced(MR_EVERY));
        self.poll.discussions_due = Some(self.now + self.paced(DISCUSSIONS_EVERY));
    }

    pub(super) fn schedule_discussions(&mut self) {
        self.poll.discussions_due = Some(self.now + self.paced(DISCUSSIONS_EVERY));
    }

    /// Polls five times slower while the forge says few requests are left, so the window lasts.
    fn paced(&self, every: Duration) -> Duration {
        if self.rate.is_low() { every * SLOW_DOWN } else { every }
    }

    pub(super) fn back_off(&mut self) {
        let later = Some(self.now + BACKOFF);
        self.poll = Poll { queue_due: later, mr_due: later, discussions_due: later };
    }

    pub fn loading(&self) -> bool {
        self.queue_loading || self.opening.is_some()
    }
}
