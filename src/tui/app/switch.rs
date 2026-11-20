//! `:local` and `:forge`: this session ends and the loop opens the other space in the same screen.
use super::App;
use crate::forge::{Kind, Me, MrKey, QueueMr};

/// What a session reads: the worktrees of the checkout, or the forge's queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Space {
    Local,
    Forge,
}

impl Space {
    fn name(self) -> &'static str {
        match self {
            Self::Local => "the worktrees of this checkout",
            Self::Forge => "the forge",
        }
    }

    /// The command that switches to it, which also marks its rows in the palette.
    pub(super) fn word(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Forge => "forge",
        }
    }

    /// The space `:<word>` switches to.
    pub(crate) fn from_word(word: &str) -> Option<Self> {
        [Self::Local, Self::Forge].into_iter().find(|space| space.word() == word)
    }

    pub(super) fn other(self) -> Self {
        match self {
            Self::Local => Self::Forge,
            Self::Forge => Self::Local,
        }
    }
}

/// Where the loop goes once this session ends: the other space, on the MR the palette picked in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Switch {
    pub(crate) to: Space,
    /// Opened at once; without one the space opens as its command does.
    pub(crate) open: Option<MrKey>,
}

/// The MRs of the other space, listed by the palette after this one's, the forge they live on and who I am there.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OtherRows {
    pub(crate) kind: Kind,
    /// What `@me` and `is:mine` match on these rows.
    pub(crate) me: Me,
    pub(crate) mrs: Vec<QueueMr>,
}

impl App {
    pub(super) fn space(&self) -> Space {
        if self.is_local() { Space::Local } else { Space::Forge }
    }

    /// The loop takes the switch once this key is handled; a draft the forge does not hold yet keeps the reader here.
    pub(super) fn switch_space(&mut self, to: Space, open: Option<MrKey>) {
        let unsaved = self.unsaved_drafts();
        if unsaved > 0 {
            self.warn(format!("{unsaved} draft{} not saved yet: switch once the forge holds them", if unsaved == 1 { "" } else { "s" }));
            return;
        }
        self.switch = Some(Switch { to, open });
    }

    /// The loop stops: the reader quits, or asked for the other space.
    pub(crate) fn session_over(&self) -> bool {
        self.should_quit || self.switch.is_some()
    }

    /// Taken by the loop, which ends this session and opens the space asked for.
    pub(crate) fn take_switch(&mut self) -> Option<Switch> {
        self.switch.take()
    }

    /// Shown while the loop opens `space`, which keeps the screen still.
    pub(crate) fn opening(&mut self, space: Space) {
        self.toast(format!("opening {}…", space.name()));
    }

    /// The first words of a session the reader switched to; a toast the session started with says more, and stays.
    pub(crate) fn switched(&mut self) {
        if self.live_toast().is_some() {
            return;
        }
        self.toast(if self.is_local() {
            "local: the worktrees of this checkout · :forge goes back"
        } else {
            "forge: your queue · :local reads the worktrees of this checkout"
        });
    }

    /// The other space did not open: this session goes on as it was.
    pub(crate) fn switch_failed(&mut self, space: Space, reason: &str) {
        self.warn(format!("cannot open {}: {reason}", space.name()));
    }
}

#[cfg(test)]
mod tests {
    use super::{OtherRows, Space, Switch};
    use crate::tui::app::test_support::*;

    #[test]
    fn local_on_a_forge_asks_the_loop_for_the_worktrees() {
        let mut app = with_queue();
        type_text(&mut app, ":local");
        assert!(app.session_over());
        assert_eq!(app.take_switch(), Some(Switch { to: Space::Local, open: None }));
        assert_eq!(app.take_switch(), None, "taken once");
    }

    #[test]
    fn forge_on_a_local_diff_asks_the_loop_for_the_forge() {
        let mut app = local_app();
        type_text(&mut app, ":forge");
        assert_eq!(app.take_switch(), Some(Switch { to: Space::Forge, open: None }));
    }

    #[test]
    fn forge_on_the_forge_is_no_switch() {
        let mut app = with_queue();
        type_text(&mut app, ":forge");
        assert!(!app.session_over());
        assert_eq!(toast(&app), "`forge` does nothing here");
    }

    #[test]
    fn local_on_a_local_diff_is_no_switch() {
        let mut app = local_app();
        type_text(&mut app, ":local");
        assert!(!app.session_over());
        assert_eq!(toast(&app), "`local` does nothing here");
    }

    #[test]
    fn a_draft_the_forge_does_not_hold_yet_keeps_the_reader_on_the_forge() {
        let mut app = with_review();
        on_line(&mut app);
        press(&mut app, "c");
        type_text(&mut app, "nit");
        type_text(&mut app, ":local");
        assert!(!app.session_over());
        assert_eq!(toast(&app), "1 draft not saved yet: switch once the forge holds them");
    }

    #[test]
    fn a_space_that_did_not_open_leaves_the_session_running_and_says_why() {
        let mut app = with_queue();
        type_text(&mut app, ":local");
        let space = app.take_switch().unwrap().to;
        app.opening(space);
        assert_eq!(toast(&app), "opening the worktrees of this checkout…");
        app.switch_failed(space, "not a git repository");
        assert!(!app.session_over(), "the loop runs this session again");
        assert_eq!(toast(&app), "cannot open the worktrees of this checkout: not a git repository");
    }

    #[test]
    fn a_switched_local_session_says_it_reads_the_worktrees() {
        let mut app = local_app();
        app.toast = None;
        app.switched();
        assert_eq!(toast(&app), "local: the worktrees of this checkout · :forge goes back");
    }

    #[test]
    fn a_switched_forge_session_says_it_reads_the_queue() {
        let mut app = with_queue();
        app.toast = None;
        app.switched();
        assert_eq!(toast(&app), "forge: your queue · :local reads the worktrees of this checkout");
    }

    #[test]
    fn a_switched_session_keeps_the_toast_it_started_with() {
        let mut app = with_queue();
        app.warn("config: `tui.colour` is not a setting");
        app.switched();
        assert_eq!(toast(&app), "config: `tui.colour` is not a setting");
    }

    /// A worktree as the bare listing names it: titled by its branch.
    fn worktree_rows() -> OtherRows {
        let mr = Mr { source_branch: "spike/ledger".into(), ..local_mr(0, "bbbb") };
        let worktree = QueueMr::from_mr(&mr, local_key().host).unwrap();
        OtherRows { kind: Kind::Local, me: "widgets".into(), mrs: vec![QueueMr { title: mr.source_branch, ..worktree }] }
    }

    fn cached_queue() -> OtherRows {
        OtherRows { kind: Kind::GitHub, me: "nina".into(), mrs: sections().all().cloned().collect() }
    }

    /// The labels and details the palette lists, in order.
    fn listed(app: &App) -> Vec<(String, String)> {
        app.palette_candidates(app.palette.as_ref().unwrap()).into_iter().map(|c| (c.label, c.detail)).collect()
    }

    fn marked(app: &App, word: &str) -> usize {
        listed(app).iter().filter(|(_, detail)| detail == word).count()
    }

    #[test]
    fn only_the_palette_on_mrs_asks_for_the_other_space() {
        let mut app = with_queue();
        assert_eq!(app.handle_key(ctrl_k()), vec![Action::FindOtherSpace]);
        app.handle_key(code(KeyCode::Esc));
        assert_eq!(press(&mut app, ":"), vec![], "commands list no MR");
    }

    #[test]
    fn the_palette_opened_again_while_the_other_space_is_searched_asks_nothing_more() {
        let mut app = with_queue();
        app.handle_key(ctrl_k());
        app.handle_key(code(KeyCode::Esc));
        assert_eq!(app.handle_key(ctrl_k()), vec![], "the first search still runs");
        app.handle_key(code(KeyCode::Esc));
        app.apply(Incoming::OtherSpace(None));
        assert_eq!(app.handle_key(ctrl_k()), vec![Action::FindOtherSpace]);
    }

    #[test]
    fn the_worktrees_follow_the_queue_in_the_palette_marked_local() {
        let mut app = with_queue();
        app.handle_key(ctrl_k());
        let queued = listed(&app).len();
        app.apply(Incoming::OtherSpace(Some(worktree_rows())));
        let listed = listed(&app);
        assert_eq!(listed.len(), queued + 1);
        assert_eq!(listed.last().unwrap(), &("spike/ledger".to_owned(), "local".to_owned()));
    }

    #[test]
    fn a_long_queue_leaves_room_for_the_other_space_on_an_empty_line() {
        let mut app = app();
        let queue = fixture::queue(include_str!("../../forge/gitlab/fixtures/queue.json"));
        let one = queue.review_requested[0].clone();
        let many = (100..120).map(|number| QueueMr { number, ..one.clone() }).collect();
        let sections = crate::forge::Queue { review_requested: many, ..queue }.sections(&[]);
        app.apply(Incoming::Queue { scope: None, me: "nina".into(), sections, opened: HashMap::new(), cached: false });
        app.apply(Incoming::OtherSpace(Some(worktree_rows())));
        app.handle_key(ctrl_k());
        let listed = listed(&app);
        assert_eq!(listed.len(), vtui::palette::MAX_SHOWN);
        assert_eq!(listed.last().unwrap().1, "local");
    }

    #[test]
    fn a_typed_word_ranks_the_other_space_with_this_one() {
        let mut app = with_queue();
        app.apply(Incoming::OtherSpace(Some(worktree_rows())));
        app.handle_key(ctrl_k());
        press(&mut app, "ledger");
        assert_eq!(listed(&app)[0], ("spike/ledger".to_owned(), "local".to_owned()));
        press(&mut app, "zzz");
        assert_eq!(marked(&app, "local"), 0);
    }

    #[test]
    fn the_cached_queue_is_listed_on_a_local_diff_marked_forge_with_its_forge_sigil() {
        let mut app = local_app();
        app.apply(Incoming::OtherSpace(Some(cached_queue())));
        app.handle_key(ctrl_k());
        let listed = listed(&app);
        assert!(!listed.is_empty() && listed.iter().all(|(label, detail)| label.starts_with('#') && detail == "forge"), "{listed:?}");
    }

    #[test]
    fn me_on_the_cached_queue_is_who_i_am_on_its_forge() {
        let mut app = local_app();
        app.apply(Incoming::OtherSpace(Some(cached_queue())));
        app.handle_key(ctrl_k());
        press(&mut app, "is:mine");
        let mine = cached_queue().mrs.iter().filter(|mr| mr.author == "nina").count();
        assert!(mine > 0);
        assert_eq!(marked(&app, "forge"), mine);
    }

    #[test]
    fn picking_a_cached_mr_switches_to_the_forge_on_it() {
        let mut app = local_app();
        app.apply(Incoming::OtherSpace(Some(cached_queue())));
        app.handle_key(ctrl_k());
        app.handle_key(code(KeyCode::Enter));
        let first = cached_queue().mrs[0].key();
        assert_eq!(app.take_switch(), Some(Switch { to: Space::Forge, open: Some(first) }));
    }

    #[test]
    fn picking_a_worktree_switches_to_local_on_it() {
        let mut app = with_queue();
        app.apply(Incoming::OtherSpace(Some(worktree_rows())));
        app.handle_key(ctrl_k());
        type_text(&mut app, "ledger");
        assert_eq!(app.take_switch(), Some(Switch { to: Space::Local, open: Some(local_key()) }));
    }

    #[test]
    fn without_an_answer_the_palette_lists_this_space_only() {
        let mut app = with_queue();
        app.handle_key(ctrl_k());
        assert_eq!(marked(&app, "local"), 0);
    }

    #[test]
    fn the_palette_draws_the_other_space_apart() {
        let mut app = with_queue();
        app.apply(Incoming::OtherSpace(Some(worktree_rows())));
        app.handle_key(ctrl_k());
        insta::assert_snapshot!("palette_other_space", render(&mut app, 100, 24));
    }
}
