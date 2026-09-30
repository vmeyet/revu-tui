//! `^v` and `:view`: which file, which version, which line; the loop hands the terminal over.
use super::{Action, App};
use crate::open;
use crate::review::{FileKind, Side};

impl App {
    /// `v` in the review: the file under the cursor, at head, at the cursor's line.
    pub(super) fn view_here(&mut self, side: Side) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let Some(row) = open.row().cloned() else { return vec![] };
        let Some(file) = row.file() else {
            self.toast("move onto a file first");
            return vec![];
        };
        let lines = (open::line_for(&open.review, &row, Side::New), open::line_for(&open.review, &row, Side::Old));
        self.view(file, side, lines)
    }

    /// `v` in the pane: the file at the line of the thread, or draft, under the cursor.
    pub(super) fn view_thread(&mut self) -> Vec<Action> {
        let Some(open) = &self.open else { return vec![] };
        let thread = open.focused_thread().and_then(|id| open.review.thread(&id)).and_then(|t| t.anchor.clone());
        let anchor = thread.or_else(|| open.focused_draft().and_then(|i| open.review.drafts[i].anchor.clone()));
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crate::tui::app::test_support::*;

    fn view(path: &str, sha: &str, line: u32, note: Option<&str>) -> Action {
        Action::View { key: mr_key(), path: path.into(), sha: sha.into(), line, note: note.map(str::to_owned) }
    }

    #[test]
    fn ctrl_v_hands_the_head_file_at_the_cursors_line() {
        let mut app = with_review();
        on_line(&mut app);
        assert_eq!(app.handle_key(ctrl('v')), vec![view("src/pay/charge.rs", "bbbb", 12, None)]);
        assert!(app.live_toast().unwrap().text.contains("opening charge.rs"));
        press(&mut app, "j");
        assert_eq!(app.handle_key(ctrl('v')), vec![view("src/pay/charge.rs", "bbbb", 13, None)], "a removed line opens the next head line");
    }

    #[test]
    fn a_plain_v_opens_nothing() {
        let mut app = with_review();
        on_line(&mut app);
        assert_eq!(press(&mut app, "v"), vec![]);
    }

    #[test]
    fn view_old_hands_the_base_file_and_view_path_any_file() {
        let mut app = with_review();
        on_line(&mut app);
        assert_eq!(type_palette(&mut app, "view old"), vec![view("src/pay/charge.rs", "aaaa", 12, None)]);
        assert_eq!(type_palette(&mut app, "view src/pay/charge.rs:41"), vec![view("src/pay/charge.rs", "bbbb", 41, None)]);
        assert_eq!(type_palette(&mut app, "view nope.rs"), vec![]);
        assert!(app.live_toast().unwrap().danger);
    }

    fn type_palette(app: &mut App, line: &str) -> Vec<Action> {
        press(app, ":");
        press(app, line);
        app.handle_key(code(KeyCode::Enter))
    }

    #[test]
    fn ctrl_v_in_the_thread_pane_and_the_tree_uses_their_file() {
        let mut app = with_review();
        press(&mut app, "]N");
        app.handle_key(code(KeyCode::Enter));
        assert_eq!(app.focus, Focus::Side);
        assert_eq!(app.handle_key(ctrl('v')), vec![view("src/pay/charge.rs", "bbbb", 13, None)], "the old line 13 opens at head line 13");
        app.close_pane();
        press(&mut app, "t");
        assert_eq!(app.handle_key(ctrl('v')), vec![view("src/pay/charge.rs", "bbbb", 1, None)], "the tree opens on the cursor's file");
    }

    #[test]
    fn deleted_and_binary_files_say_what_they_can() {
        let mut app = with_review();
        let deleted = DiffFile {
            diff: "@@ -1,2 +0,0 @@\n-a\n-b\n".into(),
            old_path: "src/gone.rs".into(),
            new_path: "src/gone.rs".into(),
            change: crate::forge::FileKind::Deleted,
            ..DiffFile::default()
        };
        let binary = DiffFile { old_path: "logo.png".into(), new_path: "logo.png".into(), ..DiffFile::default() };
        let review = Review::new(mr(), &[deleted, binary], vec![], &[]);
        app.apply(Incoming::Review { key: mr_key(), review: Box::new(review), cached: None });
        app.review_jump_to(|row| matches!(row, Row::Line { file: 0, .. }));
        assert_eq!(app.handle_key(ctrl('v')), vec![view("src/gone.rs", "aaaa", 1, Some("deleted in this MR · showing the old file"))]);
        app.review_jump_to(|row| matches!(row, Row::File { index: 1, .. }));
        assert_eq!(app.handle_key(ctrl('v')), vec![]);
        assert!(app.live_toast().unwrap().text.starts_with("binary file"));
    }

    fn ready(key: MrKey) -> Incoming {
        let view = crate::open::View {
            argv: vec!["hx".into(), "/tmp/charge.rs:12".into()],
            shown: "charge.rs:12".into(),
            note: None,
            _copy: None,
        };
        Incoming::ViewReady { key, view }
    }

    #[test]
    fn a_file_ready_for_another_mr_is_dropped_and_the_way_back_is_said() {
        let mut app = with_review();
        app.apply(ready(MrKey::new("acme/other", 7)));
        assert!(app.take_view().is_none(), "the reader moved on: no program jumps on screen");
        app.apply(ready(mr_key()));
        let view = app.take_view().unwrap();
        app.apply(Incoming::Viewed { view: view.clone(), outcome: Ok(()) });
        assert_eq!(app.live_toast().unwrap().text, "back from hx · charge.rs:12");
        app.apply(Incoming::Viewed { view, outcome: Err("hx not found · set [open] default in config".into()) });
        assert!(app.live_toast().unwrap().danger);
    }
}
