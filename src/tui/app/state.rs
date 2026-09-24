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
    /// How the terminal draws pictures, asked once at start; `None` when it cannot or `[tui] images = false`.
    pub pictures: Option<ratatui_image::picker::Picker>,
    /// `[tui] queue`: two-line rows, or one.
    pub queue_layout: crate::config::QueueLayout,
    /// How wide the diff reads in zen.
    pub zen_width: u16,
    /// `[queue.views]`, in name order: saved filters `'` and the digits apply.
    /// `[share]`: where `Y` can post an MR, the bare target first.
    pub share: Vec<crate::share::Target>,
    pub views: Vec<(String, String)>,
    /// `[queue] prefetch`: how many MRs that need me are loaded ahead; 0 turns it off.
    pub prefetch: usize,
    /// `[tui] ascii`: reactions in plain words, for terminals that draw emoji at the wrong width.
    pub ascii: bool,
    /// `[keys] quit_confirm`: `q` and `ctrl-c` need a second press.
    pub quit_confirm: bool,
    /// `[usage] enabled`: count the actions used and the time per screen.
    pub usage: bool,
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
    /// The saved view the filter came from, named in the queue title until the filter changes.
    pub view: Option<String>,
    /// Where `Y` can post an MR.
    pub share_targets: Vec<crate::share::Target>,
    /// The post being prepared; nothing is sent before its preview gets a `y`.
    pub sharing: Option<super::Sharing>,
    pub views: Vec<(String, String)>,
    /// The filter row is taking keys.
    pub filtering: bool,
    /// Queue sections folded to their header; Done starts folded.
    /// The review header on one row instead of two.
    pub header_folded: bool,
    /// Long diff lines wrap under their text instead of ending in `…`, `w`.
    pub wrap: bool,
    /// Zen, `zz`: the diff alone in a centred column, no frames, no status line, nothing pulsing.
    pub zen: bool,
    /// How wide the diff reads in zen, `[tui] zen_width`.
    pub zen_width: u16,
    /// The MR zen just switched to, shown on top for a moment: when, and what it says.
    pub zen_switch: Option<(Instant, String)>,
    /// Notifications that arrived in zen, sent when it ends.
    pub quiet_notices: Vec<Action>,
    /// What the forge's rate limit says, copied in by the loop each tick.
    pub rate: crate::forge::RateLimit,
    /// What changed in the open MR since the reader last pressed a key: `● 2 new notes`.
    pub news: Option<String>,
    /// A commit waiting for `y`; every key answers it first.
    pub confirm: Option<super::Confirm>,
    /// The reaction picker `+` opened on a note; every key answers it first.
    pub react: Option<super::Pick>,
    pub ascii: bool,
    pub notify: bool,
    pub hosts: crate::forge::Hosts,
    /// What To review held at the last fresh answer, so only newcomers are announced.
    pub seen: Option<super::notify::Seen>,
    pub closed_sections: std::collections::BTreeSet<&'static str>,
    /// How rows sit inside the sections: `s` sorts, `S` groups by author.
    pub queue_view: super::QueueView,
    pub queue_layout: crate::config::QueueLayout,
    pub queue_loading: bool,
    /// How many MRs to load ahead after each fresh queue.
    pub prefetch_limit: usize,
    /// A plan held back while an MR was being opened: it goes out once that MR arrived.
    pub prefetch_due: bool,
    /// The next MR that needs me, offered in the status line after a publish.
    pub offer: Option<MrKey>,
    /// Viewed files per started MR: the queue shows how far each review went.
    pub viewed_counts: HashMap<MrKey, usize>,
    /// The cursor's place as last saved, so a resting cursor is written once.
    pub spot_saved: Option<(MrKey, super::Spot)>,
    /// Where the cursor rests and since when: it is saved once it rested a moment.
    pub spot_pending: Option<(super::Spot, Instant)>,
    pub open: Option<Open>,
    /// The MR being fetched for the first time; the review pane shows a spinner until it lands.
    pub opening: Option<MrKey>,
    /// First half of `z`, `[` or `]`.
    /// The first key of a two-key binding of the user's, waiting for its second.
    pub held: Option<crossterm::event::KeyEvent>,
    /// A quit key pressed once, and when: the same key again inside the window quits.
    pub quitting: Option<(super::quit::QuitKey, Instant)>,
    pub quit_confirm: bool,
    /// What this session counted since the last flush; `None` when `[usage]` is off.
    pub usage: Option<crate::usage::Tally>,
    /// The last time screen time was counted.
    pub usage_at: Instant,
    /// How many j or k in a row moved through a diff that has hunks or threads to jump to.
    pub walk: u32,
    /// Zen was on at some point this session.
    pub zen_seen: bool,
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
    /// Pictures of the comments the right pane shows, each fetched once.
    pub thumbs: crate::tui::images::Thumbs,
    /// A file ready for the reader's program; the loop takes it and hands over the terminal.
    pub viewing: Option<crate::open::View>,
    /// What the editor's text turned into; the loop drains it after `apply`.
    pub composed: Vec<Action>,
    /// The key list, open, with how far it is scrolled.
    pub help: Option<usize>,
    pub palette: Option<crate::tui::palette::Palette>,
    pub palette_history: Vec<String>,
    pub ground: Option<u32>,
    /// Jev ranks the queue and the files: on with `[ai.typesafe]` and a key, off for good once it fails.
    pub triage: bool,
    /// Claude's model while `a` may ask it; `:ai off` clears it for the session.
    pub ask_model: Option<String>,
    /// What the config switched on at start, so `:ai on` can bring it back after `:ai off`.
    pub ai_configured: (bool, Option<String>),
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
            view: None,
            share_targets: settings.share,
            sharing: None,
            views: settings.views,
            filtering: false,
            header_folded: false,
            wrap: false,
            zen: false,
            zen_width: settings.zen_width,
            zen_switch: None,
            quiet_notices: vec![],
            rate: crate::forge::RateLimit::default(),
            news: None,
            confirm: None,
            react: None,
            ascii: settings.ascii,
            notify: settings.notify,
            hosts: settings.hosts,
            seen: None,
            closed_sections: std::collections::BTreeSet::from(["DONE", "DRAFTS", "OTHER"]),
            queue_view: super::QueueView::default(),
            queue_layout: settings.queue_layout,
            prefetch_limit: settings.prefetch,
            prefetch_due: false,
            offer: None,
            viewed_counts: HashMap::new(),
            spot_saved: None,
            spot_pending: None,
            queue_loading: true,
            open: None,
            opening: None,
            held: None,
            quitting: None,
            quit_confirm: settings.quit_confirm,
            usage: settings.usage.then(crate::usage::Tally::default),
            usage_at: now,
            walk: 0,
            zen_seen: false,
            keymap: settings.keymap,
            pending: None,
            input: None,
            buffer: Field::default(),
            unsent: HashMap::new(),
            compose_from: Focus::default(),
            publish: None,
            brief: None,
            links: vec![],
            thumbs: settings.pictures.map_or_else(crate::tui::images::Thumbs::off, crate::tui::images::Thumbs::with),
            viewing: None,
            composed: vec![],
            help: None,
            palette: None,
            palette_history: vec![],
            ground: settings.ground,
            ai_configured: (settings.triage, settings.ask.clone()),
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
        self.count_time();
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
        actions.extend(self.spot_tick());
        actions.extend(self.picture_requests());
        actions
    }

    /// The pictures of the conversations the right pane shows that were never asked for.
    fn picture_requests(&mut self) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let Some((conversations, _, _)) = open.pane_view() else { return vec![] };
        let review = &open.review;
        let bodies = conversations.iter().flat_map(|c| {
            let notes =
                c.thread.as_deref().and_then(|id| review.thread(id)).into_iter().flat_map(|t| t.notes.iter().map(|n| n.body.as_str()));
            notes.chain(c.drafts.iter().map(|&d| review.drafts[d].body.as_str()))
        });
        let urls: Vec<String> = bodies.flat_map(crate::review::image::images_in).map(|image| image.url).collect();
        let key = open.key.clone();
        self.thumbs.wanted(urls).into_iter().map(|url| Action::LoadImage { key: key.clone(), url }).collect()
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
