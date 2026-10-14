use super::{Action, Brief, Focus, Input, MrKey, Open, Publish, Toast};
use crate::forge::Sections;
use crate::tui::field::Field;
use crate::tui::theme::Theme;
use chrono::{DateTime, Utc};
use std::collections::{BTreeSet, HashMap};
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
    /// How wide the diff reads in zen; `None` grows it with the screen.
    pub zen_width: Option<u16>,
    /// `[tui] narrow_width` and `wide_width`.
    pub widths: crate::config::Widths,
    /// `[tui] tap_bar`: a row of labels to tap under the compact layout.
    pub tap_bar: bool,
    /// `[queue.views]`, in name order: saved filters `'` and the digits apply.
    /// `[share]`: where `Y` can post an MR, the bare target first.
    pub share: Vec<crate::share::Target>,
    pub views: Vec<(String, String)>,
    /// `[queue] prefetch`: how many MRs that need me are loaded ahead; 0 turns it off.
    pub prefetch: usize,
    /// `[tui] ascii`: reactions in plain words, for terminals that draw emoji at the wrong width.
    pub ascii: bool,
    /// The reactions I gave on each host, read once at start: the picker offers the most used first.
    pub reactions: crate::reactions::History,
    /// `[keys] quit_confirm`: `q` and `ctrl-c` need a second press.
    pub quit_confirm: bool,
    /// `[usage] enabled`: count the actions used and the time per screen, from this tally on.
    pub usage: Option<crate::usage::Tally>,
    /// revu's own changelog, and how far the reader had got in it.
    pub whats_new: super::WhatsNew,
    /// The keys of the config this revu ignores, toasted at start.
    pub ignored_settings: Vec<String>,
    /// `[agent] enabled`: `a g` opens an agent session on an MR.
    pub agent: bool,
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
    /// Long diff lines wrap under their text instead of ending in `…`, once `w` chose; until then the layout does.
    pub wrap: Option<bool>,
    /// Zen, `zz`: the diff alone in a centred column, no frames, no status line, nothing pulsing.
    pub zen: bool,
    /// The last frame was too narrow for the right pane beside the diff.
    pub narrow: bool,
    /// The last frame showed the compact layout: one page at a time, full screen.
    pub compact: bool,
    /// The last frame was compact and too short for the rows around the page.
    pub short: bool,
    /// `[tui] tap_bar`: the compact layout ends on a row of labels to tap.
    pub tap_bar: bool,
    /// The labels of the tap bar, as this frame drew them.
    pub taps: Vec<super::TapSpot>,
    /// How wide the diff reads in zen, `[tui] zen_width`; `None` grows it with the screen.
    pub zen_width: Option<u16>,
    /// The terminal widths the layout turns on, `[tui] narrow_width` and `wide_width`.
    pub widths: crate::config::Widths,
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
    /// The reactions I gave on each host, counted as I give them.
    pub reactions: crate::reactions::History,
    /// What `/` looks for in the diff, while it is typed and after, until `esc`.
    pub search: Option<super::Search>,
    /// The answers `a h` found kept for the open MR, which the search box lists.
    pub past_answers: Vec<super::PastAnswer>,
    pub ascii: bool,
    pub notify: bool,
    pub hosts: crate::forge::Hosts,
    /// What To review held at the last fresh answer, so only newcomers are announced.
    pub seen: Option<super::notify::Seen>,
    /// How rows sit inside the sections: `s` sorts, `S` groups by author.
    pub queue_view: super::QueueView,
    /// The MRs pinned in this scope: PINNED shows them above every section.
    pub pins: BTreeSet<MrKey>,
    pub queue_layout: crate::config::QueueLayout,
    pub queue_loading: bool,
    /// The last load failed: with nothing to show, the pane says so until one succeeds or `r` asks again.
    pub queue_failed: bool,
    /// How many MRs to load ahead after each fresh queue.
    pub prefetch_limit: usize,
    /// A plan held back while an MR was being opened: it goes out once that MR arrived.
    pub prefetch_due: bool,
    /// The next MR that needs me, offered in the status line after a publish.
    pub offer: Option<MrKey>,
    /// Per started MR: the queue shows how far each review went.
    pub progress: HashMap<MrKey, crate::review::Progress>,
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
    /// The key pressed over and over right now, as a held key repeats.
    pub repeat: Option<super::repeat::Repeat>,
    /// A quit key pressed once, and when: the same key again inside the window quits.
    pub quitting: Option<(super::quit::QuitKey, Instant)>,
    pub quit_confirm: bool,
    /// What this session counted since the last flush; `None` when `[usage]` is off.
    pub usage: Option<crate::usage::Tally>,
    /// Up to when screen time is counted, and what stops it.
    pub usage_clock: super::usage::Clock,
    /// The j or k in a row moving through the diff.
    pub walk: super::usage::Walk,
    /// When zen was left with a key, and on which MR.
    pub left_zen: Option<(Instant, MrKey)>,
    /// What the key in flight did, named by its handler where the same key does something else elsewhere.
    pub key_did: Option<&'static str>,
    pub keymap: crate::keymap::Keymap,
    pub pending: Option<char>,
    /// The input row is open for this; `buffer` holds what is typed.
    pub input: Option<Input>,
    pub buffer: Field,
    /// Text typed for a target and left with `esc`, by target: it comes back when the box opens there again.
    pub unsent: HashMap<String, String>,
    /// Where the compose box was opened from: the keys go back there once it closes.
    pub compose_from: Focus,
    /// The diff row the cursor left for the compose box under another row; it goes back there once the box closes.
    pub compose_left: Option<crate::review::Row>,
    pub publish: Option<Publish>,
    /// The MR description modal.
    pub brief: Option<Brief>,
    /// revu's own changelog, and how far the reader had got in it before this start.
    pub whats_new: super::WhatsNew,
    /// The first row of the `:news` page while it is open.
    pub news_page: Option<usize>,
    /// Where the `!iid`s were drawn this frame, so the loop can make them clickable.
    pub links: Vec<crate::tui::ui::Link>,
    /// Where the panes were drawn this frame, so the wheel scrolls the one under the pointer.
    pub areas: super::Areas,
    /// The rows of text a drag can select, as this frame drew them.
    pub text_rows: Vec<crate::tui::drag::TextRow>,
    /// The list rows a click can pick, as this frame drew them.
    pub list_rows: Vec<super::ListRow>,
    /// Where the left button went down, so a release elsewhere is a drag and not a click.
    pub pressed: Option<ratatui::layout::Position>,
    /// The text a drag covers; it stays lit after the copy until the next key or click.
    pub drag: Option<crate::tui::drag::Drag>,
    /// Pictures of the comments the right pane shows, each fetched once.
    pub thumbs: crate::tui::images::Thumbs,
    /// What frames read from the open review, kept until it changes.
    pub kept: super::Kept,
    /// The right pane as the last frame laid it out.
    pub pane_layout: Option<crate::tui::thread_view::PaneLayout>,
    /// The conversations the diff draws under their lines, as the last frame laid them out.
    pub thread_rows: crate::tui::thread_view::InlineLayout,
    /// A file ready for the reader's program; the loop takes it and hands over the terminal.
    pub viewing: Option<crate::open::View>,
    /// `[agent] enabled`: `a g` opens an agent session on an MR.
    pub agent: bool,
    /// The sessions `a g` found for an MR, which the search box lists.
    pub session_pick: Option<super::SessionPick>,
    /// An agent session ready for the terminal; the loop takes it and hands the terminal over.
    pub handoff: Option<crate::agent::Handoff>,
    /// What the editor's text turned into; the loop drains it after `apply`.
    pub composed: Vec<Action>,
    /// The key list, open.
    pub help: Option<crate::tui::help::Help>,
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
    /// The local id of the last draft written.
    pub next_draft: u64,
    /// The number of the last state save sent: one that finishes after a newer one is dropped.
    pub next_save: u64,
    /// What Jev said about each queue MR.
    pub verdicts: HashMap<MrKey, crate::ai::triage::Verdict>,
    /// MRs Jev is being asked about right now, so a new queue does not ask twice.
    pub triage_asked: std::collections::HashSet<MrKey>,
    /// What Jev read in each opened MR, with the head commit it read.
    pub readings: HashMap<MrKey, (crate::forge::Sha, crate::ai::triage::Reading)>,
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

/// A config this revu partly ignores says so before what is new in revu, which shows again on `:news`.
fn first_toast(settings: &Settings, now: Instant) -> Option<Toast> {
    if !settings.ignored_settings.is_empty() {
        return Some(Toast::starting(settings.ignored_settings.join(" · "), true, now));
    }
    settings.whats_new.announcement().map(|text| Toast::starting(text, false, now))
}

impl App {
    pub fn new(settings: Settings) -> Self {
        let now = Instant::now();
        let toast = first_toast(&settings, now);
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
            wrap: None,
            zen: false,
            narrow: false,
            compact: false,
            short: false,
            tap_bar: settings.tap_bar,
            taps: vec![],
            zen_width: settings.zen_width,
            widths: settings.widths,
            zen_switch: None,
            quiet_notices: vec![],
            rate: crate::forge::RateLimit::default(),
            news: None,
            confirm: None,
            react: None,
            reactions: settings.reactions,
            search: None,
            past_answers: vec![],
            ascii: settings.ascii,
            notify: settings.notify,
            hosts: settings.hosts,
            seen: None,
            queue_view: super::QueueView::default(),
            pins: BTreeSet::new(),
            queue_layout: settings.queue_layout,
            prefetch_limit: settings.prefetch,
            prefetch_due: false,
            offer: None,
            progress: HashMap::new(),
            spot_saved: None,
            spot_pending: None,
            queue_loading: true,
            queue_failed: false,
            open: None,
            opening: None,
            held: None,
            repeat: None,
            quitting: None,
            quit_confirm: settings.quit_confirm,
            usage: settings.usage,
            usage_clock: super::usage::Clock::starting(now),
            walk: super::usage::Walk::default(),
            left_zen: None,
            key_did: None,
            keymap: settings.keymap,
            pending: None,
            input: None,
            buffer: Field::default(),
            unsent: HashMap::new(),
            compose_from: Focus::default(),
            compose_left: None,
            publish: None,
            brief: None,
            toast,
            whats_new: settings.whats_new,
            news_page: None,
            links: vec![],
            areas: super::Areas::default(),
            text_rows: vec![],
            list_rows: vec![],
            pressed: None,
            drag: None,
            kept: super::Kept::default(),
            pane_layout: None,
            thread_rows: crate::tui::thread_view::InlineLayout::default(),
            thumbs: settings.pictures.map_or_else(crate::tui::images::Thumbs::off, crate::tui::images::Thumbs::with),
            viewing: None,
            agent: settings.agent,
            session_pick: None,
            handoff: None,
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
            next_draft: 0,
            next_save: 0,
            verdicts: HashMap::new(),
            triage_asked: std::collections::HashSet::new(),
            readings: HashMap::new(),
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

    /// `revu 42`: the MR opens in zen at once, and the queue loads behind it.
    pub fn start_on(&mut self, key: MrKey) -> Vec<Action> {
        let opening = self.open_key(key);
        self.zen = true;
        [self.start(), opening].concat()
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
        let Some((key, head)) = self.open.as_ref().map(|o| (o.key.clone(), o.review.mr.refs.head.clone())) else { return actions };
        if self.poll.mr_due.is_some_and(|due| self.now >= due) {
            self.poll.mr_due = None;
            actions.push(Action::PollMr { key: key.clone(), head });
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
        let Some((conversations, _, _)) = self.kept.pane_view(open) else { return vec![] };
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crate::tui::app::test_support::*;

    #[test]
    fn starts_by_loading_the_queue() {
        assert_eq!(app().start(), vec![Action::LoadQueue { scope: None, from_cache: true }]);
    }

    #[test]
    fn polling_fires_once_per_due_date_and_backs_off_on_failure() {
        let mut app = with_review();
        assert_eq!(app.tick(), vec![]);
        app.now += Duration::from_secs(31);
        assert_eq!(app.tick(), vec![Action::RefreshDiscussions(mr_key())]);
        assert_eq!(app.tick(), vec![]);
        app.now += Duration::from_secs(30);
        let actions = app.tick();
        assert!(
            actions.contains(&Action::LoadQueue { scope: None, from_cache: false })
                && actions.contains(&Action::PollMr { key: mr_key(), head: "bbbb".into() }),
            "{actions:?}"
        );
        app.apply(Incoming::Failed { what: Failure::Poll, message: "offline".into() });
        assert!(app.offline.is_some());
        app.now += Duration::from_secs(120);
        assert_eq!(app.tick(), vec![], "backed off for five minutes");
        app.now += Duration::from_secs(200);
        assert_ne!(app.tick(), [] as [Action; 0]);
    }

    #[test]
    fn few_requests_left_slow_polling_and_a_wait_shows_in_the_status_line() {
        let mut app = with_queue();
        app.rate = crate::forge::RateLimit { remaining: Some(12), wait: None };
        app.schedule_queue();
        assert_eq!(app.poll.queue_due, Some(app.now + Duration::from_secs(300)), "five times slower");
        assert!(render(&mut app, 120, 20).contains("12 requests left"));
        app.rate = crate::forge::RateLimit { remaining: Some(0), wait: Some(Duration::from_secs(42)) };
        let screen = render(&mut app, 120, 20);
        assert!(screen.contains("⏳") && screen.contains("42s"), "{screen}");
    }

    #[test]
    fn the_pane_asks_for_each_of_its_pictures_once() {
        let mut app = with_note("Look:\n![chart](/uploads/0123456789abcdef0123456789abcdef/chart.png)");
        assert!(app.tick().iter().all(|a| !matches!(a, Action::LoadImage { .. })), "a terminal without pictures asks for none");
        app.thumbs = crate::tui::images::tests::test_thumbs();
        let url = "/uploads/0123456789abcdef0123456789abcdef/chart.png".to_owned();
        let asked: Vec<Action> = app.tick().into_iter().filter(|a| matches!(a, Action::LoadImage { .. })).collect();
        assert_eq!(asked, vec![Action::LoadImage { key: mr_key(), url: url.clone() }]);
        assert!(app.tick().iter().all(|a| !matches!(a, Action::LoadImage { .. })), "asked once");
        app.apply(Incoming::Image { url: url.clone(), image: Some(image::DynamicImage::new_rgb8(200, 100)) });
        assert!(matches!(app.thumbs.get(&url), Some(crate::tui::images::Thumb::Ready(..))));
    }
}
