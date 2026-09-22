use super::*;
use crate::api::types::from_fixture;
use crate::api::{DiffFile, Discussion, Mr, Queue};
use crate::review::Row;
use crate::tui::theme::Theme;
use crate::tui::ui;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::json;
use std::time::Duration;

const KEY: MrKey = (7, 42);

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
    Settings { theme: Theme::default(), host: "gitlab.com".into(), me: "nina".into(), fold_globs: vec![], watch_labels: vec![] }
}

fn app() -> App {
    let mut app = App::new(settings());
    app.today = today();
    app
}

fn sections() -> Sections {
    Queue::from_json(include_str!("../../api/fixtures/queue.json")).unwrap().sections(&[])
}

fn with_queue() -> App {
    let mut app = app();
    app.apply(Incoming::Queue { sections: sections(), opened: HashMap::new() });
    app
}

fn mr() -> Mr {
    from_fixture(
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
        from_fixture(include_str!("../../api/fixtures/discussions.json")),
        from_fixture(include_str!("../../api/fixtures/diff_note.json")),
        from_fixture(include_str!("../../review/fixtures/old_side_note.json")),
    ]
}

fn review() -> Review {
    Review::new(mr(), diffs(), discussions(), &["*.lock".into()])
}

fn with_review() -> App {
    let mut app = with_queue();
    app.queue_move(0);
    assert_eq!(press(&mut app, "\r"), vec![]);
    let actions = app.handle_key(code(KeyCode::Enter));
    assert_eq!(actions, vec![Action::Open(KEY)]);
    app.apply(Incoming::Review { key: KEY, review: Box::new(review()), cached: None });
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
    assert_eq!(app().start(), vec![Action::LoadQueue]);
}

#[test]
fn the_queue_lands_in_sections_and_the_cursor_on_the_first_mr() {
    let app = with_queue();
    assert!(!app.queue_loading);
    assert!(app.poll.queue_due.is_some());
    assert_eq!(app.selected_mr().map(|m| m.iid), Some(42));
    let rows = app.queue_rows();
    assert!(matches!(rows[0], QueueRow::Section { name: "TO REVIEW", count: 1, open: true }));
    assert!(rows.iter().any(|r| matches!(r, QueueRow::Section { name: "DONE", open: false, .. })));
}

#[test]
fn j_and_k_skip_section_headers_and_stop_at_the_ends() {
    let mut app = with_queue();
    press(&mut app, "j");
    assert_eq!(app.selected_mr().map(|m| m.iid), Some(41), "MINE header is skipped");
    press(&mut app, "kkk");
    assert_eq!(app.selected_mr().map(|m| m.iid), Some(42));
    press(&mut app, "G");
    assert_eq!(app.selected_mr().map(|m| m.iid), Some(35));
    press(&mut app, "g");
    assert_eq!(app.selected_mr().map(|m| m.iid), Some(42));
}

#[test]
fn zo_in_the_queue_shows_the_done_section() {
    let mut app = with_queue();
    press(&mut app, "zo");
    assert!(app.done_open);
    press(&mut app, "G");
    assert_eq!(app.selected_mr().map(|m| m.iid), Some(40));
    press(&mut app, "zc");
    assert_eq!(app.selected_mr().map(|m| m.iid), Some(35), "the cursor settles on a visible row");
}

#[test]
fn the_filter_narrows_live_and_esc_clears_it() {
    let mut app = with_queue();
    press(&mut app, "/runner");
    assert!(app.filtering);
    assert_eq!(app.selected_mr().map(|m| m.iid), Some(35));
    app.handle_key(code(KeyCode::Enter));
    assert!(!app.filtering && app.filter == "runner");
    app.handle_key(code(KeyCode::Esc));
    assert!(app.filter.is_empty());
    press(&mut app, "/omar");
    let iids: Vec<u64> = app.queue_rows().iter().filter_map(|r| if let QueueRow::Mr(m) = r { Some(m.iid) } else { None }).collect();
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
            .find(|m| m.iid == iid)
            .unwrap()
            .clone()
    };
    assert_eq!(app.badge(&by_iid(40)), Some(Badge::Failed), "conflicts beat approved");
    assert_eq!(app.badge(&by_iid(35)), Some(Badge::Running));
    assert_eq!(app.badge(&by_iid(42)), None);
    app.opened.insert(KEY, "2026-09-21T00:00:00Z".parse().unwrap());
    assert_eq!(app.badge(&by_iid(42)), Some(Badge::Activity), "updated after it was last opened");
    app.opened.insert(KEY, today());
    assert_eq!(app.badge(&by_iid(42)), None);
}

#[test]
fn r_refreshes_once_and_o_y_take_the_mr_url() {
    let mut app = with_queue();
    assert_eq!(press(&mut app, "r"), vec![Action::LoadQueue]);
    assert_eq!(press(&mut app, "r"), vec![], "not while one is in flight");
    app.apply(Incoming::Queue { sections: sections(), opened: HashMap::new() });
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
    assert_eq!(open.key, KEY);
    assert_eq!(open.row(), Some(&Row::File { index: 0, open: true }), "the cursor starts on the first file");
    assert_eq!(app.opened.get(&KEY), Some(&today()));
    assert!(app.poll.mr_due.is_some() && app.poll.discussions_due.is_some());
}

#[test]
fn a_cached_review_paints_first_and_the_fresh_one_clears_the_age() {
    let mut app = with_queue();
    app.handle_key(code(KeyCode::Enter));
    app.apply(Incoming::Review { key: KEY, review: Box::new(review()), cached: Some(Duration::from_secs(120)) });
    assert!(app.opening.is_some(), "still fetching");
    assert_eq!(app.open.as_ref().unwrap().staleness(app.now), Some(Duration::from_secs(120)));
    app.apply(Incoming::Failed { what: Failure::Open, message: "offline".into() });
    assert!(app.offline.is_some() && app.open.is_some(), "the cached view stays");
    app.apply(Incoming::Review { key: KEY, review: Box::new(review()), cached: None });
    assert_eq!(app.open.as_ref().unwrap().staleness(app.now), None);
    assert_eq!(app.offline, None);
}

#[test]
fn a_review_for_another_mr_is_ignored() {
    let mut app = with_review();
    app.apply(Incoming::Review { key: (7, 99), review: Box::new(review()), cached: None });
    assert_eq!(app.open.as_ref().unwrap().key, KEY);
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
    assert!(matches!(actions.as_slice(), [Action::SaveState { key: KEY, .. }]));
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
    app.apply(Incoming::Discussions { key: KEY, discussions: vec![from_fixture(include_str!("../../api/fixtures/diff_note.json"))] });
    assert_eq!(app.open.as_ref().unwrap().review.threads.len(), 1);
    assert!(app.poll.discussions_due.is_some());
}

#[test]
fn a_fresh_review_keeps_the_folds_of_unchanged_files() {
    let mut app = with_review();
    press(&mut app, "za");
    app.apply(Incoming::Review { key: KEY, review: Box::new(review()), cached: None });
    assert!(!app.open.as_ref().unwrap().review.fold.file_is_open("src/pay/charge.rs"));
}

#[test]
fn polling_fires_once_per_due_date_and_backs_off_on_failure() {
    let mut app = with_review();
    assert_eq!(app.tick(), vec![]);
    app.now += Duration::from_secs(31);
    assert_eq!(app.tick(), vec![Action::RefreshDiscussions(KEY)]);
    assert_eq!(app.tick(), vec![]);
    app.now += Duration::from_secs(30);
    let actions = app.tick();
    assert!(actions.contains(&Action::LoadQueue) && actions.contains(&Action::RefreshMr(KEY)), "{actions:?}");
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
    app.apply(Incoming::Queue { sections: Sections::default(), opened: HashMap::new() });
    insta::assert_snapshot!("queue_empty", render(&mut app, 100, 14));
}

#[test]
fn snapshot_queue_loaded() {
    let mut app = with_queue();
    app.opened.insert((7, 41), "2026-09-01T00:00:00Z".parse().unwrap());
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
    insta::assert_snapshot!("help", render(&mut app, 100, 28));
}

#[test]
fn snapshot_offline_and_filter() {
    let mut app = with_review();
    app.apply(Incoming::Failed { what: Failure::Poll, message: "offline".into() });
    app.open.as_mut().unwrap().cached = Some((app.now - Duration::from_secs(60), Duration::from_secs(120)));
    press(&mut app, "h/pay");
    insta::assert_snapshot!("offline_filter", render(&mut app, 100, 12));
}
