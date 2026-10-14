//! Every action revu has, in one table: the name `[keys.bind]` and `revu usage` give it, its default keys, its
//! lines in the `?` help and its palette verb. The keymap, the help, the palette, the usage report and
//! `docs/reference/keys.md` all read it from here.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::sync::LazyLock;

/// One action; its row in [`ROWS`] says everything else about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    NextThread,
    PrevThread,
    NextAnyThread,
    PrevAnyThread,
    NextHunk,
    PrevHunk,
    NextFileUnresolved,
    PrevFileUnresolved,
    NextReview,
    PrevReview,
    NextFile,
    PrevFile,
    FoldToggle,
    FoldOpen,
    FoldClose,
    FoldAll,
    UnfoldAll,
    FoldHeader,
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
    Outline,
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
    GroupByAuthor,
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
    NextInLine,
    PrevInLine,
    OpenLink,
    CopyFirstLink,
    AskExplain,
    AskRisks,
    AskSummary,
    AskThread,
    AskComment,
    AskFree,
    PaletteMr,
    PaletteFile,
    GoCommand,
    OpenCommand,
    CopyCommand,
    ApproveCommand,
    MergeCommand,
    ReadyCommand,
    PublishCommand,
    ThreadsCommand,
    AllCommand,
    PinCommand,
    UnpinCommand,
    OutlineCommand,
    SetCommand,
    ViewCommand,
    AskCommand,
    AiCommand,
    ShareCommand,
    SessionCommand,
    NewsCommand,
    HelpCommand,
    QuitCommand,
    Pin,
    ViewedNext,
    Search,
    Mine,
    PostNow,
    NewLine,
    WordBack,
    Leave,
    OutlineTree,
    OutlineCalls,
    OutlineStack,
    OutlinePublic,
    AskAnswers,
    AnswerFollowUp,
    AnswerAgain,
    AnswerCopy,
    AnswerDraft,
}

/// A task of the `?` help and its keys; a key that only works in one pane says which in its words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Move,
    Queue,
    View,
    Comment,
    Thread,
    Outline,
    Ask,
    App,
}

/// A line of the `?` help: its keys as the help writes them, and what they do in a few words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Line {
    pub group: Group,
    pub keys: &'static str,
    pub does: &'static str,
}

/// A palette command: the word typed after `:`, and what the list says it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Verb {
    pub word: &'static str,
    pub does: &'static str,
}

/// What revu knows about one action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub act: Act,
    pub name: Name,
    /// The keys revu reads for it outside a text box, as `[keys.bind]` writes them; a binding stands for the first.
    pub keys: &'static [&'static str],
    pub help: &'static [Line],
    pub verb: Option<Verb>,
    /// Where its one-key keys do it.
    pub panes: &'static [Pane],
    /// Where `revu usage` lists it; `None` for an action it does not count.
    pub section: Option<Section>,
}

/// Where a key does an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    /// Wherever the keys go to a pane, before the pane gets them.
    Anywhere,
    Queue,
    Diff,
    /// A thread, note or draft under the diff's cursor, before the diff's own keys.
    ThreadRow,
    /// The right pane's conversations.
    Threads,
    Tree,
    Outline,
    Pipeline,
    /// Claude's answer in the right pane.
    Answer,
    /// The prose view of a Markdown file, after `Page`.
    Prose,
    /// A page that only scrolls: the prose view, the `?` help, the news.
    Page,
    /// The MR cover page, `i`.
    Cover,
}

/// Every pane with a cursor or a scroll: where the moving keys work.
const MOVES: &[Pane] =
    &[Pane::Queue, Pane::Diff, Pane::Threads, Pane::Tree, Pane::Outline, Pane::Pipeline, Pane::Answer, Pane::Page, Pane::Cover];

/// What an action is called, and who uses the name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Name {
    /// `[keys.bind]` can give it more keys; the usage counts and `revu usage` use the name too.
    Bind(&'static str),
    /// Counted, and listed by `revu usage` even when never used.
    Listed(&'static str),
    /// Pressing it is not counted.
    None,
}

impl Name {
    /// The name its usage is counted under.
    pub fn counted(self) -> Option<&'static str> {
        match self {
            Self::Bind(name) | Self::Listed(name) => Some(name),
            Self::None => None,
        }
    }
}

/// The section of `revu usage` an action is listed in, in the order the report shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Move,
    Jump,
    Queue,
    View,
    Comment,
    Thread,
    Ask,
    App,
    Commands,
}

impl Section {
    pub const ALL: [Section; 9] = [
        Section::Move,
        Section::Jump,
        Section::Queue,
        Section::View,
        Section::Comment,
        Section::Thread,
        Section::Ask,
        Section::App,
        Section::Commands,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Move => "move",
            Self::Jump => "jump",
            Self::Queue => "queue",
            Self::View => "view",
            Self::Comment => "comment & publish",
            Self::Thread => "thread pane",
            Self::Ask => "ask claude",
            Self::App => "search & app",
            Self::Commands => "commands",
        }
    }
}

const fn row(act: Act, name: Name, keys: &'static [&'static str]) -> Row {
    Row { act, name, keys, help: &[], verb: None, panes: &[], section: None }
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
            section: Some(Section::Commands),
            ..row($act, Name::Listed(concat!(":", $word)), &[])
        }
    };
}

const fn line(group: Group, keys: &'static str, does: &'static str) -> Line {
    Line { group, keys, does }
}

impl Row {
    const fn help(self, help: &'static [Line]) -> Self {
        Self { help, ..self }
    }

    const fn section(self, section: Section) -> Self {
        Self { section: Some(section), ..self }
    }

    const fn panes(self, panes: &'static [Pane]) -> Self {
        Self { panes, ..self }
    }
}

/// One row per [`Act`], in its order: bindable actions first, then the others `revu usage` lists, then the rest.
pub const ROWS: &[Row] = &[
    bound(Act::NextThread, "next_thread", &["]n", "n"])
        .help(&[
            line(Group::Move, "]n [n", "next, previous open conversation"),
            line(Group::Move, "n N", "the same, while no search is on"),
        ])
        .panes(&[Pane::Diff])
        .section(Section::Jump),
    bound(Act::PrevThread, "prev_thread", &["[n", "N"]).panes(&[Pane::Diff]).section(Section::Jump),
    bound(Act::NextAnyThread, "next_any_thread", &["]N"])
        .help(&[line(Group::Move, "]N [N", "next, previous conversation, resolved too")])
        .section(Section::Jump),
    bound(Act::PrevAnyThread, "prev_any_thread", &["[N"]).section(Section::Jump),
    bound(Act::NextHunk, "next_hunk", &["]c"]).help(&[line(Group::Move, "]c [c", "next, previous hunk")]).section(Section::Jump),
    bound(Act::PrevHunk, "prev_hunk", &["[c"]).section(Section::Jump),
    bound(Act::NextFileUnresolved, "next_file_unresolved", &["]f"])
        .help(&[line(Group::Move, "]f [f", "next, previous file with a thread")])
        .section(Section::Jump),
    bound(Act::PrevFileUnresolved, "prev_file_unresolved", &["[f"]).section(Section::Jump),
    bound(Act::NextReview, "next_review", &["]r"])
        .help(&[line(Group::Move, "]r [r", "next, previous MR that needs you")])
        .section(Section::Jump),
    bound(Act::PrevReview, "prev_review", &["[r"]).section(Section::Jump),
    bound(Act::NextFile, "next_file", &["tab"])
        .help(&[line(Group::Move, "tab S-tab", "next, previous file")])
        .panes(&[Pane::Diff])
        .section(Section::Jump),
    bound(Act::PrevFile, "prev_file", &["backtab"]).panes(&[Pane::Diff]).section(Section::Jump),
    bound(Act::FoldToggle, "fold_toggle", &["za"])
        .help(&[line(Group::View, "za zc zo", "toggle, close, open the fold")])
        .section(Section::View),
    bound(Act::FoldOpen, "fold_open", &["zo"])
        .help(&[
            line(Group::Queue, "zo zc", "open, fold the section or stack"),
            line(Group::Outline, "zo zc za", "open, close, toggle the branch"),
        ])
        .section(Section::View),
    bound(Act::FoldClose, "fold_close", &["zc"]).section(Section::View),
    bound(Act::FoldAll, "fold_all", &["zM"]).help(&[line(Group::View, "zM zR", "fold, unfold every file")]).section(Section::View),
    bound(Act::UnfoldAll, "unfold_all", &["zR"]).section(Section::View),
    bound(Act::FoldHeader, "fold_header", &["zh"]).help(&[line(Group::View, "zh", "fold the MR header")]).section(Section::View),
    bound(Act::Viewed, "viewed", &["zv"]).help(&[line(Group::View, "zv", "file viewed: it folds away")]).section(Section::View),
    bound(Act::Zen, "zen", &["zz"]).help(&[line(Group::View, "zz", "zen: the diff alone, quiet")]).section(Section::View),
    bound(Act::PrevMr, "prev_mr", &["[m"]).section(Section::Jump),
    bound(Act::NextMr, "next_mr", &["]m"]).help(&[line(Group::Move, "]m [m", "next, previous MR in the queue")]).section(Section::Jump),
    bound(Act::SideBySide, "side_by_side", &["D"])
        .help(&[line(Group::View, "D", "inline or side by side, diff and prose")])
        .panes(&[Pane::Diff, Pane::Prose])
        .section(Section::View),
    bound(Act::PeekAfter, "peek_after", &[">"])
        .help(&[line(Group::View, "> <", "the code after, before, alone")])
        .panes(&[Pane::Diff])
        .section(Section::View),
    bound(Act::PeekBefore, "peek_before", &["<"]).panes(&[Pane::Diff]).section(Section::View),
    bound(Act::Tree, "tree", &["t"]).help(&[line(Group::View, "t", "file tree")]).panes(&[Pane::Diff, Pane::Tree]).section(Section::View),
    bound(Act::EveryThread, "every_thread", &["T"])
        .help(&[line(Group::View, "T", "every thread of the MR"), line(Group::Thread, "T", "every thread; enter goes to it")])
        .panes(&[Pane::Diff, Pane::Threads])
        .section(Section::Thread),
    bound(Act::Pipeline, "pipeline", &["p"])
        .help(&[line(Group::View, "p", "pipeline")])
        .panes(&[Pane::Diff, Pane::Pipeline, Pane::Cover])
        .section(Section::View),
    bound(Act::Outline, "outline", &["O"])
        .help(&[line(Group::View, "O", "outline: the symbols the MR changes")])
        .panes(&[Pane::Diff, Pane::Outline])
        .section(Section::View),
    bound(Act::Prose, "prose", &["v"])
        .help(&[line(Group::View, "v", "prose: the Markdown file rendered, its blocks marked")])
        .panes(&[Pane::Diff, Pane::Prose])
        .section(Section::View),
    bound(Act::Wrap, "wrap", &["w"])
        .help(&[line(Group::View, "w W", "wrap long lines, hide whitespace changes")])
        .panes(&[Pane::Diff])
        .section(Section::View),
    bound(Act::Whitespace, "whitespace", &["W"]).panes(&[Pane::Diff]).section(Section::View),
    bound(Act::MoreContext, "more_context", &["="])
        .help(&[line(Group::View, "= +", "more lines, react on a thread")])
        .panes(&[Pane::Diff])
        .section(Section::View),
    bound(Act::ViewFile, "view_file", &["ctrl-v"])
        .help(&[line(Group::View, "^v", "the file in your own program")])
        .panes(&[Pane::Diff, Pane::ThreadRow, Pane::Tree])
        .section(Section::View),
    bound(Act::Description, "description", &["i"])
        .help(&[line(Group::View, "i", "the MR cover page")])
        .panes(&[Pane::Queue, Pane::Diff, Pane::Cover])
        .section(Section::View),
    bound(Act::Comment, "comment", &["c"])
        .help(&[line(Group::Comment, "c", "comment on the line, reply on a thread")])
        .panes(&[Pane::Diff])
        .section(Section::Comment),
    bound(Act::CommentOld, "comment_old", &["C"])
        .help(&[line(Group::Comment, "C", "comment on the old side")])
        .panes(&[Pane::Diff])
        .section(Section::Comment),
    bound(Act::Select, "select", &["V"]).help(&[line(Group::Comment, "V", "select lines")]).panes(&[Pane::Diff]).section(Section::Comment),
    bound(Act::Suggest, "suggest", &["s"])
        .help(&[line(Group::Comment, "s", "suggest a change")])
        .panes(&[Pane::Diff])
        .section(Section::Comment),
    bound(Act::Editor, "editor", &["E"])
        .help(&[line(Group::Comment, "E", "write in $EDITOR")])
        .panes(&[Pane::Diff, Pane::ThreadRow])
        .section(Section::Comment),
    bound(Act::Resolve, "resolve", &["R"])
        .help(&[line(Group::Thread, "R", "resolve, unresolve")])
        .panes(&[Pane::Diff, Pane::ThreadRow])
        .section(Section::Comment),
    bound(Act::Publish, "publish", &["P"])
        .help(&[line(Group::Comment, "P", "publish every draft")])
        .panes(&[Pane::Diff, Pane::Threads])
        .section(Section::Comment),
    bound(Act::Approve, "approve", &["A"])
        .help(&[line(Group::Comment, "A M", "approve or unapprove, merge mine")])
        .panes(&[Pane::Diff])
        .section(Section::Comment),
    bound(Act::Merge, "merge", &["M"]).panes(&[Pane::Diff]).section(Section::Comment),
    bound(Act::Ready, "ready", &["H"])
        .help(&[line(Group::Comment, "H", "mark mine draft or ready")])
        .panes(&[Pane::Diff])
        .section(Section::Comment),
    bound(Act::Share, "share", &["Y"])
        .help(&[line(Group::Comment, "Y", "share the MR, after a preview")])
        .panes(&[Pane::Anywhere])
        .section(Section::Comment),
    bound(Act::Session, "session", &["ag"]).help(&[line(Group::App, "ag", "the agent session of the MR: a, then g")]).section(Section::App),
    bound(Act::React, "react", &["+"]).panes(&[Pane::Diff, Pane::ThreadRow]).section(Section::Thread),
    bound(Act::OpenBrowser, "open_browser", &["o"])
        .help(&[line(Group::App, "o y", "open in the browser, copy the link")])
        .panes(&[Pane::Queue, Pane::Diff, Pane::ThreadRow, Pane::Pipeline, Pane::Cover])
        .section(Section::App),
    bound(Act::CopyLink, "copy_link", &["y"])
        .panes(&[Pane::Queue, Pane::Diff, Pane::ThreadRow, Pane::Pipeline, Pane::Cover])
        .section(Section::App),
    bound(Act::Scope, "scope", &["*"])
        .help(&[line(Group::Queue, "*", "this repo, or every project")])
        .panes(&[Pane::Queue])
        .section(Section::Queue),
    bound(Act::SortQueue, "sort_queue", &["s"])
        .help(&[line(Group::Queue, "s", "next sort order")])
        .panes(&[Pane::Queue])
        .section(Section::Queue),
    bound(Act::GroupByAuthor, "group_by_author", &["S"])
        .help(&[line(Group::Queue, "S", "group by author")])
        .panes(&[Pane::Queue])
        .section(Section::Queue),
    bound(Act::Filter, "filter", &["/"])
        .help(&[line(Group::Queue, "/", "filter: words @author !42 ~label")])
        .panes(&[Pane::Queue])
        .section(Section::Queue),
    bound(Act::Palette, "palette", &[":"])
        .help(&[line(Group::App, ":", "commands: :go !42, :set theme=nord")])
        .panes(&[Pane::Anywhere])
        .section(Section::App),
    bound(Act::Views, "views", &["'", "1", "2", "3", "4", "5", "6", "7", "8", "9"])
        .help(&[line(Group::Queue, "' 1-9", "a saved view, by letter or rank")])
        .panes(&[Pane::Queue])
        .section(Section::Queue),
    bound(Act::Jump, "jump", &["ctrl-k", ","])
        .help(&[line(Group::App, "^k ,", "MRs · / files · > commands")])
        .panes(&[Pane::Anywhere])
        .section(Section::App),
    bound(Act::Help, "help", &["?"]).help(&[line(Group::App, "?", "this help")]).panes(&[Pane::Anywhere]).section(Section::App),
    bound(Act::Quit, "quit", &["q", "ctrl-c"])
        .help(&[line(Group::App, "q ^c", "quit: twice; q closes a pane first")])
        .panes(&[Pane::Anywhere, Pane::Cover, Pane::Page])
        .section(Section::App),
    listed(Act::MoveDown, "move_down", &["j", "down"]).help(&[line(Group::Move, "j k", "move")]).panes(MOVES).section(Section::Move),
    listed(Act::MoveUp, "move_up", &["k", "up"]).panes(MOVES).section(Section::Move),
    listed(Act::PageDown, "page_down", &["ctrl-d"])
        .help(&[line(Group::Move, "^d ^u", "half a page; space too")])
        .panes(MOVES)
        .section(Section::Move),
    listed(Act::PageUp, "page_up", &["ctrl-u"]).panes(MOVES).section(Section::Move),
    listed(Act::FullPageDown, "full_page_down", &["ctrl-f"])
        .help(&[line(Group::Move, "^f ^b", "a full page; PgDn PgUp too")])
        .panes(MOVES)
        .section(Section::Move),
    listed(Act::FullPageUp, "full_page_up", &["ctrl-b"]).panes(MOVES).section(Section::Move),
    listed(Act::Top, "top", &["g"]).help(&[line(Group::Move, "g G", "first, last")]).panes(MOVES).section(Section::Move),
    listed(Act::Bottom, "bottom", &["G"]).panes(MOVES).section(Section::Move),
    listed(Act::Open, "open", &["enter"])
        .help(&[
            line(Group::Move, "enter", "open, or toggle the fold or the thread"),
            line(Group::Outline, "enter", "the call in the diff, the definition, a section"),
        ])
        .panes(&[Pane::Queue, Pane::Diff, Pane::Threads, Pane::Tree, Pane::Outline, Pane::Cover])
        .section(Section::Move),
    listed(Act::Back, "back", &["esc"])
        .panes(&[Pane::Queue, Pane::Diff, Pane::Tree, Pane::Outline, Pane::Pipeline, Pane::Answer, Pane::Prose, Pane::Cover, Pane::Page])
        .section(Section::Move),
    listed(Act::FocusLeft, "focus_left", &["h", "left"])
        .help(&[line(Group::Move, "h l", "pane to the left, to the right")])
        .panes(&[Pane::Anywhere])
        .section(Section::Move),
    listed(Act::FocusRight, "focus_right", &["l", "right"]).panes(&[Pane::Anywhere]).section(Section::Move),
    listed(Act::Refresh, "refresh", &["r"])
        .help(&[line(Group::App, "r", "refresh")])
        .panes(&[Pane::Queue, Pane::Diff, Pane::Outline, Pane::Pipeline, Pane::Prose])
        .section(Section::Queue),
    listed(Act::Reply, "reply", &["r", "c"])
        .help(&[
            line(Group::Comment, "r R e d", "on a thread row: reply, resolve, edit, delete"),
            line(Group::Thread, "r c", "reply, as a draft"),
        ])
        .panes(&[Pane::ThreadRow])
        .section(Section::Thread),
    listed(Act::ApplySuggestion, "apply_suggestion", &["S"])
        .help(&[line(Group::Thread, "S +", "apply the suggestion, react")])
        .panes(&[Pane::ThreadRow])
        .section(Section::Thread),
    listed(Act::EditDraft, "edit_draft", &["e"])
        .help(&[line(Group::Thread, "e d", "edit, delete my draft")])
        .panes(&[Pane::ThreadRow])
        .section(Section::Thread),
    listed(Act::DeleteDraft, "delete_draft", &["d"]).panes(&[Pane::ThreadRow]).section(Section::Thread),
    listed(Act::ClosePane, "close_pane", &["esc", "x"])
        .help(&[line(Group::Move, "esc x", "close the pane, back to the queue")])
        .panes(&[Pane::Diff, Pane::Threads, Pane::Tree, Pane::Outline, Pane::Pipeline, Pane::Answer, Pane::Prose])
        .section(Section::Thread),
    listed(Act::NextInLine, "next_in_line", &["J"])
        .help(&[line(Group::Thread, "J K", "next, previous thread")])
        .panes(&[Pane::Threads])
        .section(Section::Thread),
    listed(Act::PrevInLine, "prev_in_line", &["K"]).panes(&[Pane::Threads]).section(Section::Thread),
    listed(Act::OpenLink, "open_link", &["u"])
        .help(&[line(Group::Thread, "u U", "open, copy the first link")])
        .panes(&[Pane::ThreadRow])
        .section(Section::Thread),
    listed(Act::CopyFirstLink, "copy_first_link", &["U"]).panes(&[Pane::ThreadRow]).section(Section::Thread),
    listed(Act::AskExplain, "ask_explain", &["ae"]).help(&[line(Group::Ask, "a e r s", "explain, risks, summary")]).section(Section::Ask),
    listed(Act::AskRisks, "ask_risks", &["ar"]).section(Section::Ask),
    listed(Act::AskSummary, "ask_summary", &["as"]).section(Section::Ask),
    listed(Act::AskThread, "ask_thread", &["at"]).help(&[line(Group::Ask, "a t c a", "thread, comment, anything")]).section(Section::Ask),
    listed(Act::AskComment, "ask_comment", &["ac"]).section(Section::Ask),
    listed(Act::AskFree, "ask_free", &["aa"]).section(Section::Ask),
    listed(Act::PaletteMr, "palette_mr", &[]).section(Section::App),
    listed(Act::PaletteFile, "palette_file", &[]).section(Section::App),
    command!(Act::GoCommand, "go", "open an MR: :go !42 · :go acme/widgets!42"),
    command!(Act::OpenCommand, "open", "open the MR, or the line, in the browser"),
    command!(Act::CopyCommand, "copy", "copy the link of the MR, or of the line"),
    command!(Act::ApproveCommand, "approve", "approve the open MR, or take the approval back"),
    command!(Act::MergeCommand, "merge", "merge my approved MR, after a yes"),
    command!(Act::ReadyCommand, "ready", "mark my MR ready for review, or a draft again"),
    command!(Act::PublishCommand, "publish", "publish every draft, in the publish modal"),
    command!(Act::ThreadsCommand, "threads", "every thread of the MR in the right pane"),
    command!(Act::AllCommand, "all", "the queue: this repo only, or every project"),
    command!(Act::PinCommand, "pin", "pin the MR on top of the queue"),
    command!(Act::UnpinCommand, "unpin", "take the MR's pin off"),
    command!(Act::OutlineCommand, "outline", "the functions and classes the MR changes, in the right pane"),
    command!(Act::SetCommand, "set", "change and save a setting: :set theme=nord"),
    command!(Act::ViewCommand, "view", "the file in your program: :view · :view old · :view src/a.rs:42"),
    command!(Act::AskCommand, "ask", "ask Claude about the cursor's hunk, file or the MR: :ask is this thread-safe?"),
    command!(Act::AiCommand, "ai", "switch AI off or back on for this session: :ai off, :ai on"),
    command!(Act::ShareCommand, "share", "post the MR through a [share] command: :share · :share review"),
    command!(Act::SessionCommand, "session", "open the agent session of the MR, or start one"),
    command!(Act::NewsCommand, "news", "what changed in revu, the newest version first"),
    command!(Act::HelpCommand, "help", "show the keys"),
    command!(Act::QuitCommand, "quit", "leave"),
    listed(Act::Pin, "pin", &["b"])
        .help(&[line(Group::Queue, "b", "pin on top of the queue, or unpin")])
        .panes(&[Pane::Queue])
        .section(Section::Queue),
    listed(Act::ViewedNext, "viewed", &["m"])
        .help(&[line(Group::View, "m", "file viewed, on to the next one")])
        .panes(&[Pane::Diff])
        .section(Section::View),
    unnamed(Act::Search, &["/"]).help(&[line(Group::View, "/ n N", "find text in the MR, next, previous")]).panes(&[Pane::Diff]),
    unnamed(Act::Mine, &["m"]).help(&[line(Group::Thread, "m", "every thread: only mine, or all")]).panes(&[Pane::Threads]),
    unnamed(Act::PostNow, &[]).help(&[line(Group::Comment, "⌘enter ^s", "in the box: post now, no draft")]),
    unnamed(Act::NewLine, &[]).help(&[line(Group::Comment, "⌥enter ^o", "in the box: new line, $EDITOR")]),
    unnamed(Act::WordBack, &[]).help(&[line(Group::Comment, "⌥← ⌥→ ⌥⌫", "in the box: word back, on, delete")]),
    unnamed(Act::Leave, &["esc"]).help(&[line(Group::Comment, "esc", "drop the selection, leave the box")]),
    unnamed(Act::OutlineTree, &["t"]).help(&[line(Group::Outline, "t", "call tree, or the flat list")]).panes(&[Pane::Outline]),
    unnamed(Act::OutlineCalls, &["u"]).help(&[line(Group::Outline, "u", "calls, or called by")]).panes(&[Pane::Outline]),
    unnamed(Act::OutlineStack, &["s"]).help(&[line(Group::Outline, "s", "the whole stack, unchanged calls too")]).panes(&[Pane::Outline]),
    unnamed(Act::OutlinePublic, &["a"]).help(&[line(Group::Outline, "a", "public symbols only, or all")]).panes(&[Pane::Outline]),
    unnamed(Act::AskAnswers, &["ah"]).help(&[line(Group::Ask, "a h", "answers kept for this MR")]),
    unnamed(Act::AnswerFollowUp, &["enter"]).panes(&[Pane::Answer]),
    unnamed(Act::AnswerAgain, &["R"]).panes(&[Pane::Answer]),
    unnamed(Act::AnswerCopy, &["y"]).panes(&[Pane::Answer]),
    unnamed(Act::AnswerDraft, &["c"]).help(&[line(Group::Ask, "c ⏎ R y", "answer: draft, follow up, again, copy")]).panes(&[Pane::Answer]),
];

impl Group {
    pub fn title(self) -> &'static str {
        match self {
            Self::Move => "move",
            Self::Queue => "queue",
            Self::View => "view",
            Self::Comment => "comment & publish",
            Self::Thread => "thread pane",
            Self::Outline => "outline pane",
            Self::Ask => "ask claude",
            Self::App => "search & app",
        }
    }

    /// The actions whose lines the group lists, in the order it lists them.
    fn acts(self) -> &'static [Act] {
        match self {
            Self::Move => &[
                Act::MoveDown,
                Act::Top,
                Act::PageDown,
                Act::FullPageDown,
                Act::FocusLeft,
                Act::Open,
                Act::ClosePane,
                Act::NextFile,
                Act::NextHunk,
                Act::NextThread,
                Act::NextAnyThread,
                Act::NextFileUnresolved,
                Act::NextReview,
                Act::NextMr,
            ],
            Self::Queue => &[Act::Filter, Act::Views, Act::Scope, Act::SortQueue, Act::GroupByAuthor, Act::Pin, Act::FoldOpen],
            Self::View => &[
                Act::Search,
                Act::Description,
                Act::SideBySide,
                Act::PeekAfter,
                Act::Tree,
                Act::EveryThread,
                Act::Pipeline,
                Act::Outline,
                Act::Prose,
                Act::Zen,
                Act::Wrap,
                Act::MoreContext,
                Act::ViewFile,
                Act::FoldToggle,
                Act::FoldAll,
                Act::FoldHeader,
                Act::Viewed,
                Act::ViewedNext,
            ],
            Self::Comment => &[
                Act::Comment,
                Act::CommentOld,
                Act::Select,
                Act::Suggest,
                Act::Editor,
                Act::Reply,
                Act::PostNow,
                Act::NewLine,
                Act::WordBack,
                Act::Leave,
                Act::Publish,
                Act::Approve,
                Act::Ready,
                Act::Share,
            ],
            Self::Thread => &[
                Act::Reply,
                Act::Resolve,
                Act::NextInLine,
                Act::EveryThread,
                Act::Mine,
                Act::EditDraft,
                Act::ApplySuggestion,
                Act::OpenLink,
            ],
            Self::Outline => &[Act::OutlineTree, Act::OutlineCalls, Act::OutlineStack, Act::OutlinePublic, Act::FoldOpen, Act::Open],
            Self::Ask => &[Act::AskExplain, Act::AskThread, Act::AskAnswers, Act::AnswerDraft],
            Self::App => &[Act::Jump, Act::Palette, Act::OpenBrowser, Act::Session, Act::Refresh, Act::Help, Act::Quit],
        }
    }

    /// Its lines, in the order the help shows them.
    pub fn lines(self) -> impl Iterator<Item = &'static Line> {
        self.acts().iter().flat_map(move |act| act.row().help.iter().filter(move |line| line.group == self))
    }
}

/// The palette's verbs, in the order its list shows them.
pub fn verbs() -> impl Iterator<Item = Verb> {
    ROWS.iter().filter_map(|row| row.verb)
}

impl Act {
    pub fn row(self) -> &'static Row {
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
pub fn act_after(prefix: char, second: char) -> Option<Act> {
    let typed = [KeyEvent::from(KeyCode::Char(prefix)), KeyEvent::from(KeyCode::Char(second))];
    DEFAULT_KEYS.iter().find(|(_, keys)| *keys == typed).map(|(row, _)| row.act)
}

/// The first action `applies` keeps whose one-key default is `code` with exactly `modifiers`.
pub fn with_key(code: KeyCode, modifiers: KeyModifiers, applies: impl Fn(&Row) -> bool) -> Option<Act> {
    let is_it = |(row, keys): &&(&Row, Vec<KeyEvent>)| {
        let is_the_key = matches!(keys.as_slice(), [own] if own.code == code && own.modifiers == modifiers);
        is_the_key && applies(row)
    };
    DEFAULT_KEYS.iter().find(is_it).map(|(row, _)| row.act)
}

/// What `key` does in `pane`: with ctrl held, only a ctrl default; else its plain default, whatever else is held.
/// Where two rows share a key in one pane, the first one wins.
pub fn act_in(pane: Pane, key: KeyEvent) -> Option<Act> {
    let works_here = |row: &Row| row.panes.contains(&pane);
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let modifiers = if ctrl { KeyModifiers::CONTROL } else { KeyModifiers::NONE };
    with_key(key.code, modifiers, works_here)
}

/// Every action `[keys.bind]` can name, with the default key a binding stands for.
pub fn bindable() -> impl Iterator<Item = (&'static str, &'static str)> {
    ROWS.iter().filter_map(|row| match row.name {
        Name::Bind(name) => Some((name, *row.keys.first()?)),
        _ => None,
    })
}

/// `names` in their order, each kept once: `m` and `zv` are both `viewed`.
fn once(names: impl Iterator<Item = &'static str>) -> Vec<&'static str> {
    names.fold(vec![], |mut kept, name| {
        if !kept.contains(&name) {
            kept.push(name);
        }
        kept
    })
}

/// Every action name `revu usage` lists, bindable ones first.
pub fn listed_names() -> Vec<&'static str> {
    once(ROWS.iter().filter_map(|row| row.name.counted()))
}

/// The names `revu usage` lists under `section`, in table order.
pub fn listed_in(section: Section) -> Vec<&'static str> {
    once(ROWS.iter().filter(|row| row.section == Some(section)).filter_map(|row| row.name.counted()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::tui::help::GROUPS;

    #[test]
    fn every_act_has_its_row_in_its_place() {
        for (index, row) in ROWS.iter().enumerate() {
            assert_eq!(row.act as usize, index, "{:?} is out of place", row.act);
        }
        assert_eq!(ROWS.len(), Act::AnswerDraft as usize + 1, "an act has no row");
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
    fn every_help_line_shows_once_and_a_group_lists_only_acts_with_a_line_in_it() {
        let shown: usize = GROUPS.iter().map(|group| group.lines().count()).sum();
        assert_eq!(shown, ROWS.iter().map(|row| row.help.len()).sum::<usize>(), "a help line no group lists, or one listed twice");
        for group in GROUPS {
            for act in group.acts() {
                assert!(act.row().help.iter().any(|line| line.group == group), "{act:?} has no line in {group:?}");
            }
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
            assert_eq!(row.section.is_some(), row.name.counted().is_some(), "{:?}", row.act);
        }
    }

    #[test]
    fn every_command_is_in_the_palette() {
        assert_eq!(verbs().count(), ROWS.iter().filter(|row| row.verb.is_some()).count());
    }
}
