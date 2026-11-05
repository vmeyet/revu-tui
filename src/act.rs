//! Every action revu has, in one table: the name `[keys.bind]` and `revu usage` give it, its default keys, its
//! palette verb; every `?` help line has a table of its own. The keymap, the help, the palette, the usage report and
//! `docs/reference/keys.md` all read it from here.
use crate::forge::Kind;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::sync::LazyLock;

/// One action; its row in [`ROWS`] says everything else about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Act {
    NextThread,
    PrevThread,
    NextHunk,
    PrevHunk,
    NextFile,
    PrevFile,
    FoldToggle,
    FoldOpen,
    FoldClose,
    FoldAll,
    UnfoldAll,
    Viewed,
    Zen,
    PrevMr,
    NextMr,
    SideBySide,
    PeekAfter,
    PeekBefore,
    Tree,
    EveryThread,
    Pipeline,
    Prose,
    Wrap,
    Whitespace,
    MoreContext,
    ViewFile,
    Description,
    Comment,
    CommentOld,
    Select,
    Suggest,
    Editor,
    Resolve,
    Publish,
    Approve,
    Merge,
    Ready,
    Share,
    Session,
    React,
    OpenBrowser,
    CopyLink,
    Scope,
    SortQueue,
    Filter,
    Palette,
    Views,
    Jump,
    Help,
    Quit,
    MoveDown,
    MoveUp,
    PageDown,
    PageUp,
    FullPageDown,
    FullPageUp,
    Top,
    Bottom,
    Open,
    Back,
    FocusLeft,
    FocusRight,
    Refresh,
    Reply,
    ApplySuggestion,
    EditDraft,
    DeleteDraft,
    ClosePane,
    OpenLink,
    CopyFirstLink,
    PaletteMr,
    GoCommand,
    CopyCommand,
    ApproveCommand,
    PublishCommand,
    ThreadsCommand,
    SetCommand,
    ViewCommand,
    AiCommand,
    ShareCommand,
    SessionCommand,
    NewsCommand,
    TargetCommand,
    QuitCommand,
    Pin,
    ViewedNext,
    ChangeTarget,
    ToggleUncommitted,
    Search,
    Mine,
}

/// A line of the `?` help: its keys as the help writes them, and what they do in a few words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Line {
    pub(crate) group: Group,
    pub(crate) keys: &'static str,
    pub(crate) does: &'static str,
    reach: Reach,
}

pub(crate) use vtui::palette::Verb;

/// What revu knows about one action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) act: Act,
    pub(crate) name: Name,
    /// The keys revu reads for it outside a text box, as `[keys.bind]` writes them; a binding stands for the first.
    pub(crate) keys: &'static [&'static str],
    pub(crate) verb: Option<Verb>,
    /// Where its one-key keys do it.
    pub(crate) panes: &'static [Pane],
    /// Where `revu usage` lists it; `None` for an action it does not count.
    pub(crate) report: Option<Group>,
    pub(crate) reach: Reach,
}

/// Which diffs an action makes sense on; the keys, the help and the palette all read it through [`Reach::serves`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reach {
    Both,
    /// It talks to the forge: `revu local` has none.
    Forge,
    /// `revu local` alone.
    Local,
}

impl Reach {
    pub(crate) fn serves(self, kind: Kind) -> bool {
        match self {
            Self::Both => true,
            Self::Forge => kind != Kind::Local,
            Self::Local => kind == Kind::Local,
        }
    }
}

/// Where a key does an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pane {
    /// Wherever the keys go to a pane, before the pane gets them.
    Anywhere,
    Queue,
    Diff,
    /// A thread, note or draft under the diff's cursor, before the diff's own keys.
    ThreadRow,
    /// The right pane's conversations.
    Threads,
    Tree,
    Pipeline,
    /// The prose view of a Markdown file, after `Page`.
    Prose,
    /// A page that only scrolls: the prose view, the `?` help, the news.
    Page,
    /// The MR cover page, `i`.
    Cover,
}

/// Every pane with a cursor or a scroll: where the moving keys work.
const MOVES: &[Pane] = &[Pane::Queue, Pane::Diff, Pane::Threads, Pane::Tree, Pane::Pipeline, Pane::Page, Pane::Cover];

/// What an action is called, and who uses the name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Name {
    /// `[keys.bind]` can give it more keys; the usage counts and `revu usage` use the name too.
    Bind(&'static str),
    /// Counted, and listed by `revu usage` even when never used.
    Listed(&'static str),
    /// Pressing it is not counted.
    None,
}

impl Name {
    /// The name its usage is counted under.
    pub(crate) fn counted(self) -> Option<&'static str> {
        match self {
            Self::Bind(name) | Self::Listed(name) => Some(name),
            Self::None => None,
        }
    }
}

/// A task the `?` help and `revu usage` list actions under; `help::GROUPS` holds the help's; a one-pane key says its pane in its words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Group {
    Move,
    Jump,
    Queue,
    View,
    Comment,
    Thread,
    App,
    Commands,
}

impl Group {
    pub(crate) const REPORT_ORDER: [Group; 8] =
        [Group::Move, Group::Jump, Group::Queue, Group::View, Group::Comment, Group::Thread, Group::App, Group::Commands];

    /// Its lines, in the order the help shows them.
    pub(crate) fn lines(self) -> impl Iterator<Item = &'static Line> {
        HELP.iter().filter(move |line| line.group == self)
    }

    /// Its lines that do something on `kind`.
    pub(crate) fn lines_on(self, kind: Kind) -> impl Iterator<Item = &'static Line> {
        self.lines().filter(move |line| line.reach.serves(kind))
    }

    /// Writing and threads need a forge, all of them.
    const fn reach(self) -> Reach {
        match self {
            Self::Comment | Self::Thread => Reach::Forge,
            _ => Reach::Both,
        }
    }

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Move => "move",
            Self::Jump => "jump",
            Self::Queue => "queue",
            Self::View => "view",
            Self::Comment => "comment & publish",
            Self::Thread => "thread pane",
            Self::App => "search & app",
            Self::Commands => "commands",
        }
    }
}

const fn row(act: Act, name: Name, keys: &'static [&'static str]) -> Row {
    Row { act, name, keys, verb: None, panes: &[], report: None, reach: Reach::Both }
}

const fn bound(act: Act, name: &'static str, keys: &'static [&'static str]) -> Row {
    row(act, Name::Bind(name), keys)
}

const fn listed(act: Act, name: &'static str, keys: &'static [&'static str]) -> Row {
    row(act, Name::Listed(name), keys)
}

const fn unnamed(act: Act, keys: &'static [&'static str]) -> Row {
    row(act, Name::None, keys)
}

/// A palette command, counted as its word after a colon: `:go`.
macro_rules! command {
    ($act:expr, $word:literal, $does:literal) => {
        Row {
            verb: Some(Verb { word: $word, does: $does }),
            report: Some(Group::Commands),
            ..row($act, Name::Listed(concat!(":", $word)), &[])
        }
    };
}

const fn line(group: Group, keys: &'static str, does: &'static str) -> Line {
    Line { group, keys, does, reach: group.reach() }
}

impl Line {
    const fn reach(self, reach: Reach) -> Self {
        Self { reach, ..self }
    }
}

impl Row {
    const fn report(self, group: Group) -> Self {
        Self { report: Some(group), reach: group.reach(), ..self }
    }

    const fn panes(self, panes: &'static [Pane]) -> Self {
        Self { panes, ..self }
    }

    const fn reach(self, reach: Reach) -> Self {
        Self { reach, ..self }
    }
}

/// One row per [`Act`], in its order: bindable actions first, then the others `revu usage` lists, then the rest.
pub(crate) const ROWS: &[Row] = &[
    bound(Act::NextThread, "next_thread", &["]n", "n"]).panes(&[Pane::Diff]).report(Group::Jump),
    bound(Act::PrevThread, "prev_thread", &["[n", "N"]).panes(&[Pane::Diff]).report(Group::Jump),
    bound(Act::NextHunk, "next_hunk", &["]c"]).report(Group::Jump),
    bound(Act::PrevHunk, "prev_hunk", &["[c"]).report(Group::Jump),
    bound(Act::NextFile, "next_file", &["tab"]).panes(&[Pane::Diff]).report(Group::Jump),
    bound(Act::PrevFile, "prev_file", &["backtab"]).panes(&[Pane::Diff]).report(Group::Jump),
    bound(Act::FoldToggle, "fold_toggle", &["za"]).report(Group::View),
    bound(Act::FoldOpen, "fold_open", &["zo"]).report(Group::View),
    bound(Act::FoldClose, "fold_close", &["zc"]).report(Group::View),
    bound(Act::FoldAll, "fold_all", &["zM"]).report(Group::View),
    bound(Act::UnfoldAll, "unfold_all", &["zR"]).report(Group::View),
    bound(Act::Viewed, "viewed", &["zv"]).report(Group::View),
    bound(Act::Zen, "zen", &["zz"]).report(Group::View),
    bound(Act::PrevMr, "prev_mr", &["[m"]).report(Group::Jump),
    bound(Act::NextMr, "next_mr", &["]m"]).report(Group::Jump),
    bound(Act::SideBySide, "side_by_side", &["D"]).panes(&[Pane::Diff, Pane::Prose]).report(Group::View),
    bound(Act::PeekAfter, "peek_after", &[">"]).panes(&[Pane::Diff]).report(Group::View),
    bound(Act::PeekBefore, "peek_before", &["<"]).panes(&[Pane::Diff]).report(Group::View),
    bound(Act::Tree, "tree", &["t"]).panes(&[Pane::Diff, Pane::Tree]).report(Group::View),
    bound(Act::EveryThread, "every_thread", &["T"]).panes(&[Pane::Diff, Pane::Threads]).report(Group::Thread).reach(Reach::Both),
    bound(Act::Pipeline, "pipeline", &["p"]).panes(&[Pane::Diff, Pane::Pipeline, Pane::Cover]).report(Group::View).reach(Reach::Forge),
    bound(Act::Prose, "prose", &["v"]).panes(&[Pane::Diff, Pane::Prose]).report(Group::View),
    bound(Act::Wrap, "wrap", &["w"]).panes(&[Pane::Diff]).report(Group::View),
    bound(Act::Whitespace, "whitespace", &["W"]).panes(&[Pane::Diff]).report(Group::View),
    bound(Act::MoreContext, "more_context", &["="]).panes(&[Pane::Diff]).report(Group::View),
    bound(Act::ViewFile, "view_file", &["ctrl-v"]).panes(&[Pane::Diff, Pane::ThreadRow, Pane::Tree]).report(Group::View),
    bound(Act::Description, "description", &["i"]).panes(&[Pane::Queue, Pane::Diff, Pane::Cover]).report(Group::View).reach(Reach::Forge),
    bound(Act::Comment, "comment", &["c"]).panes(&[Pane::Diff]).report(Group::Comment),
    bound(Act::CommentOld, "comment_old", &["C"]).panes(&[Pane::Diff]).report(Group::Comment),
    bound(Act::Select, "select", &["V"]).panes(&[Pane::Diff]).report(Group::Comment),
    bound(Act::Suggest, "suggest", &["s"]).panes(&[Pane::Diff]).report(Group::Comment),
    bound(Act::Editor, "editor", &["E"]).panes(&[Pane::Diff, Pane::ThreadRow]).report(Group::Comment),
    bound(Act::Resolve, "resolve", &["R"]).panes(&[Pane::Diff, Pane::ThreadRow]).report(Group::Comment),
    bound(Act::Publish, "publish", &["P"]).panes(&[Pane::Diff, Pane::Threads]).report(Group::Comment),
    bound(Act::Approve, "approve", &["A"]).panes(&[Pane::Diff]).report(Group::Comment),
    bound(Act::Merge, "merge", &["M"]).panes(&[Pane::Diff]).report(Group::Comment),
    bound(Act::Ready, "ready", &["H"]).panes(&[Pane::Diff]).report(Group::Comment),
    bound(Act::Share, "share", &["Y"]).panes(&[Pane::Anywhere]).report(Group::Comment),
    bound(Act::Session, "session", &["a"]).panes(&[Pane::Anywhere]).report(Group::App).reach(Reach::Forge),
    bound(Act::React, "react", &["+"]).panes(&[Pane::Diff, Pane::ThreadRow]).report(Group::Thread),
    bound(Act::OpenBrowser, "open_browser", &["o"])
        .panes(&[Pane::Queue, Pane::Diff, Pane::ThreadRow, Pane::Pipeline, Pane::Cover])
        .report(Group::App)
        .reach(Reach::Forge),
    bound(Act::CopyLink, "copy_link", &["y"])
        .panes(&[Pane::Queue, Pane::Diff, Pane::ThreadRow, Pane::Pipeline, Pane::Cover])
        .report(Group::App)
        .reach(Reach::Forge),
    bound(Act::Scope, "scope", &["*"]).panes(&[Pane::Queue]).report(Group::Queue).reach(Reach::Forge),
    bound(Act::SortQueue, "sort_queue", &["s"]).panes(&[Pane::Queue]).report(Group::Queue),
    bound(Act::Filter, "filter", &["/"]).panes(&[Pane::Queue]).report(Group::Queue),
    bound(Act::Palette, "palette", &[":"]).panes(&[Pane::Anywhere]).report(Group::App),
    bound(Act::Views, "views", &["'", "1", "2", "3", "4", "5", "6", "7", "8", "9"]).panes(&[Pane::Queue]).report(Group::Queue),
    bound(Act::Jump, "jump", &["ctrl-k", ","]).panes(&[Pane::Anywhere]).report(Group::App),
    bound(Act::Help, "help", &["?"]).panes(&[Pane::Anywhere]).report(Group::App),
    bound(Act::Quit, "quit", &["q", "ctrl-c"]).panes(&[Pane::Anywhere, Pane::Cover, Pane::Page]).report(Group::App),
    listed(Act::MoveDown, "move_down", &["j", "down"]).panes(MOVES).report(Group::Move),
    listed(Act::MoveUp, "move_up", &["k", "up"]).panes(MOVES).report(Group::Move),
    listed(Act::PageDown, "page_down", &["ctrl-d"]).panes(MOVES).report(Group::Move),
    listed(Act::PageUp, "page_up", &["ctrl-u"]).panes(MOVES).report(Group::Move),
    listed(Act::FullPageDown, "full_page_down", &["ctrl-f"]).panes(MOVES).report(Group::Move),
    listed(Act::FullPageUp, "full_page_up", &["ctrl-b"]).panes(MOVES).report(Group::Move),
    listed(Act::Top, "top", &["g"]).panes(MOVES).report(Group::Move),
    listed(Act::Bottom, "bottom", &["G"]).panes(MOVES).report(Group::Move),
    listed(Act::Open, "open", &["enter"]).panes(&[Pane::Queue, Pane::Diff, Pane::Threads, Pane::Tree, Pane::Cover]).report(Group::Move),
    listed(Act::Back, "back", &["esc"])
        .panes(&[Pane::Queue, Pane::Diff, Pane::Tree, Pane::Pipeline, Pane::Prose, Pane::Cover, Pane::Page])
        .report(Group::Move),
    listed(Act::FocusLeft, "focus_left", &["h", "left"]).panes(&[Pane::Anywhere]).report(Group::Move),
    listed(Act::FocusRight, "focus_right", &["l", "right"]).panes(&[Pane::Anywhere]).report(Group::Move),
    listed(Act::Refresh, "refresh", &["r"]).panes(&[Pane::Queue, Pane::Diff, Pane::Pipeline, Pane::Prose]).report(Group::Queue),
    listed(Act::Reply, "reply", &["r", "c"]).panes(&[Pane::ThreadRow]).report(Group::Thread),
    listed(Act::ApplySuggestion, "apply_suggestion", &["S"]).panes(&[Pane::ThreadRow]).report(Group::Thread),
    listed(Act::EditDraft, "edit_draft", &["e"]).panes(&[Pane::ThreadRow]).report(Group::Thread),
    listed(Act::DeleteDraft, "delete_draft", &["d"]).panes(&[Pane::ThreadRow]).report(Group::Thread),
    listed(Act::ClosePane, "close_pane", &["esc", "x"])
        .panes(&[Pane::Diff, Pane::Threads, Pane::Tree, Pane::Pipeline, Pane::Prose])
        .report(Group::Thread),
    listed(Act::OpenLink, "open_link", &["u"]).panes(&[Pane::ThreadRow]).report(Group::Thread),
    listed(Act::CopyFirstLink, "copy_first_link", &["U"]).panes(&[Pane::ThreadRow]).report(Group::Thread),
    listed(Act::PaletteMr, "palette_mr", &[]).report(Group::App),
    command!(Act::GoCommand, "go", "open an MR: :go !42 · :go acme/widgets!42"),
    command!(Act::CopyCommand, "copy", "copy the link of the MR, or of the line").reach(Reach::Forge),
    command!(Act::ApproveCommand, "approve", "approve the open MR, or take the approval back").reach(Reach::Forge),
    command!(Act::PublishCommand, "publish", "publish every draft, in the publish modal").reach(Reach::Forge),
    command!(Act::ThreadsCommand, "threads", "every thread of the MR in the right pane"),
    command!(Act::SetCommand, "set", "change and save a setting: :set theme=nord"),
    command!(Act::ViewCommand, "view", "the file in your program: :view · :view old · :view src/a.rs:42"),
    command!(Act::AiCommand, "ai", "switch AI off or back on for this session: :ai off, :ai on"),
    command!(Act::ShareCommand, "share", "post the MR through a [share] command: :share · :share review").reach(Reach::Forge),
    command!(Act::SessionCommand, "session", "open the agent session of the MR, or start one").reach(Reach::Forge),
    command!(Act::NewsCommand, "news", "what changed in revu, the newest version first"),
    command!(Act::TargetCommand, "target", "compare a local diff against this branch: :target main").reach(Reach::Local),
    command!(Act::QuitCommand, "quit", "leave"),
    listed(Act::Pin, "pin", &["b"]).panes(&[Pane::Queue]).report(Group::Queue),
    listed(Act::ViewedNext, "viewed", &["m"]).panes(&[Pane::Diff]).report(Group::View),
    listed(Act::ChangeTarget, "change_target", &["B"]).panes(&[Pane::Queue, Pane::Diff]).report(Group::View).reach(Reach::Local),
    listed(Act::ToggleUncommitted, "toggle_uncommitted", &["I"]).panes(&[Pane::Queue, Pane::Diff]).report(Group::View).reach(Reach::Local),
    unnamed(Act::Search, &["/"]).panes(&[Pane::Diff]),
    unnamed(Act::Mine, &["m"]).panes(&[Pane::Threads]),
];

/// Every line of the `?` help, in the order it shows them.
const HELP: &[Line] = &[
    line(Group::Move, "j k", "move"),
    line(Group::Move, "g G", "first, last"),
    line(Group::Move, "^d ^u", "half a page; space too"),
    line(Group::Move, "^f ^b", "a full page; PgDn PgUp too"),
    line(Group::Move, "h l", "pane to the left, to the right"),
    line(Group::Move, "enter", "open, or toggle the fold or the thread"),
    line(Group::Move, "esc x", "close the pane, back to the queue"),
    line(Group::Move, "tab S-tab", "next, previous file"),
    line(Group::Move, "]c [c", "next, previous hunk"),
    line(Group::Move, "]n [n", "next, previous open conversation"),
    line(Group::Move, "n N", "the same, while no search is on"),
    line(Group::Move, "]m [m", "next, previous MR in the queue"),
    line(Group::Queue, "/", "filter: words @author !42 ~label"),
    line(Group::Queue, "' 1-9", "a saved view, by letter or rank"),
    line(Group::Queue, "*", "this repo, or every project").reach(Reach::Forge),
    line(Group::Queue, "s", "next sort order"),
    line(Group::Queue, "b", "pin on top of the queue, or unpin"),
    line(Group::Queue, "zo zc", "open, fold the section or stack"),
    line(Group::View, "/ n N", "find text in the MR, next, previous"),
    line(Group::View, "i", "the MR cover page").reach(Reach::Forge),
    line(Group::View, "D", "inline or side by side, diff and prose"),
    line(Group::View, "> <", "the code after, before, alone"),
    line(Group::View, "t", "file tree"),
    line(Group::View, "T", "every thread of the MR"),
    line(Group::View, "p", "pipeline").reach(Reach::Forge),
    line(Group::View, "v", "prose: the Markdown file rendered, its blocks marked"),
    line(Group::View, "zz", "zen: the diff alone, quiet"),
    line(Group::View, "w W", "wrap long lines, hide whitespace changes"),
    line(Group::View, "=", "more lines"),
    line(Group::View, "+", "react on a thread").reach(Reach::Forge),
    line(Group::View, "B", "`revu local`: compare against another branch").reach(Reach::Local),
    line(Group::View, "I", "`revu local`: show uncommitted work, or hide it").reach(Reach::Local),
    line(Group::View, "^v", "the file in your own program"),
    line(Group::View, "za zc zo", "toggle, close, open the fold"),
    line(Group::View, "zR zM", "in prose: show, fold the unchanged blocks"),
    line(Group::View, "zv", "file viewed: it folds away"),
    line(Group::View, "m", "file viewed, on to the next one"),
    line(Group::Comment, "c", "comment on the line, reply on a thread"),
    line(Group::Comment, "C", "comment on the old side"),
    line(Group::Comment, "V", "select lines"),
    line(Group::Comment, "s", "suggest a change"),
    line(Group::Comment, "E", "write in $EDITOR"),
    line(Group::Comment, "r R e d", "on a thread row: reply, resolve, edit, delete"),
    line(Group::Comment, "⌘enter ^s", "in the box: post now, no draft"),
    line(Group::Comment, "⌥enter ^o", "in the box: new line, $EDITOR"),
    line(Group::Comment, "⌥← ⌥→ ⌥⌫", "in the box: word back, on, delete"),
    line(Group::Comment, "esc", "drop the selection, leave the box"),
    line(Group::Comment, "P", "publish every draft"),
    line(Group::Comment, "A M", "approve or unapprove, merge mine"),
    line(Group::Comment, "H", "mark mine draft or ready"),
    line(Group::Comment, "Y", "share the MR, after a preview"),
    line(Group::Thread, "r c", "reply, as a draft"),
    line(Group::Thread, "R", "resolve, unresolve"),
    line(Group::Thread, "T", "every thread; enter goes to it"),
    line(Group::Thread, "m", "every thread: only mine, or all"),
    line(Group::Thread, "e d", "edit, delete my draft"),
    line(Group::Thread, "S +", "apply the suggestion, react"),
    line(Group::Thread, "u U", "open, copy the first link"),
    line(Group::App, "^k ,", "MRs · > commands"),
    line(Group::App, ":", "commands: :go !42, :set theme=nord"),
    line(Group::App, "o y", "open in the browser, copy the link").reach(Reach::Forge),
    line(Group::App, "a", "the agent session of the MR").reach(Reach::Forge),
    line(Group::App, "r", "refresh"),
    line(Group::App, "?", "this help"),
    line(Group::App, "q ^c", "quit: twice; q closes a pane first"),
];

/// The palette's verbs, in the order its list shows them.
pub(crate) fn verbs() -> impl Iterator<Item = Verb> {
    ROWS.iter().filter_map(|row| row.verb)
}

/// The palette's verbs that do something on `kind`.
pub(crate) fn verbs_on(kind: Kind) -> impl Iterator<Item = Verb> {
    ROWS.iter().filter(move |row| row.reach.serves(kind)).filter_map(|row| row.verb)
}

impl Act {
    pub(crate) fn row(self) -> &'static Row {
        &ROWS[self as usize]
    }
}

/// Every default key of every action, as the terminal reports it.
static DEFAULT_KEYS: LazyLock<Vec<(&'static Row, Vec<KeyEvent>)>> = LazyLock::new(|| {
    ROWS.iter()
        .flat_map(|row| row.keys.iter().filter_map(|text| crate::keymap::parse(text).ok()).map(move |keys| (row, keys)))
        .map(|(row, keys)| (row, keys.iter().map(|key| key.event()).collect()))
        .collect()
});

/// The action whose default is `prefix` then `second`: `z` then `a`.
pub(crate) fn act_after(prefix: char, second: char) -> Option<Act> {
    let typed = [KeyEvent::from(KeyCode::Char(prefix)), KeyEvent::from(KeyCode::Char(second))];
    DEFAULT_KEYS.iter().find(|(_, keys)| *keys == typed).map(|(row, _)| row.act)
}

/// The first action `applies` keeps whose one-key default is `code` with exactly `modifiers`.
pub(crate) fn with_key(code: KeyCode, modifiers: KeyModifiers, applies: impl Fn(&Row) -> bool) -> Option<Act> {
    let is_it = |(row, keys): &&(&Row, Vec<KeyEvent>)| {
        let is_the_key = matches!(keys.as_slice(), [own] if own.code == code && own.modifiers == modifiers);
        is_the_key && applies(row)
    };
    DEFAULT_KEYS.iter().find(is_it).map(|(row, _)| row.act)
}

/// What `key` does in `pane`: with ctrl held, only a ctrl default; else its plain default, whatever else is held.
/// Where two rows share a key in one pane, the first one wins.
pub(crate) fn act_in(pane: Pane, key: KeyEvent) -> Option<Act> {
    let works_here = |row: &Row| row.panes.contains(&pane);
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let modifiers = if ctrl { KeyModifiers::CONTROL } else { KeyModifiers::NONE };
    with_key(key.code, modifiers, works_here)
}

/// Every action `[keys.bind]` can name, with the default key a binding stands for.
pub(crate) fn bindable() -> impl Iterator<Item = (&'static str, &'static str)> {
    ROWS.iter().filter_map(|row| match row.name {
        Name::Bind(name) => Some((name, *row.keys.first()?)),
        _ => None,
    })
}

/// `names` in their order, each kept once: `m` and `zv` are both `viewed`.
fn without_repeats(names: impl Iterator<Item = &'static str>) -> Vec<&'static str> {
    names.fold(vec![], |mut kept, name| {
        if !kept.contains(&name) {
            kept.push(name);
        }
        kept
    })
}

/// Every action name `revu usage` lists, bindable ones first.
pub(crate) fn listed_names() -> Vec<&'static str> {
    without_repeats(ROWS.iter().filter_map(|row| row.name.counted()))
}

/// The names `revu usage` lists under `group`, in table order.
pub(crate) fn listed_in(group: Group) -> Vec<&'static str> {
    without_repeats(ROWS.iter().filter(|row| row.report == Some(group)).filter_map(|row| row.name.counted()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::help::GROUPS;

    #[test]
    fn every_act_has_its_row_in_its_place() {
        for (index, row) in ROWS.iter().enumerate() {
            assert_eq!(row.act as usize, index, "{:?} is out of place", row.act);
        }
        assert_eq!(ROWS.len(), Act::Mine as usize + 1, "an act has no row");
    }

    #[test]
    fn every_key_parses_and_a_bindable_action_has_one() {
        for row in ROWS {
            for key in row.keys {
                assert!(crate::keymap::parse(key).is_ok(), "{:?}: `{key}`", row.act);
            }
        }
        assert_eq!(bindable().count(), ROWS.iter().filter(|row| matches!(row.name, Name::Bind(_))).count());
    }

    #[test]
    fn every_help_line_is_in_a_group_the_help_lists() {
        for line in HELP {
            assert!(GROUPS.contains(&line.group), "`{}` is in {:?}", line.keys, line.group);
        }
    }

    fn press(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn a_ctrl_default_does_its_action_in_every_pane_it_works_in() {
        let ctrl_keys: Vec<(&Row, KeyEvent)> = DEFAULT_KEYS
            .iter()
            .filter_map(|(row, keys)| match keys.as_slice() {
                [only] if only.modifiers == KeyModifiers::CONTROL => Some((*row, *only)),
                _ => None,
            })
            .collect();
        let names: Vec<&str> = ctrl_keys.iter().filter_map(|(row, _)| row.name.counted()).collect();
        assert_eq!(names, ["view_file", "jump", "quit", "page_down", "page_up", "full_page_down", "full_page_up"]);
        for (row, key) in ctrl_keys {
            for pane in row.panes {
                assert_eq!(act_in(*pane, key), Some(row.act), "{key:?} in {pane:?}");
            }
        }
    }

    #[test]
    fn a_letter_with_ctrl_does_nothing_unless_it_is_a_ctrl_default() {
        assert_eq!(act_in(Pane::Tree, press(KeyCode::Char('t'), KeyModifiers::CONTROL)), None);
        assert_eq!(act_in(Pane::ThreadRow, press(KeyCode::Char('r'), KeyModifiers::CONTROL)), None);
        assert_eq!(act_in(Pane::Cover, press(KeyCode::Char('i'), KeyModifiers::CONTROL)), None);
        assert_eq!(act_in(Pane::Queue, press(KeyCode::Char('j'), KeyModifiers::CONTROL)), None);
        assert_eq!(act_in(Pane::Queue, press(KeyCode::Char('j'), KeyModifiers::ALT)), Some(Act::MoveDown), "only ctrl is told apart");
        assert_eq!(act_in(Pane::Diff, press(KeyCode::Char('v'), KeyModifiers::CONTROL)), Some(Act::ViewFile));
    }

    #[test]
    fn an_action_is_listed_by_revu_usage_exactly_when_it_is_counted() {
        for row in ROWS {
            assert_eq!(row.report.is_some(), row.name.counted().is_some(), "{:?}", row.act);
        }
    }

    #[test]
    fn every_command_is_in_the_palette() {
        assert_eq!(verbs().count(), ROWS.iter().filter(|row| row.verb.is_some()).count());
    }
}
