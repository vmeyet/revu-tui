//! `v` and `:view`: which file, which version, which line; the loop hands the terminal over.
use super::{Action, App};
use crate::open;
use crate::review::{FileKind, Side};

impl App {
    /// `v` in the review: the file under the cursor, at head, at the cursor's line.
    pub(super) fn view_here(&mut self, side: Side) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let Some(row) = open.row().cloned() else { return vec![] };
        let Some(file) = open.file_of(&row) else {
            self.toast("move onto a file first");
            return vec![];
        };
        let lines = (open::line_for(&open.review, &row, Side::New), open::line_for(&open.review, &row, Side::Old));
        self.view(file, side, lines)
    }

    /// `v` in the thread pane: the file at the open thread's line.
    pub(super) fn view_thread(&mut self) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let anchor = open.thread.as_ref().and_then(|id| open.review.thread(id)).and_then(|t| t.anchor.clone());
        let Some(anchor) = anchor else {
            self.toast("this thread is on the MR, not on a file");
            return vec![];
        };
        let Some(file) = open.review.files.iter().position(|f| f.new_path == anchor.path || f.old_path == anchor.path) else {
            self.toast("this thread's file is no longer in the MR");
            return vec![];
        };
        let lines = (open::line_of_anchor(&open.review, &anchor, Side::New), open::line_of_anchor(&open.review, &anchor, Side::Old));
        self.view(file, Side::New, lines)
    }

    /// `v` on a file of the tree: its first line.
    pub(super) fn view_file(&mut self, file: usize) -> Vec<Action> {
        self.view(file, Side::New, (1, 1))
    }

    /// `:view`, `:view old`, `:view src/a.rs:42`.
    pub(super) fn view_command(&mut self, argument: &str) -> Vec<Action> {
        match argument.trim() {
            "" => self.view_here(Side::New),
            "old" => self.view_here(Side::Old),
            place => {
                let (path, line) = place.rsplit_once(':').and_then(|(path, line)| Some((path, line.parse().ok()?))).unwrap_or((place, 1));
                let file = self.open.as_ref().and_then(|o| o.review.files.iter().position(|f| f.new_path == path || f.old_path == path));
                let Some(file) = file else {
                    self.warn(format!("{path} is not a file of this MR"));
                    return vec![];
                };
                self.view(file, Side::New, (line, line))
            }
        }
    }

    /// Asks the loop for `file` of the open MR, on `side`; `lines` are its head and base lines.
    fn view(&mut self, file: usize, side: Side, lines: (u32, u32)) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let file = &open.review.files[file];
        if file.binary {
            self.toast("binary file · o opens it in the browser");
            return vec![];
        }
        let (side, note) = match (side, file.kind) {
            (Side::New, FileKind::Deleted) => (Side::Old, Some("deleted in this MR · showing the old file".to_owned())),
            (Side::Old, FileKind::Added) => {
                self.toast("added in this MR · there is no old file");
                return vec![];
            }
            (side, _) => (side, None),
        };
        let refs = &open.review.mr.refs;
        let (path, sha, line) = match side {
            Side::New => (file.new_path.clone(), refs.head.clone(), lines.0),
            Side::Old => (file.old_path.clone(), refs.base.clone(), lines.1),
        };
        let key = open.key.clone();
        self.toast(format!("opening {}…", path.rsplit('/').next().unwrap_or(&path)));
        vec![Action::View { key, path, sha, line, note }]
    }

    /// The loop's cue: a file is ready, unless the reader moved on to another MR meanwhile.
    pub(super) fn apply_view_ready(&mut self, key: &super::MrKey, view: open::View) {
        if self.open.as_ref().is_some_and(|o| &o.key == key) {
            self.viewing = Some(view);
        }
    }

    /// Taken by the loop, which gives the terminal to the program.
    pub fn take_view(&mut self) -> Option<open::View> {
        self.viewing.take()
    }

    /// Back from the program: what happened, in the status line.
    pub(super) fn apply_viewed(&mut self, view: &open::View, outcome: Result<(), String>) {
        match outcome {
            Ok(()) => {
                let note = view.note.as_ref().map(|n| format!(" · {n}")).unwrap_or_default();
                self.toast(format!("back from {} · {}{note}", view.program(), view.shown));
            }
            Err(message) => self.warn(message),
        }
    }
}
