//! Every action revu has, in one table: the name `[keys.bind]` and `revu usage` give it, its default keys, its
//! lines in the `?` help and its palette verb. The keymap, the help, the palette, the usage report and
//! `docs/reference/keys.md` all read it from here.

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
    PublishCommand,
    ThreadsCommand,
    AllCommand,
    SetCommand,
    ViewCommand,
    AiCommand,
    AskCommand,
    ShareCommand,
    HelpCommand,
    QuitCommand,
    ReadyCommand,
    PinCommand,
    UnpinCommand,
    OutlineCommand,
    SessionCommand,
    NewsCommand,
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
}

/// What an action is called, and who uses the name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Name {
    /// `[keys.bind]` can give it more keys; the usage counts and `revu usage` use the name too.
    Bind(&'static str),
    /// Counted, and listed by `revu usage` even when never used.
    Listed(&'static str),
    /// Counted, but `revu usage` names it only once used.
    Counted(&'static str),
    /// Pressing it is not counted.
    None,
}

impl Name {
    /// The name its usage is counted under.
    pub fn counted(self) -> Option<&'static str> {
        match self {
            Self::Bind(name) | Self::Listed(name) | Self::Counted(name) => Some(name),
            Self::None => None,
        }
    }
}

const fn row(act: Act, name: Name, keys: &'static [&'static str]) -> Row {
    Row { act, name, keys, help: &[], verb: None }
}

const fn bound(act: Act, name: &'static str, keys: &'static [&'static str]) -> Row {
    row(act, Name::Bind(name), keys)
}

const fn listed(act: Act, name: &'static str, keys: &'static [&'static str]) -> Row {
    row(act, Name::Listed(name), keys)
}

const fn counted(act: Act, name: &'static str, keys: &'static [&'static str]) -> Row {
    row(act, Name::Counted(name), keys)
}

const fn unnamed(act: Act, keys: &'static [&'static str]) -> Row {
    row(act, Name::None, keys)
}

/// A palette command, counted as its word after a colon: `:go`.
macro_rules! command {
    ($kind:ident, $act:expr, $word:literal, $does:literal) => {
        Row { verb: Some(Verb { word: $word, does: $does }), ..row($act, Name::$kind(concat!(":", $word)), &[]) }
    };
}

const fn line(group: Group, keys: &'static str, does: &'static str) -> Line {
    Line { group, keys, does }
}

impl Row {
    const fn help(self, help: &'static [Line]) -> Self {
        Self { help, ..self }
    }
}

/// One row per [`Act`], in its order: bindable actions first, then the others `revu usage` lists, then the rest.
pub const ROWS: &[Row] = &[
    bound(Act::NextThread, "next_thread", &["]n", "n"]).help(&[
        line(Group::Move, "]n [n", "next, previous open conversation"),
        line(Group::Move, "n N", "the same, while no search is on"),
    ]),
    bound(Act::PrevThread, "prev_thread", &["[n", "N"]),
    bound(Act::NextAnyThread, "next_any_thread", &["]N"]).help(&[line(Group::Move, "]N [N", "next, previous conversation, resolved too")]),
    bound(Act::PrevAnyThread, "prev_any_thread", &["[N"]),
    bound(Act::NextHunk, "next_hunk", &["]c"]).help(&[line(Group::Move, "]c [c", "next, previous hunk")]),
    bound(Act::PrevHunk, "prev_hunk", &["[c"]),
    bound(Act::NextFileUnresolved, "next_file_unresolved", &["]f"]).help(&[line(
        Group::Move,
        "]f [f",
        "next, previous file with a thread",
    )]),
    bound(Act::PrevFileUnresolved, "prev_file_unresolved", &["[f"]),
    bound(Act::NextReview, "next_review", &["]r"]).help(&[line(Group::Move, "]r [r", "next, previous MR that needs you")]),
    bound(Act::PrevReview, "prev_review", &["[r"]),
    bound(Act::NextFile, "next_file", &["tab"]).help(&[line(Group::Move, "tab S-tab", "next, previous file")]),
    bound(Act::PrevFile, "prev_file", &["backtab"]),
    bound(Act::FoldToggle, "fold_toggle", &["za"]).help(&[line(Group::View, "za zc zo", "toggle, close, open the fold")]),
    bound(Act::FoldOpen, "fold_open", &["zo"]).help(&[
        line(Group::Queue, "zo zc", "open, fold the section or stack"),
        line(Group::Outline, "zo zc za", "open, close, toggle the branch"),
    ]),
    bound(Act::FoldClose, "fold_close", &["zc"]),
    bound(Act::FoldAll, "fold_all", &["zM"]).help(&[line(Group::View, "zM zR", "fold, unfold every file")]),
    bound(Act::UnfoldAll, "unfold_all", &["zR"]),
    bound(Act::FoldHeader, "fold_header", &["zh"]).help(&[line(Group::View, "zh", "fold the MR header")]),
    bound(Act::Viewed, "viewed", &["zv"]).help(&[line(Group::View, "zv", "file viewed: it folds away")]),
    bound(Act::Zen, "zen", &["zz"]).help(&[line(Group::View, "zz", "zen: the diff alone, quiet")]),
    bound(Act::PrevMr, "prev_mr", &["[m"]),
    bound(Act::NextMr, "next_mr", &["]m"]).help(&[line(Group::Move, "]m [m", "next, previous MR in the queue")]),
    bound(Act::SideBySide, "side_by_side", &["D"]).help(&[line(Group::View, "D", "inline or side by side, diff and prose")]),
    bound(Act::PeekAfter, "peek_after", &[">"]).help(&[line(Group::View, "> <", "the code after, before, alone")]),
    bound(Act::PeekBefore, "peek_before", &["<"]),
    bound(Act::Tree, "tree", &["t"]).help(&[line(Group::View, "t", "file tree")]),
    bound(Act::EveryThread, "every_thread", &["T"])
        .help(&[line(Group::View, "T", "every thread of the MR"), line(Group::Thread, "T", "every thread; enter goes to it")]),
    bound(Act::Pipeline, "pipeline", &["p"]).help(&[line(Group::View, "p", "pipeline")]),
    bound(Act::Outline, "outline", &["O"]).help(&[line(Group::View, "O", "outline: the symbols the MR changes")]),
    bound(Act::Prose, "prose", &["v"]).help(&[line(Group::View, "v", "prose: the Markdown file rendered, its blocks marked")]),
    bound(Act::Wrap, "wrap", &["w"]).help(&[line(Group::View, "w W", "wrap long lines, hide whitespace changes")]),
    bound(Act::Whitespace, "whitespace", &["W"]),
    bound(Act::MoreContext, "more_context", &["="]).help(&[line(Group::View, "= +", "more lines, react on a thread")]),
    bound(Act::ViewFile, "view_file", &["ctrl-v"]).help(&[line(Group::View, "^v", "the file in your own program")]),
    bound(Act::Description, "description", &["i"]).help(&[line(Group::View, "i", "the MR cover page")]),
    bound(Act::Comment, "comment", &["c"]).help(&[line(Group::Comment, "c", "comment on the line, reply on a thread")]),
    bound(Act::CommentOld, "comment_old", &["C"]).help(&[line(Group::Comment, "C", "comment on the old side")]),
    bound(Act::Select, "select", &["V"]).help(&[line(Group::Comment, "V", "select lines")]),
    bound(Act::Suggest, "suggest", &["s"]).help(&[line(Group::Comment, "s", "suggest a change")]),
    bound(Act::Editor, "editor", &["E"]).help(&[line(Group::Comment, "E", "write in $EDITOR")]),
    bound(Act::Resolve, "resolve", &["R"]).help(&[line(Group::Thread, "R", "resolve, unresolve")]),
    bound(Act::Publish, "publish", &["P"]).help(&[line(Group::Comment, "P", "publish every draft")]),
    bound(Act::Approve, "approve", &["A"]).help(&[line(Group::Comment, "A M", "approve or unapprove, merge mine")]),
    bound(Act::Merge, "merge", &["M"]),
    bound(Act::Ready, "ready", &["H"]).help(&[line(Group::Comment, "H", "mark mine draft or ready")]),
    bound(Act::Share, "share", &["Y"]).help(&[line(Group::Comment, "Y", "share the MR, after a preview")]),
    bound(Act::Session, "session", &["ag"]).help(&[line(Group::App, "ag", "the agent session of the MR: a, then g")]),
    bound(Act::React, "react", &["+"]),
    bound(Act::OpenBrowser, "open_browser", &["o"]).help(&[line(Group::App, "o y", "open in the browser, copy the link")]),
    bound(Act::CopyLink, "copy_link", &["y"]),
    bound(Act::Scope, "scope", &["*"]).help(&[line(Group::Queue, "*", "this repo, or every project")]),
    bound(Act::SortQueue, "sort_queue", &["s"]).help(&[line(Group::Queue, "s", "next sort order")]),
    bound(Act::GroupByAuthor, "group_by_author", &["S"]).help(&[line(Group::Queue, "S", "group by author")]),
    bound(Act::Filter, "filter", &["/"]).help(&[line(Group::Queue, "/", "filter: words @author !42 ~label")]),
    bound(Act::Palette, "palette", &[":"]).help(&[line(Group::App, ":", "commands: :go !42, :set theme=nord")]),
    bound(Act::Views, "views", &["'", "1", "2", "3", "4", "5", "6", "7", "8", "9"]).help(&[line(
        Group::Queue,
        "' 1-9",
        "a saved view, by letter or rank",
    )]),
    bound(Act::Jump, "jump", &["ctrl-k", ","]).help(&[line(Group::App, "^k ,", "MRs · / files · > commands")]),
    bound(Act::Help, "help", &["?"]).help(&[line(Group::App, "?", "this help")]),
    bound(Act::Quit, "quit", &["q", "ctrl-c"]).help(&[line(Group::App, "q ^c", "quit: twice; q closes a pane first")]),
    listed(Act::MoveDown, "move_down", &["j", "down"]).help(&[line(Group::Move, "j k", "move")]),
    listed(Act::MoveUp, "move_up", &["k", "up"]),
    listed(Act::PageDown, "page_down", &["ctrl-d"]).help(&[line(Group::Move, "^d ^u", "half a page; space too")]),
    listed(Act::PageUp, "page_up", &["ctrl-u"]),
    listed(Act::FullPageDown, "full_page_down", &["ctrl-f"]).help(&[line(Group::Move, "^f ^b", "a full page; PgDn PgUp too")]),
    listed(Act::FullPageUp, "full_page_up", &["ctrl-b"]),
    listed(Act::Top, "top", &["g"]).help(&[line(Group::Move, "g G", "first, last")]),
    listed(Act::Bottom, "bottom", &["G"]),
    listed(Act::Open, "open", &["enter"]).help(&[
        line(Group::Move, "enter", "open, or toggle the fold or the thread"),
        line(Group::Outline, "enter", "the call in the diff, the definition, a section"),
    ]),
    listed(Act::Back, "back", &["esc"]),
    listed(Act::FocusLeft, "focus_left", &["h", "left"]).help(&[line(Group::Move, "h l", "pane to the left, to the right")]),
    listed(Act::FocusRight, "focus_right", &["l", "right"]),
    listed(Act::Refresh, "refresh", &["r"]).help(&[line(Group::App, "r", "refresh")]),
    listed(Act::Reply, "reply", &["r", "c"]).help(&[
        line(Group::Comment, "r R e d", "on a thread row: reply, resolve, edit, delete"),
        line(Group::Thread, "r c", "reply, as a draft"),
    ]),
    listed(Act::ApplySuggestion, "apply_suggestion", &["S"]).help(&[line(Group::Thread, "S +", "apply the suggestion, react")]),
    listed(Act::EditDraft, "edit_draft", &["e"]).help(&[line(Group::Thread, "e d", "edit, delete my draft")]),
    listed(Act::DeleteDraft, "delete_draft", &["d"]),
    listed(Act::ClosePane, "close_pane", &["esc", "x"]).help(&[line(Group::Move, "esc x", "close the pane, back to the queue")]),
    listed(Act::NextInLine, "next_in_line", &["J"]).help(&[line(Group::Thread, "J K", "next, previous thread")]),
    listed(Act::PrevInLine, "prev_in_line", &["K"]),
    listed(Act::OpenLink, "open_link", &["u"]).help(&[line(Group::Thread, "u U", "open, copy the first link")]),
    listed(Act::CopyFirstLink, "copy_first_link", &["U"]),
    listed(Act::AskExplain, "ask_explain", &["ae"]).help(&[line(Group::Ask, "a e r s", "explain, risks, summary")]),
    listed(Act::AskRisks, "ask_risks", &["ar"]),
    listed(Act::AskSummary, "ask_summary", &["as"]),
    listed(Act::AskThread, "ask_thread", &["at"]).help(&[line(Group::Ask, "a t c a", "thread, comment, anything")]),
    listed(Act::AskComment, "ask_comment", &["ac"]),
    listed(Act::AskFree, "ask_free", &["aa"]),
    listed(Act::PaletteMr, "palette_mr", &[]),
    listed(Act::PaletteFile, "palette_file", &[]),
    command!(Listed, Act::GoCommand, "go", "open an MR: :go !42 · :go acme/widgets!42"),
    command!(Listed, Act::OpenCommand, "open", "open the MR, or the line, in the browser"),
    command!(Listed, Act::CopyCommand, "copy", "copy the link of the MR, or of the line"),
    command!(Listed, Act::ApproveCommand, "approve", "approve the open MR, or take the approval back"),
    command!(Listed, Act::MergeCommand, "merge", "merge my approved MR, after a yes"),
    command!(Listed, Act::PublishCommand, "publish", "publish every draft, in the publish modal"),
    command!(Listed, Act::ThreadsCommand, "threads", "every thread of the MR in the right pane"),
    command!(Listed, Act::AllCommand, "all", "the queue: this repo only, or every project"),
    command!(Listed, Act::SetCommand, "set", "change and save a setting: :set theme=nord"),
    command!(Listed, Act::ViewCommand, "view", "the file in your program: :view · :view old · :view src/a.rs:42"),
    command!(Listed, Act::AiCommand, "ai", "switch AI off or back on for this session: :ai off, :ai on"),
    command!(Listed, Act::AskCommand, "ask", "ask Claude about the cursor's hunk, file or the MR: :ask is this thread-safe?"),
    command!(Listed, Act::ShareCommand, "share", "post the MR through a [share] command: :share · :share review"),
    command!(Listed, Act::HelpCommand, "help", "show the keys"),
    command!(Listed, Act::QuitCommand, "quit", "leave"),
    command!(Counted, Act::ReadyCommand, "ready", "mark my MR ready for review, or a draft again"),
    command!(Counted, Act::PinCommand, "pin", "pin the MR on top of the queue"),
    command!(Counted, Act::UnpinCommand, "unpin", "take the MR's pin off"),
    command!(Counted, Act::OutlineCommand, "outline", "the functions and classes the MR changes, in the right pane"),
    command!(Counted, Act::SessionCommand, "session", "open the agent session of the MR, or start one"),
    command!(Counted, Act::NewsCommand, "news", "what changed in revu, the newest version first"),
    counted(Act::Pin, "pin", &["b"]).help(&[line(Group::Queue, "b", "pin on top of the queue, or unpin")]),
    counted(Act::ViewedNext, "viewed", &["m"]).help(&[line(Group::View, "m", "file viewed, on to the next one")]),
    unnamed(Act::Search, &["/"]).help(&[line(Group::View, "/ n N", "find text in the MR, next, previous")]),
    unnamed(Act::Mine, &["m"]).help(&[line(Group::Thread, "m", "every thread: only mine, or all")]),
    unnamed(Act::PostNow, &[]).help(&[line(Group::Comment, "⌘enter ^s", "in the box: post now, no draft")]),
    unnamed(Act::NewLine, &[]).help(&[line(Group::Comment, "⌥enter ^o", "in the box: new line, $EDITOR")]),
    unnamed(Act::WordBack, &[]).help(&[line(Group::Comment, "⌥← ⌥→ ⌥⌫", "in the box: word back, on, delete")]),
    unnamed(Act::Leave, &["esc"]).help(&[line(Group::Comment, "esc", "drop the selection, leave the box")]),
    unnamed(Act::OutlineTree, &["t"]).help(&[line(Group::Outline, "t", "call tree, or the flat list")]),
    unnamed(Act::OutlineCalls, &["u"]).help(&[line(Group::Outline, "u", "calls, or called by")]),
    unnamed(Act::OutlineStack, &["s"]).help(&[line(Group::Outline, "s", "the whole stack, unchanged calls too")]),
    unnamed(Act::OutlinePublic, &["a"]).help(&[line(Group::Outline, "a", "public symbols only, or all")]),
    unnamed(Act::AskAnswers, &["ah"]).help(&[line(Group::Ask, "a h", "answers kept for this MR")]),
    unnamed(Act::AnswerDraft, &["c"]).help(&[line(Group::Ask, "c ⏎ R y", "answer: draft, follow up, again, copy")]),
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

/// The palette's commands, in the order its list shows them.
const COMMANDS: [Act; 21] = [
    Act::GoCommand,
    Act::OpenCommand,
    Act::CopyCommand,
    Act::ApproveCommand,
    Act::MergeCommand,
    Act::ReadyCommand,
    Act::PublishCommand,
    Act::ThreadsCommand,
    Act::AllCommand,
    Act::PinCommand,
    Act::UnpinCommand,
    Act::OutlineCommand,
    Act::SetCommand,
    Act::ViewCommand,
    Act::AskCommand,
    Act::AiCommand,
    Act::ShareCommand,
    Act::SessionCommand,
    Act::NewsCommand,
    Act::HelpCommand,
    Act::QuitCommand,
];

/// The palette's verbs, in the order its list shows them.
pub fn verbs() -> impl Iterator<Item = Verb> {
    COMMANDS.iter().filter_map(|act| act.row().verb)
}

impl Act {
    pub fn row(self) -> &'static Row {
        &ROWS[self as usize]
    }
}

/// Every action `[keys.bind]` can name, with the default key a binding stands for.
pub fn bindable() -> impl Iterator<Item = (&'static str, &'static str)> {
    ROWS.iter().filter_map(|row| match row.name {
        Name::Bind(name) => Some((name, *row.keys.first()?)),
        _ => None,
    })
}

/// Every action name `revu usage` lists, bindable ones first.
pub fn listed_names() -> impl Iterator<Item = &'static str> {
    ROWS.iter().filter_map(|row| match row.name {
        Name::Bind(name) | Name::Listed(name) => Some(name),
        _ => None,
    })
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

    #[test]
    fn every_command_is_in_the_palette() {
        assert_eq!(verbs().count(), ROWS.iter().filter(|row| row.verb.is_some()).count());
    }
}
