#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::forge::gitlab::fixture;
use crate::forge::{DiffFile, Discussion, Kind, Mr};
use crate::review::Row;
use crate::tui::theme::Theme;
use crate::tui::ui;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;
use serde_json::json;
use std::time::Duration;

fn mr_key() -> MrKey {
    fixture::key()
}

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

fn code(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn press(app: &mut App, keys: &str) -> Vec<Action> {
    keys.chars().flat_map(|c| app.handle_key(key(c))).collect()
}

fn today() -> DateTime<Utc> {
    "2026-09-22T12:00:00Z".parse().unwrap()
}

fn settings() -> Settings {
    Settings { theme: Theme::default(), host: "gitlab.com".into(), kind: Kind::GitLab, me: "nina".into(), project: None }
}

fn app() -> App {
    let mut app = App::new(settings());
    app.today = today();
    app
}

fn sections() -> Sections {
    fixture::queue(include_str!("../../forge/gitlab/fixtures/queue.json")).sections(&[])
}

fn with_queue() -> App {
    let mut app = app();
    app.apply(Incoming::Queue { scope: None, sections: sections(), opened: HashMap::new(), cached: false });
    app
}

fn mr() -> Mr {
    fixture::mr(
        &json!({
            "id": 1042, "iid": 42, "project_id": 7, "title": "feat: charge cards at checkout",
            "state": "opened", "draft": false,
            "author": {"id": 5, "username": "omar", "name": "Omar"},
            "source_branch": "feat/checkout", "target_branch": "main",
            "web_url": "https://gitlab.com/acme/widgets/-/merge_requests/42",
            "updated_at": "2026-09-22T09:12:00Z", "sha": "bbbb",
            "diff_refs": {"base_sha": "aaaa", "head_sha": "bbbb", "start_sha": "aaaa"},
            "head_pipeline": {"status": "success", "web_url": "https://gitlab.com/acme/widgets/-/pipelines/1"},
            "approvals": {"approved": false, "approvals_left": 1, "approved_by": [{"user": {"id": 3, "username": "lea", "name": "Léa"}}]}
        })
        .to_string(),
    )
}

fn diffs() -> Vec<DiffFile> {
    vec![
        DiffFile {
            diff: include_str!("../../review/fixtures/charge.diff").to_owned(),
            old_path: "src/pay/charge.rs".into(),
            new_path: "src/pay/charge.rs".into(),
            a_mode: "100644".into(),
            b_mode: "100644".into(),
            ..DiffFile::default()
        },
        DiffFile {
            diff: "@@ -1 +1 @@\n-a\n+b\n".into(),
            old_path: "Cargo.lock".into(),
            new_path: "Cargo.lock".into(),
            ..DiffFile::default()
        },
    ]
}

fn discussions() -> Vec<Discussion> {
    vec![
        fixture::discussion(include_str!("../../forge/gitlab/fixtures/discussions.json")),
        fixture::discussion(include_str!("../../forge/gitlab/fixtures/diff_note.json")),
        fixture::discussion(include_str!("../../review/fixtures/old_side_note.json")),
    ]
}

fn review() -> Review {
    Review::new(mr(), &diffs(), discussions(), &["*.lock".into()])
}

fn with_review() -> App {
    let mut app = with_queue();
    app.queue_move(0);
    assert_eq!(press(&mut app, "\r"), vec![]);
    let actions = app.handle_key(code(KeyCode::Enter));
    assert_eq!(actions, vec![Action::Open(mr_key())]);
    app.apply(Incoming::Review { key: mr_key(), review: Box::new(review()), cached: None });
    app
}

fn render(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| ui::draw(f, app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..height)
        .map(|y| (0..width).map(|x| buffer[(x, y)].symbol().to_owned()).collect::<String>().trim_end().to_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn starts_by_loading_the_queue() {
    assert_eq!(app().start(), vec![Action::LoadQueue { scope: None, from_cache: true }]);
}

#[test]
fn the_queue_lands_in_sections_and_the_cursor_on_the_first_mr() {
    let app = with_queue();
    assert!(!app.queue_loading);
    assert!(app.poll.queue_due.is_some());
    assert_eq!(app.selected_mr().map(|m| m.number), Some(42));
    let rows = app.queue_rows();
    assert!(matches!(rows[0], QueueRow::Section { name: "TO REVIEW", count: 1, open: true }));
    assert!(rows.iter().any(|r| matches!(r, QueueRow::Section { name: "DONE", open: false, .. })));
}

#[test]
fn j_and_k_skip_section_headers_and_stop_at_the_ends() {
    let mut app = with_queue();
    press(&mut app, "j");
    assert_eq!(app.selected_mr().map(|m| m.number), Some(41), "MINE header is skipped");
    press(&mut app, "kkk");
    assert_eq!(app.selected_mr().map(|m| m.number), Some(42));
    press(&mut app, "G");
    assert_eq!(app.selected_mr().map(|m| m.number), Some(35));
    press(&mut app, "g");
    assert_eq!(app.selected_mr().map(|m| m.number), Some(42));
}

#[test]
fn zo_in_the_queue_shows_the_done_section() {
    let mut app = with_queue();
    press(&mut app, "zo");
    assert!(app.done_open);
    press(&mut app, "G");
    assert_eq!(app.selected_mr().map(|m| m.number), Some(40));
    press(&mut app, "zc");
    assert_eq!(app.selected_mr().map(|m| m.number), Some(35), "the cursor settles on a visible row");
}

#[test]
fn the_filter_narrows_live_and_esc_clears_it() {
    let mut app = with_queue();
    press(&mut app, "/runner");
    assert!(app.filtering);
    assert_eq!(app.selected_mr().map(|m| m.number), Some(35));
    app.handle_key(code(KeyCode::Enter));
    assert!(!app.filtering && app.filter == "runner");
    app.handle_key(code(KeyCode::Esc));
    assert!(app.filter.is_empty());
    press(&mut app, "/omar");
    let iids: Vec<u64> = app.queue_rows().iter().filter_map(|r| if let QueueRow::Mr(m) = r { Some(m.number) } else { None }).collect();
    assert_eq!(iids, [42, 35], "author matches too");
    app.handle_key(code(KeyCode::Esc));
    assert!(app.filter.is_empty() && !app.filtering);
}

#[test]
fn badges_follow_the_spec_order() {
    let mut app = with_queue();
    let sections = app.sections.clone().unwrap();
    let by_iid = |iid: u64| {
        [&sections.to_review, &sections.mine, &sections.watching, &sections.done]
            .into_iter()
            .flatten()
            .find(|m| m.number == iid)
            .unwrap()
            .clone()
    };
    assert_eq!(app.badge(&by_iid(40)), Some(Badge::Failed), "conflicts beat approved");
    assert_eq!(app.badge(&by_iid(35)), Some(Badge::Running));
    assert_eq!(app.badge(&by_iid(42)), None);
    app.opened.insert(mr_key(), "2026-09-21T00:00:00Z".parse().unwrap());
    assert_eq!(app.badge(&by_iid(42)), Some(Badge::Activity), "updated after it was last opened");
    app.opened.insert(mr_key(), today());
    assert_eq!(app.badge(&by_iid(42)), None);
}

#[test]
fn r_refreshes_once_and_o_y_take_the_mr_url() {
    let mut app = with_queue();
    assert_eq!(press(&mut app, "r"), vec![Action::LoadQueue { scope: None, from_cache: false }]);
    assert_eq!(press(&mut app, "r"), vec![], "not while one is in flight");
    app.apply(Incoming::Queue { scope: None, sections: sections(), opened: HashMap::new(), cached: false });
    let url = "https://gitlab.com/acme/widgets/-/merge_requests/42".to_owned();
    assert_eq!(press(&mut app, "o"), vec![Action::OpenUrl(url.clone())]);
    assert_eq!(press(&mut app, "y"), vec![Action::Yank(url)]);
}

#[test]
fn enter_opens_the_mr_and_the_review_arrives() {
    let app = with_review();
    assert_eq!(app.opening, None);
    assert_eq!(app.focus, Focus::Review);
    let open = app.open.as_ref().unwrap();
    assert_eq!(open.key, mr_key());
    assert_eq!(open.row(), Some(&Row::File { index: 0, open: true }), "the cursor starts on the first file");
    assert_eq!(app.opened.get(&mr_key()), Some(&today()));
    assert!(app.poll.mr_due.is_some() && app.poll.discussions_due.is_some());
}

#[test]
fn a_cached_review_paints_first_and_the_fresh_one_clears_the_age() {
    let mut app = with_queue();
    app.handle_key(code(KeyCode::Enter));
    app.apply(Incoming::Review { key: mr_key(), review: Box::new(review()), cached: Some(Duration::from_secs(120)) });
    assert!(app.opening.is_some(), "still fetching");
    assert_eq!(app.open.as_ref().unwrap().staleness(app.now), Some(Duration::from_secs(120)));
    app.apply(Incoming::Failed { what: Failure::Open, message: "offline".into() });
    assert!(app.offline.is_some() && app.open.is_some(), "the cached view stays");
    app.apply(Incoming::Review { key: mr_key(), review: Box::new(review()), cached: None });
    assert_eq!(app.open.as_ref().unwrap().staleness(app.now), None);
    assert_eq!(app.offline, None);
}

#[test]
fn a_review_for_another_mr_is_ignored() {
    let mut app = with_review();
    app.apply(Incoming::Review { key: MrKey::new("acme/widgets", 99), review: Box::new(review()), cached: None });
    assert_eq!(app.open.as_ref().unwrap().key, mr_key());
}

#[test]
fn tab_and_brackets_jump_between_files_hunks_and_threads() {
    let mut app = with_review();
    press(&mut app, "]c");
    assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::Hunk { index: 0, .. })));
    press(&mut app, "]c");
    assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::Hunk { index: 1, .. })));
    press(&mut app, "]n");
    assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::Outdated { .. })), "wraps to the outdated block");
    press(&mut app, "]n");
    assert_eq!(app.open.as_ref().unwrap().row(), Some(&Row::Thread { id: "c0ffee00c0ffee00".into() }));
    app.handle_key(code(KeyCode::Tab));
    assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::File { index: 1, .. })));
    app.handle_key(code(KeyCode::BackTab));
    assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::File { index: 0, .. })));
}

#[test]
fn bracket_f_finds_files_with_unresolved_threads() {
    let mut app = with_review();
    press(&mut app, "G");
    press(&mut app, "]f");
    assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::File { index: 0, .. })));
    press(&mut app, "]f");
    assert!(app.live_toast().is_some(), "only one such file: nothing to jump to");
}

#[test]
fn folds_save_state_and_keep_the_cursor_on_the_same_place() {
    let mut app = with_review();
    let actions = press(&mut app, "za");
    assert!(matches!(actions.as_slice(), [Action::SaveState { key, .. }] if *key == mr_key()));
    let open = app.open.as_ref().unwrap();
    assert_eq!(open.row(), Some(&Row::File { index: 0, open: false }));
    assert_eq!(open.rows.len(), 5, "header, gap, file, gap, file");
    assert_eq!(press(&mut app, "zc"), vec![], "already closed");
    press(&mut app, "zo");
    assert!(app.open.as_ref().unwrap().rows.len() > 5);
    press(&mut app, "zM");
    assert_eq!(app.open.as_ref().unwrap().rows.len(), 5);
    press(&mut app, "zR");
    assert!(app.open.as_ref().unwrap().review.fold.file_is_open("Cargo.lock"));
}

#[test]
fn enter_toggles_a_hunk_and_opens_a_thread() {
    let mut app = with_review();
    press(&mut app, "]c");
    app.handle_key(code(KeyCode::Enter));
    assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::Hunk { open: false, .. })));
    press(&mut app, "]n");
    app.handle_key(code(KeyCode::Enter));
    assert_eq!(app.focus, Focus::Side);
    assert_eq!(app.open.as_ref().unwrap().thread.as_deref(), Some("9f2c0aa1d4e5b6c7"), "the outdated block opens its first thread");
    assert_eq!(press(&mut app, "u"), vec![], "no link in that thread");
    assert!(app.live_toast().is_some());
    app.handle_key(code(KeyCode::Esc));
    assert_eq!(app.focus, Focus::Review);
    assert_eq!(app.open.as_ref().unwrap().thread, None);
}

#[test]
fn h_l_and_esc_move_the_focus() {
    let mut app = with_review();
    press(&mut app, "h");
    assert_eq!(app.focus, Focus::Queue);
    press(&mut app, "l");
    assert_eq!(app.focus, Focus::Review);
    press(&mut app, "l");
    assert_eq!(app.focus, Focus::Review, "no thread open");
    app.handle_key(code(KeyCode::Esc));
    assert_eq!(app.focus, Focus::Queue);
}

#[test]
fn o_and_y_on_a_line_use_the_line_url() {
    let mut app = with_review();
    press(&mut app, "]cj");
    let url = match press(&mut app, "y").as_slice() {
        [Action::Yank(url)] => url.clone(),
        other => panic!("{other:?}"),
    };
    let digest = sha1_smol::Sha1::from("src/pay/charge.rs".as_bytes()).digest().to_string();
    assert_eq!(url, format!("https://gitlab.com/acme/widgets/-/merge_requests/42/diffs#{digest}_12_12"));
}

#[test]
fn fresh_discussions_replace_the_threads_and_reschedule() {
    let mut app = with_review();
    app.poll.discussions_due = None;
    app.apply(Incoming::Discussions {
        key: mr_key(),
        discussions: vec![fixture::discussion(include_str!("../../forge/gitlab/fixtures/diff_note.json"))],
    });
    assert_eq!(app.open.as_ref().unwrap().review.threads.len(), 1);
    assert!(app.poll.discussions_due.is_some());
}

#[test]
fn a_fresh_review_keeps_the_folds_of_unchanged_files() {
    let mut app = with_review();
    press(&mut app, "za");
    app.apply(Incoming::Review { key: mr_key(), review: Box::new(review()), cached: None });
    assert!(!app.open.as_ref().unwrap().review.fold.file_is_open("src/pay/charge.rs"));
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
        actions.contains(&Action::LoadQueue { scope: None, from_cache: false }) && actions.contains(&Action::RefreshMr(mr_key())),
        "{actions:?}"
    );
    app.apply(Incoming::Failed { what: Failure::Poll, message: "offline".into() });
    assert!(app.offline.is_some());
    app.now += Duration::from_secs(120);
    assert_eq!(app.tick(), vec![], "backed off for five minutes");
    app.now += Duration::from_secs(200);
    assert!(!app.tick().is_empty());
}

#[test]
fn queue_failures_toast_and_stop_the_spinner() {
    let mut app = app();
    app.apply(Incoming::Failed { what: Failure::Queue, message: "HTTP 401".into() });
    assert!(!app.queue_loading);
    let toast = app.live_toast().unwrap();
    assert!(toast.danger && toast.text.contains("r to retry"));
    app.now += Duration::from_secs(5);
    assert!(app.live_toast().is_none(), "toasts age out");
}

#[test]
fn help_and_quit() {
    let mut app = app();
    press(&mut app, "?");
    assert!(app.help);
    press(&mut app, "j");
    assert!(!app.help);
    app.handle_key(ctrl('c'));
    assert!(app.should_quit);
}

#[test]
fn snapshot_queue_loading_and_empty() {
    let mut app = app();
    app.queue_loading = false;
    app.now = app.started;
    insta::assert_snapshot!("queue_loading", render(&mut app, 100, 14));
    app.apply(Incoming::Queue { scope: None, sections: Sections::default(), opened: HashMap::new(), cached: false });
    insta::assert_snapshot!("queue_empty", render(&mut app, 100, 14));
}

#[test]
fn snapshot_queue_loaded() {
    let mut app = with_queue();
    app.opened.insert(MrKey::new("acme/widgets", 41), "2026-09-01T00:00:00Z".parse().unwrap());
    app.now = app.started;
    insta::assert_snapshot!("queue_loaded", render(&mut app, 100, 16));
}

#[test]
fn snapshot_review_open() {
    let mut app = with_review();
    press(&mut app, "]cj");
    insta::assert_snapshot!("review_open", render(&mut app, 120, 24));
}

#[test]
fn snapshot_thread_open() {
    let mut app = with_review();
    press(&mut app, "]n]n");
    app.handle_key(code(KeyCode::Enter));
    insta::assert_snapshot!("thread_open", render(&mut app, 120, 24));
}

#[test]
fn snapshot_help() {
    let mut app = with_queue();
    press(&mut app, "?");
    insta::assert_snapshot!("help", render(&mut app, 100, 40));
}

#[test]
fn snapshot_offline_and_filter() {
    let mut app = with_review();
    app.apply(Incoming::Failed { what: Failure::Poll, message: "offline".into() });
    app.open.as_mut().unwrap().cached = Some((app.now.checked_sub(Duration::from_secs(60)).unwrap(), Duration::from_secs(120)));
    press(&mut app, "h/pay");
    insta::assert_snapshot!("offline_filter", render(&mut app, 100, 12));
}

fn on_line(app: &mut App) {
    press(app, "]cj");
    assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::Line { .. })));
}

fn type_text(app: &mut App, text: &str) -> Vec<Action> {
    press(app, text);
    app.handle_key(code(KeyCode::Enter))
}

fn draft_rows(app: &App) -> Vec<usize> {
    app.open.as_ref().unwrap().rows.iter().filter_map(|r| if let Row::Draft { index } = r { Some(*index) } else { None }).collect()
}

fn with_saved_draft() -> App {
    let mut app = with_review();
    on_line(&mut app);
    press(&mut app, "c");
    type_text(&mut app, "nit");
    app.apply(Incoming::DraftSaved { key: mr_key(), index: 0, id: 9 });
    app
}

#[test]
fn c_on_a_line_opens_the_input_and_enter_makes_a_draft() {
    let mut app = with_review();
    assert_eq!(press(&mut app, "c"), vec![], "the cursor is on a file row");
    assert!(app.input.is_none() && app.live_toast().is_some());
    on_line(&mut app);
    press(&mut app, "c");
    assert_eq!(app.input_label(), "comment charge.rs:12");
    let actions = type_text(&mut app, "nit: rename");
    let [Action::SaveDraft { key, index: 0, draft }] = actions.as_slice() else { panic!("{actions:?}") };
    assert_eq!(*key, mr_key());
    assert_eq!(draft.body, "nit: rename");
    assert_eq!(draft.position.as_ref().and_then(|p| p.line.new), Some(12));
    assert_eq!(draft.id, None);
    assert!(app.input.is_none());
    let open = app.open.as_ref().unwrap();
    assert_eq!(draft_rows(&app), [0]);
    assert!(matches!(open.rows[open.selected + 1], Row::Draft { index: 0 }), "the draft row follows the line");
    assert_eq!(app.unsaved_drafts(), 1);
    app.apply(Incoming::DraftSaved { key: mr_key(), index: 0, id: 9 });
    assert_eq!(app.open.as_ref().unwrap().review.drafts[0].id, Some(9));
    assert_eq!(app.unsaved_drafts(), 0);
}

#[test]
fn the_input_row_edits_in_place_and_esc_cancels() {
    let mut app = with_review();
    on_line(&mut app);
    press(&mut app, "cab");
    app.handle_key(code(KeyCode::Left));
    press(&mut app, "x");
    assert_eq!(app.buffer.text(), "axb");
    app.handle_key(ctrl('a'));
    app.handle_key(code(KeyCode::Delete));
    assert_eq!(app.buffer.text(), "xb");
    app.handle_key(code(KeyCode::Esc));
    assert!(app.input.is_none() && app.buffer.text().is_empty());
    press(&mut app, "c");
    assert_eq!(app.handle_key(code(KeyCode::Enter)), vec![], "an empty comment is dropped");
    assert!(app.input.is_none());
}

#[test]
fn v_selects_a_range_for_c_y_and_esc() {
    let mut app = with_review();
    on_line(&mut app);
    press(&mut app, "Vj");
    let actions = press(&mut app, "y");
    let [Action::Yank(text)] = actions.as_slice() else { panic!("{actions:?}") };
    assert_eq!(text, " pub async fn charge(card: &Card, amount: Money) -> Result<Receipt> {\n-    let client = Client::new();");
    assert_eq!(app.open.as_ref().unwrap().select_from, None, "copying drops the selection");
    press(&mut app, "k");
    press(&mut app, "Vjjj");
    let open = app.open.as_ref().unwrap();
    assert_eq!(open.selection().count(), 4, "three lines and the thread row between them");
    assert!(open.is_selected(open.selected - 1));
    press(&mut app, "c");
    let actions = type_text(&mut app, "fold these");
    let [Action::SaveDraft { draft, .. }] = actions.as_slice() else { panic!("{actions:?}") };
    let position = draft.position.as_ref().unwrap();
    assert_eq!(position.line.new, Some(13));
    let start = position.start.expect("a range");
    assert_eq!((start.old, position.line.new), (Some(12), Some(13)));
    assert_eq!(app.open.as_ref().unwrap().select_from, None, "sending drops the selection");
    press(&mut app, "Vj");
    app.handle_key(code(KeyCode::Esc));
    assert_eq!(app.open.as_ref().unwrap().select_from, None);
    assert_eq!(app.focus, Focus::Review, "esc dropped the selection, nothing else");
}

#[test]
fn r_in_a_thread_replies_as_a_draft_shown_in_the_pane_not_the_diff() {
    let mut app = with_review();
    press(&mut app, "]n");
    app.handle_key(code(KeyCode::Enter));
    press(&mut app, "r");
    assert_eq!(app.input_label(), "reply");
    let actions = type_text(&mut app, "agreed");
    let [Action::SaveDraft { draft, .. }] = actions.as_slice() else { panic!("{actions:?}") };
    assert_eq!(draft.reply_to.as_deref(), Some("c0ffee00c0ffee00"));
    assert!(draft_rows(&app).is_empty());
    assert_eq!(app.focus, Focus::Side);
    assert!(render(&mut app, 120, 24).contains("◇ you · unsaved"));
}

#[test]
fn big_r_flips_resolved_at_once_and_a_refusal_flips_it_back() {
    let mut app = with_review();
    press(&mut app, "]n");
    app.handle_key(code(KeyCode::Enter));
    let id = "c0ffee00c0ffee00".to_owned();
    assert!(app.open.as_ref().unwrap().review.thread(&id).unwrap().resolved);
    let actions = press(&mut app, "R");
    assert_eq!(actions, vec![Action::Resolve { key: mr_key(), thread: id.clone(), resolved: false }]);
    assert!(!app.open.as_ref().unwrap().review.thread(&id).unwrap().resolved);
    app.apply(Incoming::Failed { what: Failure::Resolve { thread: id.clone(), resolved: false }, message: "HTTP 403".into() });
    assert!(app.open.as_ref().unwrap().review.thread(&id).unwrap().resolved, "back to resolved");
    assert!(app.live_toast().unwrap().danger);
    press(&mut app, "R");
    app.apply(Incoming::Resolved { key: mr_key(), thread: id.clone(), resolved: false });
    assert!(!app.open.as_ref().unwrap().review.thread(&id).unwrap().resolved);
}

#[test]
fn enter_edits_a_draft_and_d_deletes_it() {
    let mut app = with_saved_draft();
    press(&mut app, "j");
    assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::Draft { index: 0 })));
    app.handle_key(code(KeyCode::Enter));
    assert_eq!((app.input_label(), app.buffer.text()), ("edit draft".to_owned(), "nit"));
    let actions = type_text(&mut app, " (typo)");
    let [Action::UpdateDraft { key, id: 9, draft }] = actions.as_slice() else { panic!("{actions:?}") };
    assert_eq!(*key, mr_key());
    assert_eq!((draft.body.as_str(), draft.position.is_some()), ("nit (typo)", true), "the position travels with the edit");
    assert_eq!(app.open.as_ref().unwrap().review.drafts[0].body, "nit (typo)");
    assert_eq!(press(&mut app, "d"), vec![Action::DeleteDraft { key: mr_key(), id: 9 }]);
    assert!(draft_rows(&app).is_empty());
    assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::Line { .. })), "the cursor lands on the next row");
}

#[test]
fn an_unsaved_draft_is_posted_again_by_r_and_deleted_without_a_request() {
    let mut app = with_review();
    on_line(&mut app);
    press(&mut app, "c");
    type_text(&mut app, "nit");
    app.apply(Incoming::Failed { what: Failure::Draft { index: 0 }, message: "offline".into() });
    assert!(app.live_toast().unwrap().text.contains("r to retry"));
    let actions = press(&mut app, "r");
    assert!(matches!(actions.as_slice(), [Action::RefreshMr(key), Action::SaveDraft { index: 0, .. }] if *key == mr_key()), "{actions:?}");
    press(&mut app, "j");
    assert_eq!(press(&mut app, "d"), vec![], "GitLab never had it");
    assert_eq!(app.draft_count(), 0);
}

#[test]
fn the_publish_modal_walks_the_drafts_toggles_approve_and_publishes() {
    let mut app = with_saved_draft();
    press(&mut app, "jjc");
    type_text(&mut app, "second");
    app.apply(Incoming::DraftSaved { key: mr_key(), index: 1, id: 10 });
    press(&mut app, "P");
    let publish = app.publish.clone().unwrap();
    assert_eq!((publish.selected, publish.approve, publish.busy), (0, false, false));
    press(&mut app, "jjj");
    assert_eq!(app.publish.as_ref().unwrap().selected, 2, "stops on the publish row");
    press(&mut app, "a");
    assert!(app.publish.as_ref().unwrap().approve);
    let actions = app.handle_key(code(KeyCode::Enter));
    assert_eq!(actions, vec![Action::Publish { key: mr_key(), approve: true, count: 2 }]);
    assert!(app.publish.as_ref().unwrap().busy);
    assert_eq!(press(&mut app, "a"), vec![], "keys wait for the answer");
    app.apply(Incoming::Published { key: mr_key(), approved: true, count: 2 });
    assert_eq!(app.publish, None);
    assert_eq!(app.draft_count(), 0);
    assert!(app.open.as_ref().unwrap().review.mr.approvals.user_has_approved);
    assert_eq!(app.poll.discussions_due, Some(app.now), "threads refresh at once");
    assert_eq!(app.live_toast().unwrap().text, "published 2 comments and approved");
}

#[test]
fn the_publish_modal_edits_deletes_and_survives_a_failure() {
    let mut app = with_saved_draft();
    press(&mut app, "P");
    app.handle_key(code(KeyCode::Enter));
    assert_eq!(app.input_label(), "edit draft");
    assert!(app.publish.is_some(), "the modal stays under the input row");
    type_text(&mut app, "!");
    press(&mut app, "p");
    app.apply(Incoming::Failed { what: Failure::Publish, message: "HTTP 500".into() });
    assert!(!app.publish.as_ref().unwrap().busy);
    assert_eq!(app.open.as_ref().unwrap().review.drafts[0].body, "nit!");
    assert!(app.live_toast().unwrap().text.contains("not published"));
    assert_eq!(press(&mut app, "d"), vec![Action::DeleteDraft { key: mr_key(), id: 9 }]);
    assert_eq!(app.draft_count(), 0);
    app.handle_key(code(KeyCode::Esc));
    assert_eq!(app.publish, None);
    press(&mut app, "P");
    assert_eq!(app.publish, None);
    assert_eq!(app.live_toast().unwrap().text, "no drafts");
}

#[test]
fn publishing_waits_for_unsaved_drafts() {
    let mut app = with_review();
    on_line(&mut app);
    press(&mut app, "c");
    type_text(&mut app, "nit");
    press(&mut app, "P");
    assert_eq!(press(&mut app, "p"), vec![]);
    assert!(app.live_toast().unwrap().danger);
    assert!(!app.publish.as_ref().unwrap().busy);
}

#[test]
fn big_a_approves_then_unapproves() {
    let mut app = with_review();
    assert_eq!(press(&mut app, "A"), vec![Action::Approve { key: mr_key(), approve: true }]);
    app.apply(Incoming::Approved { key: mr_key(), approve: true });
    assert_eq!(app.live_toast().unwrap().text, "approved");
    assert_eq!(press(&mut app, "A"), vec![Action::Approve { key: mr_key(), approve: false }]);
    app.apply(Incoming::Failed { what: Failure::Approve, message: "you cannot approve this MR".into() });
    assert!(app.live_toast().unwrap().danger);
}

#[test]
fn big_e_and_s_open_the_editor_and_what_comes_back_is_a_draft() {
    let mut app = with_review();
    on_line(&mut app);
    let actions = press(&mut app, "E");
    let [Action::Compose { input: Input::Comment { position }, draft }] = actions.as_slice() else { panic!("{actions:?}") };
    assert!(draft.is_empty() && position.line.new == Some(12));
    let actions = press(&mut app, "Vjs");
    let [Action::Compose { draft, .. }] = actions.as_slice() else { panic!("{actions:?}") };
    assert_eq!(
        draft,
        "```suggestion:-0+1\npub async fn charge(card: &Card, amount: Money) -> Result<Receipt> {\n    let client = Client::new();\n```\n"
    );
    let input = Input::Comment { position: position.clone() };
    app.apply(Incoming::Composed { input: input.clone(), text: None });
    assert_eq!(app.take_actions(), vec![]);
    assert_eq!(app.draft_count(), 0);
    app.apply(Incoming::Composed { input, text: Some("from the editor".into()) });
    let actions = app.take_actions();
    assert!(matches!(actions.as_slice(), [Action::SaveDraft { index: 0, .. }]), "{actions:?}");
    assert_eq!(app.open.as_ref().unwrap().review.drafts[0].body, "from the editor");
    press(&mut app, "k");
    assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::Draft { index: 0 })));
    let actions = press(&mut app, "E");
    assert!(matches!(actions.as_slice(), [Action::Compose { input: Input::EditDraft { index: 0 }, draft }] if draft == "from the editor"));
}

#[test]
fn snapshot_review_with_a_draft_and_the_input_row() {
    let mut app = with_saved_draft();
    press(&mut app, "jjVjc");
    press(&mut app, "fold the");
    insta::assert_snapshot!("input_comment", render(&mut app, 120, 24));
}

#[test]
fn snapshot_publish_modal() {
    let mut app = with_saved_draft();
    press(&mut app, "jjc");
    type_text(&mut app, "second one, a bit longer so the modal cuts it with an ellipsis");
    press(&mut app, "Pja");
    app.now = app.started;
    insta::assert_snapshot!("publish_modal", render(&mut app, 120, 24));
}

#[test]
fn snapshot_thread_with_a_draft_reply() {
    let mut app = with_review();
    press(&mut app, "]n");
    app.handle_key(code(KeyCode::Enter));
    press(&mut app, "r");
    type_text(&mut app, "agreed, keys are per card");
    app.apply(Incoming::DraftSaved { key: mr_key(), index: 0, id: 9 });
    insta::assert_snapshot!("thread_draft_reply", render(&mut app, 120, 24));
}

#[test]
fn right_from_the_queue_opens_the_selected_mr_like_enter() {
    let mut app = with_queue();
    assert_eq!(press(&mut app, "l"), vec![Action::Open(mr_key())]);
    assert_eq!(app.focus, Focus::Review);
    app.apply(Incoming::Review { key: mr_key(), review: Box::new(review()), cached: None });
    press(&mut app, "hj");
    let next = app.selected_mr().unwrap().key();
    assert_ne!(next, mr_key());
    assert_eq!(press(&mut app, "l"), vec![Action::Open(next)], "the diff must follow the queue row, never show the previous MR");
    assert_eq!(app.focus, Focus::Review);
}

fn cells(app: &mut App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| ui::draw(f, app)).unwrap();
    terminal.backend().buffer().clone()
}

/// The cell holding the first character of the removed text, and the last cell of that row
/// inside the pane (the column before the border is the pane's own padding).
fn removed_line_cells(app: &mut App) -> (ratatui::buffer::Cell, ratatui::buffer::Cell) {
    let buffer = cells(app, 120, 30);
    let text = "let client = Client::new();";
    let (x, y) = (0..30)
        .find_map(|y| {
            let row: String = (0..120).map(|x| buffer[(x, y)].symbol().to_owned()).collect();
            row.find(text).map(|i| (i as u16, y))
        })
        .expect("the removed line is on screen");
    let border = (x..120).find(|&x| buffer[(x, y)].symbol() == "│").expect("a pane border closes the row");
    (buffer[(x, y)].clone(), buffer[(border - 2, y)].clone())
}

#[test]
fn removed_lines_read_on_every_theme_and_fill_the_row_where_the_theme_knows_its_ground() {
    let mut app = with_review();
    press(&mut app, "]cj");
    let (text, edge) = removed_line_cells(&mut app);
    assert_eq!((text.fg, text.bg), (Color::Red, Color::Reset), "the default theme paints the text, never a fill it cannot see");
    assert_eq!(edge.bg, Color::Reset);
    app.theme = Theme::named("tokyonight").unwrap();
    let (text, edge) = removed_line_cells(&mut app);
    assert_eq!((text.fg, text.bg), (Color::Reset, app.theme.removed_fill.unwrap()), "the terminal's own text on the theme's fill");
    assert_eq!(edge.bg, app.theme.removed_fill.unwrap(), "the fill reaches the edge of the pane");
}

fn scoped_app() -> App {
    let mut app = App::new(Settings { project: Some("acme/widgets".into()), ..settings() });
    app.today = today();
    app
}

fn scoped_sections() -> Sections {
    fixture::queue_in(include_str!("../../forge/gitlab/fixtures/queue_scoped.json"), "acme/widgets").sections(&[])
}

fn queue_answer(scope: Option<&str>, sections: Sections, cached: bool) -> Incoming {
    Incoming::Queue { scope: scope.map(str::to_owned), sections, opened: HashMap::new(), cached }
}

#[test]
fn in_a_checkout_the_queue_starts_on_its_project_and_star_widens_it() {
    let mut app = scoped_app();
    let scope = Some("acme/widgets".to_owned());
    assert_eq!(app.start(), vec![Action::LoadQueue { scope: scope.clone(), from_cache: true }]);
    app.apply(queue_answer(Some("acme/widgets"), scoped_sections(), false));
    assert!(app.queue_rows().iter().any(|r| matches!(r, QueueRow::Section { name: "OPEN", count: 2, .. })));
    assert_eq!(press(&mut app, "*"), vec![Action::LoadQueue { scope: None, from_cache: true }]);
    assert_eq!(app.sections, None, "the project's list is gone before the wider one paints");
    assert_eq!(press(&mut app, "*"), vec![Action::LoadQueue { scope, from_cache: true }]);
}

#[test]
fn an_answer_for_the_other_scope_is_dropped_and_a_cached_one_never_hides_a_fresh_one() {
    let mut app = scoped_app();
    app.apply(queue_answer(None, sections(), false));
    assert_eq!(app.sections, None, "the answer to a scope we left");
    app.apply(queue_answer(Some("acme/widgets"), scoped_sections(), true));
    assert!(app.sections.is_some() && app.queue_loading, "the cache paints while the fetch runs");
    app.apply(queue_answer(Some("acme/widgets"), Sections::default(), false));
    app.apply(queue_answer(Some("acme/widgets"), scoped_sections(), true));
    assert_eq!(app.sections, Some(Sections::default()), "a late cache answer never replaces a fresh one");
    assert!(!app.queue_loading);
}

#[test]
fn star_outside_a_checkout_says_why_it_does_nothing() {
    let mut app = with_queue();
    assert_eq!(press(&mut app, "*"), vec![]);
    assert!(app.live_toast().unwrap().text.contains("checkout"));
}

#[test]
fn i_opens_the_description_from_the_queue_and_the_review_and_closes_on_esc() {
    let mut app = scoped_app();
    app.apply(queue_answer(Some("acme/widgets"), scoped_sections(), false));
    press(&mut app, "Gk");
    press(&mut app, "i");
    let brief = app.brief.clone().unwrap();
    assert_eq!((brief.number, brief.description.as_str()), (50, "Adds the refund flow."));
    assert_eq!(press(&mut app, "o"), vec![Action::OpenUrl(brief.web_url)]);
    app.handle_key(code(KeyCode::Esc));
    assert_eq!(app.brief, None);
    let mut app = with_review();
    press(&mut app, "i");
    assert_eq!(app.brief.as_ref().map(|b| b.number), Some(42));
    press(&mut app, "q");
    assert!(app.brief.is_none() && !app.should_quit, "q closes the modal, it does not quit");
}

#[test]
fn the_description_scrolls_and_the_view_stops_it_at_the_last_page() {
    let mut app = with_review();
    let long: String = (1..=80).map(|n| format!("line {n}\n")).collect();
    app.open = app.open.clone().map(|o| {
        let mut review = o.review.clone();
        review.mr.description = long.clone();
        o.with_review(review)
    });
    press(&mut app, "ijjj");
    assert_eq!(app.brief.as_ref().unwrap().scroll, 3);
    press(&mut app, "kg");
    assert_eq!(app.brief.as_ref().unwrap().scroll, 0);
    press(&mut app, "G");
    render(&mut app, 100, 30);
    let bottom = app.brief.as_ref().unwrap().scroll;
    assert!(bottom > 0 && bottom < 80, "clamped to the last page, got {bottom}");
    press(&mut app, "k");
    assert_eq!(app.brief.as_ref().unwrap().scroll, bottom - 1, "one step up from the bottom, not from usize::MAX");
}

#[test]
fn snapshot_description_modal() {
    let mut app = with_review();
    app.open = app.open.clone().map(|o| {
        let mut review = o.review.clone();
        review.mr.description = "## Why\n\nCards were charged twice.\n\n- retry with `idempotency_key`\n- log the attempt".into();
        o.with_review(review)
    });
    press(&mut app, "i");
    insta::assert_snapshot!("description_modal", render(&mut app, 100, 24));
}

#[test]
fn snapshot_queue_scoped_with_open() {
    let mut app = scoped_app();
    app.apply(queue_answer(Some("acme/widgets"), scoped_sections(), false));
    insta::assert_snapshot!("queue_scoped", render(&mut app, 120, 20));
}

#[test]
fn every_recorded_link_sits_on_the_text_it_names_and_a_modal_hides_them() {
    let mut app = with_review();
    let buffer = cells(&mut app, 120, 24);
    let texts: Vec<&str> = app.links.iter().map(|l| l.text.as_str()).collect();
    assert!(texts.contains(&"!42") && texts.contains(&"acme/widgets!42"), "{texts:?}");
    for link in &app.links {
        let drawn: String = (0..link.text.chars().count() as u16).map(|i| buffer[(link.x + i, link.y)].symbol().to_owned()).collect();
        assert_eq!(drawn, link.text, "{link:?}");
    }
    press(&mut app, "i");
    cells(&mut app, 120, 24);
    assert!(app.links.is_empty(), "no link may print over a modal");
}
