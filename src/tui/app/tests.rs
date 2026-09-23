#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::forge::gitlab::fixture;
use crate::forge::{DiffFile, Discussion, Kind, Mr};
use crate::review::{Place, Row};
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
    Settings {
        theme: Theme::default(),
        host: "gitlab.com".into(),
        kind: Kind::GitLab,
        me: "nina".into(),
        project: None,
        ground: None,
        triage: false,
        ask: None,
    }
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
    app.apply(Incoming::Queue { scope: None, me: "nina".into(), sections: sections(), opened: HashMap::new(), cached: false });
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
    assert!(app.selected_mr().is_none(), "the last row is the folded Done header");
    press(&mut app, "k");
    assert_eq!(app.selected_mr().map(|m| m.number), Some(35));
    press(&mut app, "g");
    assert_eq!(app.selected_mr().map(|m| m.number), Some(42));
}

#[test]
fn zo_and_zc_fold_the_section_under_the_cursor() {
    let mut app = with_queue();
    press(&mut app, "G");
    assert!(app.selected_mr().is_none(), "the folded Done header takes the cursor");
    press(&mut app, "zo");
    assert!(!app.closed_sections.contains("DONE"));
    press(&mut app, "G");
    assert_eq!(app.selected_mr().map(|m| m.number), Some(40));
    press(&mut app, "zc");
    assert!(app.closed_sections.contains("DONE"));
    assert!(matches!(app.queue_rows()[app.queue_selected], QueueRow::Section { name: "DONE", .. }), "the cursor stays on its header");
    press(&mut app, "g");
    press(&mut app, "zc");
    assert!(app.closed_sections.contains("TO REVIEW"), "any section folds, not only Done");
    assert!(matches!(app.queue_rows()[app.queue_selected], QueueRow::Section { name: "TO REVIEW", .. }));
    app.handle_key(code(KeyCode::Enter));
    assert!(!app.closed_sections.contains("TO REVIEW"), "enter on a folded header opens it");
}

#[test]
fn zh_folds_the_review_header_to_one_row() {
    let mut app = with_review();
    press(&mut app, "zh");
    assert!(app.header_folded);
    let screen = render(&mut app, 120, 24);
    assert!(screen.contains("▸ "), "{screen}");
    press(&mut app, "zh");
    assert!(!app.header_folded);
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
    app.apply(Incoming::Queue { scope: None, me: "nina".into(), sections: sections(), opened: HashMap::new(), cached: false });
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
    assert_eq!(app.open.as_ref().unwrap().row(), Some(&Row::Header), "wraps to the thread on the MR");
    press(&mut app, "]n");
    assert_eq!(app.open.as_ref().unwrap().row(), Some(&Row::Line { file: 0, hunk: 0, index: 1 }), "then the marked removed line");
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
fn enter_toggles_a_hunk_and_opens_the_pane_on_a_marked_line() {
    let mut app = with_review();
    press(&mut app, "]c");
    app.handle_key(code(KeyCode::Enter));
    assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::Hunk { open: false, .. })));
    app.handle_key(code(KeyCode::Enter));
    press(&mut app, "]n");
    app.handle_key(code(KeyCode::Enter));
    assert_eq!(app.focus, Focus::Side);
    let open = app.open.as_ref().unwrap();
    assert_eq!(open.pane.as_ref().map(|p| &p.place), Some(&Place::Line { file: 0, new: None, old: Some(13) }));
    assert_eq!(open.focused_thread().as_deref(), Some("c0ffee00c0ffee00"));
    assert_eq!(press(&mut app, "u"), vec![], "no link in that thread");
    assert!(app.live_toast().is_some());
    app.handle_key(code(KeyCode::Esc));
    assert_eq!(app.focus, Focus::Review);
    assert_eq!(app.open.as_ref().unwrap().pane, None);
}

#[test]
fn l_on_a_file_opens_its_outdated_threads_and_the_header_its_mr_threads() {
    let mut app = with_review();
    assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::File { index: 0, .. })));
    press(&mut app, "l");
    let open = app.open.as_ref().unwrap();
    assert_eq!(open.pane.as_ref().map(|p| &p.place), Some(&Place::Outdated { file: 0 }));
    assert_eq!(open.focused_thread().as_deref(), Some("9f2c0aa1d4e5b6c7"));
    press(&mut app, "x");
    assert_eq!(app.open.as_ref().unwrap().pane, None);
    press(&mut app, "k");
    assert_eq!(app.open.as_ref().unwrap().row(), Some(&Row::Header), "the header holds the thread on the MR");
    app.handle_key(code(KeyCode::Enter));
    assert_eq!(app.open.as_ref().unwrap().pane.as_ref().map(|p| &p.place), Some(&Place::Mr));
}

#[test]
fn the_pane_follows_the_cursor_onto_marked_lines_only() {
    let mut app = with_review();
    press(&mut app, "]n");
    press(&mut app, "l");
    press(&mut app, "h");
    press(&mut app, "k");
    let open = app.open.as_ref().unwrap();
    assert_eq!(
        open.pane.as_ref().map(|p| &p.place),
        Some(&Place::Line { file: 0, new: None, old: Some(13) }),
        "an unmarked line keeps the thread"
    );
    assert!(render(&mut app, 140, 24).contains("↑ line -13"), "and says where it belongs");
    press(&mut app, "x");
    assert_eq!(app.open.as_ref().unwrap().pane, None);
}

#[test]
fn h_l_and_esc_move_the_focus() {
    let mut app = with_review();
    press(&mut app, "h");
    assert_eq!(app.focus, Focus::Queue);
    press(&mut app, "l");
    assert_eq!(app.focus, Focus::Review);
    press(&mut app, "l");
    assert_eq!(app.focus, Focus::Side, "the file row opens its outdated threads");
    app.handle_key(code(KeyCode::Esc));
    assert_eq!(app.focus, Focus::Review, "esc closes the pane first");
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
    assert_eq!(app.help, Some(0));
    press(&mut app, "jjk");
    assert_eq!(app.help, Some(1), "moving keys scroll the list");
    press(&mut app, "G");
    assert_eq!(app.help, Some(ui::HELP.len() - 1));
    press(&mut app, "x");
    assert_eq!(app.help, None, "any other key closes it");
    app.handle_key(ctrl('c'));
    assert!(app.should_quit);
}

#[test]
fn snapshot_queue_loading_and_empty() {
    let mut app = app();
    app.queue_loading = false;
    app.now = app.started;
    insta::assert_snapshot!("queue_loading", render(&mut app, 100, 14));
    app.apply(Incoming::Queue { scope: None, me: "nina".into(), sections: Sections::default(), opened: HashMap::new(), cached: false });
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
    press(&mut app, "]n");
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
fn snapshot_help_scrolls_on_a_small_terminal() {
    let mut app = with_queue();
    press(&mut app, "?jjj");
    insta::assert_snapshot!("help_small", render(&mut app, 80, 24));
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

/// The anchor column of the row under the cursor.
fn marker_here(app: &App) -> Option<crate::review::Marker> {
    let open = app.open.as_ref().unwrap();
    open.review.marker_of(&open.review.markers(), open.row()?)
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
    assert_eq!(app.input_label(), "new thread · charge.rs:12");
    let actions = type_text(&mut app, "nit: rename");
    let [Action::SaveDraft { key, index: 0, draft }] = actions.as_slice() else { panic!("{actions:?}") };
    assert_eq!(*key, mr_key());
    assert_eq!(draft.body, "nit: rename");
    assert_eq!(draft.position.as_ref().and_then(|p| p.line.new), Some(12));
    assert_eq!(draft.id, None);
    assert!(app.input.is_none());
    let marker = marker_here(&app).expect("the line is marked");
    assert_eq!((marker.mark, marker.unsaved), (crate::review::Mark::Draft, true), "an unsaved draft of mine, no row inserted");
    assert_eq!(app.unsaved_drafts(), 1);
    app.apply(Incoming::DraftSaved { key: mr_key(), index: 0, id: 9 });
    assert_eq!(app.open.as_ref().unwrap().review.drafts[0].id, Some(9));
    assert_eq!(app.unsaved_drafts(), 0);
}

#[test]
fn the_compose_box_edits_in_place_keeps_its_text_on_esc_and_takes_newlines() {
    let mut app = with_review();
    on_line(&mut app);
    press(&mut app, "cab");
    assert_eq!(app.focus, Focus::Side, "the box lives in the pane");
    assert_eq!(app.open.as_ref().unwrap().pane.as_ref().map(|p| &p.place), Some(&Place::Line { file: 0, new: Some(12), old: Some(12) }));
    app.handle_key(code(KeyCode::Left));
    press(&mut app, "x");
    assert_eq!(app.buffer.text(), "axb");
    app.handle_key(ctrl('a'));
    app.handle_key(code(KeyCode::Delete));
    assert_eq!(app.buffer.text(), "xb");
    app.handle_key(ctrl('e'));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
    press(&mut app, "y");
    assert_eq!(app.buffer.text(), "xb\ny", "alt-enter is a newline, not a send");
    app.handle_key(code(KeyCode::Esc));
    assert!(app.input.is_none());
    assert_eq!(app.focus, Focus::Review, "back where the box was opened from");
    press(&mut app, "j");
    press(&mut app, "c");
    assert_eq!(app.buffer.text(), "", "another line starts empty");
    assert_eq!(app.handle_key(code(KeyCode::Enter)), vec![], "an empty comment is dropped");
    press(&mut app, "k");
    press(&mut app, "c");
    assert_eq!(app.buffer.text(), "xb\ny", "the text left on line 12 comes back");
    let actions = app.handle_key(code(KeyCode::Enter));
    assert!(matches!(actions.as_slice(), [Action::SaveDraft { draft, .. }] if draft.body == "xb\ny"), "{actions:?}");
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
    press(&mut app, "Vjj");
    let open = app.open.as_ref().unwrap();
    assert_eq!(open.selection().count(), 3, "three lines, nothing between them");
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
    assert_eq!(app.input_label(), "reply to nina");
    let actions = type_text(&mut app, "agreed");
    let [Action::SaveDraft { draft, .. }] = actions.as_slice() else { panic!("{actions:?}") };
    assert_eq!(draft.reply_to.as_deref(), Some("c0ffee00c0ffee00"));
    assert_eq!(app.focus, Focus::Side);
    assert!(render(&mut app, 140, 24).contains("you · unsaved ◇"));
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
fn e_in_the_pane_edits_my_draft_and_d_deletes_it() {
    let mut app = with_saved_draft();
    press(&mut app, "l");
    assert_eq!(app.open.as_ref().unwrap().focused_draft(), Some(0));
    press(&mut app, "e");
    assert_eq!((app.input_label(), app.buffer.text()), ("edit draft".to_owned(), "nit"));
    let actions = type_text(&mut app, " (typo)");
    let [Action::UpdateDraft { key, id: 9, draft }] = actions.as_slice() else { panic!("{actions:?}") };
    assert_eq!(*key, mr_key());
    assert_eq!((draft.body.as_str(), draft.position.is_some()), ("nit (typo)", true), "the position travels with the edit");
    assert_eq!(app.open.as_ref().unwrap().review.drafts[0].body, "nit (typo)");
    assert_eq!(press(&mut app, "d"), vec![Action::DeleteDraft { key: mr_key(), id: 9 }]);
    assert_eq!(app.draft_count(), 0);
    assert_eq!(marker_here(&app), None, "the line is plain again");
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
    press(&mut app, "l");
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
    press(&mut app, "e");
    assert_eq!(app.input_label(), "edit draft");
    assert!(app.publish.is_none(), "the modal steps aside for the compose box in the pane");
    assert!(app.open.as_ref().unwrap().pane.is_some());
    type_text(&mut app, "!");
    press(&mut app, "Pp");
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
    assert_eq!(press(&mut app, "Vjs"), vec![], "s opens the compose box, prefilled");
    assert_eq!(app.input_label(), "new thread · charge.rs:12–-13");
    let actions = app.handle_key(ctrl('o'));
    let [Action::Compose { draft, .. }] = actions.as_slice() else { panic!("{actions:?}") };
    assert_eq!(
        draft,
        "```suggestion:-0+1\npub async fn charge(card: &Card, amount: Money) -> Result<Receipt> {\n    let client = Client::new();\n```\n"
    );
    assert!(app.input.is_none(), "the editor takes the text over");
    let input = Input::Comment { position: position.clone() };
    app.apply(Incoming::Composed { input: input.clone(), text: None });
    assert_eq!(app.take_actions(), vec![]);
    assert_eq!(app.draft_count(), 0);
    app.apply(Incoming::Composed { input, text: Some("from the editor".into()) });
    let actions = app.take_actions();
    assert!(matches!(actions.as_slice(), [Action::SaveDraft { index: 0, .. }]), "{actions:?}");
    assert_eq!(app.open.as_ref().unwrap().review.drafts[0].body, "from the editor");
    press(&mut app, "kl");
    assert_eq!(app.open.as_ref().unwrap().focused_draft(), Some(0));
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
    assert_eq!((text.fg, text.bg), (Color::Reset, Color::Reset), "on an unknown ground, no fill and the terminal's own text");
    assert_eq!(edge.bg, Color::Reset);
    let sign = sign_of_removed_line(&mut app);
    assert_eq!(sign.fg, Color::Red, "the sign alone says removed");
    app.theme = Theme::default().with_ground(0x1e1e2e);
    let (text, edge) = removed_line_cells(&mut app);
    assert_eq!((text.fg, text.bg), (Color::Reset, app.theme.removed_fill.unwrap()), "a known ground gets the tint");
    assert_eq!(edge.bg, app.theme.removed_fill.unwrap());
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
    Incoming::Queue { scope: scope.map(str::to_owned), me: "nina".into(), sections, opened: HashMap::new(), cached }
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
    press(&mut app, "Gkk");
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

#[test]
fn the_queue_names_me_when_no_login_did() {
    let mut app = App::new(Settings { me: String::new(), ..settings() });
    app.apply(Incoming::Queue { scope: None, me: "nina".into(), sections: sections(), opened: HashMap::new(), cached: false });
    assert_eq!(app.me, "nina");
    app.apply(Incoming::Queue { scope: None, me: "someone".into(), sections: sections(), opened: HashMap::new(), cached: false });
    assert_eq!(app.me, "nina", "a stored name is never replaced");
}

fn sum_review() -> Review {
    let file = DiffFile {
        diff: include_str!("../../review/fixtures/sum.diff").to_owned(),
        old_path: "src/sum.rs".into(),
        new_path: "src/sum.rs".into(),
        a_mode: "100644".into(),
        b_mode: "100644".into(),
        ..DiffFile::default()
    };
    let on_new_line_three = json!({
        "id": "d3", "individual_note": false,
        "notes": [{
            "id": 900, "type": "DiffNote", "body": "Why twenty?",
            "author": {"id": 3, "username": "lea", "name": "Léa"},
            "created_at": "2026-09-22T10:05:00Z", "updated_at": "2026-09-22T10:05:00Z",
            "system": false, "resolvable": true, "resolved": false,
            "position": {"base_sha": "aaaa", "head_sha": "bbbb", "start_sha": "aaaa", "position_type": "text",
                         "old_path": "src/sum.rs", "new_path": "src/sum.rs", "old_line": null, "new_line": 3}
        }]
    });
    Review::new(mr(), &[file], vec![fixture::discussion(&on_new_line_three.to_string())], &[])
}

fn with_sum_review() -> App {
    let mut app = with_queue();
    app.queue_move(0);
    app.handle_key(code(KeyCode::Enter));
    app.apply(Incoming::Review { key: mr_key(), review: Box::new(sum_review()), cached: None });
    app
}

/// Moves the cursor down until it sits on a row `wanted` accepts.
fn walk_to(app: &mut App, wanted: impl Fn(&Row) -> bool) {
    for _ in 0..40 {
        if app.open.as_ref().unwrap().row().is_some_and(&wanted) {
            return;
        }
        press(app, "j");
    }
    panic!("no such row");
}

fn on_pair(app: &mut App) {
    walk_to(app, |r| matches!(r, Row::Pair { .. }));
}

#[test]
fn a_one_word_change_reads_as_one_row_with_its_thread_under_it() {
    let app = with_sum_review();
    let rows = &app.open.as_ref().unwrap().rows;
    let pair = rows.iter().position(|r| matches!(r, Row::Pair { removed: 2, added: 3, .. })).expect("the b line pairs up");
    let open = app.open.as_ref().unwrap();
    assert!(open.review.marker_of(&open.review.markers(), &rows[pair]).is_some(), "the thread on the added line marks the pair");
    let lines: Vec<usize> = rows.iter().filter_map(|r| if let Row::Line { index, .. } = r { Some(*index) } else { None }).collect();
    assert_eq!(lines, vec![0, 1, 4, 5, 6, 7], "the rewritten d line stays split");
}

#[test]
fn big_d_splits_and_joins_again_keeping_the_cursor_and_saving_the_choice() {
    let mut app = with_sum_review();
    on_pair(&mut app);
    let actions = press(&mut app, "D");
    assert!(matches!(actions.as_slice(), [Action::SaveState { split: true, .. }]), "{actions:?}");
    let open = app.open.as_ref().unwrap();
    assert!(!open.rows.iter().any(|r| matches!(r, Row::Pair { .. })));
    assert!(matches!(open.row(), Some(Row::Line { index: 2 | 3, .. })), "the cursor stays on the b line: {:?}", open.row());
    let actions = press(&mut app, "D");
    assert!(matches!(actions.as_slice(), [Action::SaveState { split: false, .. }]));
    assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::Pair { .. })));
}

#[test]
fn a_refresh_keeps_the_split_choice() {
    let mut app = with_sum_review();
    press(&mut app, "D");
    app.apply(Incoming::Review { key: mr_key(), review: Box::new(sum_review()), cached: None });
    assert!(app.open.as_ref().unwrap().review.split);
}

#[test]
fn c_on_a_pair_comments_the_new_side_and_big_c_the_old_side() {
    let mut app = with_sum_review();
    on_pair(&mut app);
    press(&mut app, "c");
    assert_eq!(app.input_label(), "new thread · sum.rs:3");
    app.handle_key(code(KeyCode::Esc));
    press(&mut app, "C");
    assert_eq!(app.input_label(), "new thread · sum.rs:-3");
    app.handle_key(code(KeyCode::Esc));
    press(&mut app, "k");
    press(&mut app, "C");
    assert_eq!(app.input_label(), "", "C only means something on a pair");
}

#[test]
fn v_treats_a_pair_as_its_added_line() {
    let mut app = with_sum_review();
    walk_to(&mut app, |r| matches!(r, Row::Line { index: 1, .. }));
    press(&mut app, "Vjjc");
    let Some(Input::Comment { position }) = &app.input else { panic!("no comment input") };
    let start = position.start.expect("a range");
    assert_eq!((start.new, position.line.new), (Some(2), Some(4)), "from line 2 through the pair to line 4");
}

#[test]
fn moving_and_jumping_step_over_pair_rows_like_lines() {
    let mut app = with_sum_review();
    on_pair(&mut app);
    press(&mut app, "j");
    assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::Line { .. })), "no thread row after the pair");
    press(&mut app, "kk");
    assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::Line { index: 1, .. })));
    press(&mut app, "[c");
    assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::Hunk { .. })));
}

#[test]
fn snapshot_review_inline() {
    let mut app = with_sum_review();
    on_pair(&mut app);
    insta::assert_snapshot!("review_inline", render(&mut app, 100, 18));
}

/// The cells holding the first character of `text` on screen.
fn cell_of(buffer: &ratatui::buffer::Buffer, text: &str) -> ratatui::buffer::Cell {
    let area = buffer.area;
    (0..area.height)
        .find_map(|y| {
            let row: Vec<String> = (0..area.width).map(|x| buffer[(x, y)].symbol().to_owned()).collect();
            let joined: String = row.concat();
            joined.find(text).map(|byte| {
                let x = joined[..byte].chars().count();
                buffer[(x as u16, y)].clone()
            })
        })
        .expect("the text is on screen")
}

#[test]
fn the_old_word_is_struck_through_in_red_and_the_new_one_green() {
    let mut app = with_sum_review();
    let buffer = cells(&mut app, 100, 18);
    let row = cell_of(&buffer, "let b = 2;20;");
    assert_eq!(row.fg, Color::Reset, "the kept text reads like context");
    let old = cell_of(&buffer, "2;20;");
    assert_eq!(old.fg, Color::Red);
    assert!(old.modifier.contains(ratatui::style::Modifier::CROSSED_OUT));
    let new = cell_of(&buffer, "20;");
    assert_eq!(new.fg, Color::Green);
    assert!(!new.modifier.contains(ratatui::style::Modifier::CROSSED_OUT));
    app.theme = Theme::named("tokyonight").unwrap();
    let buffer = cells(&mut app, 100, 18);
    assert_eq!(cell_of(&buffer, "20;").bg, app.theme.added_word.unwrap(), "RGB themes fill the new word");
}

/// The sign column of the removed line: the one cell that still says `-` when there is no fill.
fn sign_of_removed_line(app: &mut App) -> ratatui::buffer::Cell {
    let buffer = cells(app, 120, 30);
    let text = "let client = Client::new();";
    let (x, y) = (0..30)
        .find_map(|y| {
            let row: String = (0..120).map(|x| buffer[(x, y)].symbol().to_owned()).collect();
            row.find(text).map(|i| (row[..i].chars().count() as u16, y))
        })
        .expect("the removed line is on screen");
    let sign = (0..x).rev().find(|&x| buffer[(x, y)].symbol() == "-").expect("a sign before the text");
    buffer[(sign, y)].clone()
}

#[test]
fn a_draft_whose_line_left_the_diff_is_named_before_publishing_and_m_moves_it_to_the_mr() {
    let mut app = with_saved_draft();
    let open = app.open.clone().unwrap();
    let path = open.review.drafts[0].anchor.as_ref().unwrap().path.clone();
    let gone = crate::review::Draft {
        id: Some(5),
        anchor: Some(crate::review::Anchor { path, side: crate::review::Side::New, line: 9_999 }),
        ..open.review.drafts[0].clone()
    };
    let drafts = vec![open.review.drafts[0].clone(), gone];
    app.open = Some(open.with_review(open.review.with_drafts(drafts)));
    press(&mut app, "P");
    assert_eq!(app.handle_key(code(KeyCode::Enter)), vec![], "nothing is sent while a draft hangs on a missing line");
    assert_eq!(app.publish.as_ref().unwrap().selected, 1, "the cursor lands on the stranded draft");
    assert!(app.live_toast().unwrap().text.contains("m moves it to the MR"));
    let actions = press(&mut app, "m");
    let [Action::DeleteDraft { id: 5, .. }, Action::SaveDraft { index: 1, draft, .. }] = actions.as_slice() else { panic!("{actions:?}") };
    assert_eq!((draft.anchor.clone(), draft.position.clone()), (None, None), "the note now sits on the MR");
    assert!(app.open.as_ref().unwrap().review.stranded().is_empty());
}

fn run_line(app: &mut App, line: &str) -> Vec<Action> {
    press(app, ":");
    press(app, line);
    app.handle_key(code(KeyCode::Enter))
}

#[test]
fn go_opens_an_mr_by_number_or_full_reference() {
    let mut app = with_queue();
    let actions = run_line(&mut app, "go !41");
    assert!(matches!(actions.as_slice(), [Action::Open(key)] if key.number == 41), "{actions:?}");
    let actions = run_line(&mut app, "go other/thing!7");
    assert!(matches!(actions.as_slice(), [Action::Open(key)] if key.project == "other/thing" && key.number == 7), "{actions:?}");
    assert_eq!(run_line(&mut app, "go !999"), vec![]);
    assert!(app.live_toast().unwrap().text.contains("no MR !999"));
}

#[test]
fn set_theme_changes_it_now_and_asks_to_save_it() {
    let mut app = with_queue();
    assert_eq!(run_line(&mut app, "set theme=nord"), vec![Action::SaveTheme("nord".into())]);
    assert_eq!(app.theme.name, "nord");
    assert_eq!(run_line(&mut app, "set theme=nope"), vec![]);
    assert!(app.live_toast().unwrap().danger);
}

#[test]
fn tab_completes_verbs_mrs_and_themes_and_up_recalls() {
    let mut app = with_queue();
    press(&mut app, ":pu");
    app.handle_key(code(KeyCode::Tab));
    assert_eq!(app.palette.as_ref().unwrap().input, "publish ");
    app.handle_key(code(KeyCode::Esc));
    assert!(app.palette.is_none());
    assert!(app.completions_for("go ").contains(&"!42".to_owned()));
    assert!(app.completions_for("set theme=").contains(&"theme=tokyonight".to_owned()));
    run_line(&mut app, "help");
    assert_eq!(app.help, Some(0));
    press(&mut app, "x:");
    app.handle_key(code(KeyCode::Up));
    assert_eq!(app.palette.as_ref().unwrap().input, "help");
}

#[test]
fn ctrl_k_jumps_to_a_file_of_the_open_mr_or_to_another_mr() {
    let mut app = with_review();
    app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    let first_file = app.jump.as_ref().unwrap().matches()[0].clone();
    assert!(matches!(first_file.target, crate::tui::jump::Target::File(0)), "files of the open MR come first");
    app.handle_key(code(KeyCode::Enter));
    assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::File { index: 0, .. })));
    app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    press(&mut app, "!41");
    let actions = app.handle_key(code(KeyCode::Enter));
    assert!(matches!(actions.as_slice(), [Action::Open(key)] if key.number == 41));
}

#[test]
fn snapshot_palette_and_jump() {
    let mut app = with_review();
    press(&mut app, ":pu");
    insta::assert_snapshot!("palette_ghost", render(&mut app, 100, 20));
    app.handle_key(code(KeyCode::Esc));
    app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    press(&mut app, "ch");
    insta::assert_snapshot!("jump", render(&mut app, 100, 20));
}

#[test]
fn t_opens_the_tree_on_the_file_under_the_cursor_and_enter_jumps_back() {
    let mut app = with_review();
    press(&mut app, "]cj");
    press(&mut app, "t");
    assert_eq!(app.focus, Focus::Side);
    let open = app.open.clone().unwrap();
    let rows = open.tree_rows();
    let selected = &rows[open.tree.as_ref().unwrap().selected];
    assert!(matches!(selected, crate::review::tree::TreeRow::File { index: 0, .. }), "{rows:?}");
    press(&mut app, "G");
    app.handle_key(code(KeyCode::Enter));
    assert_eq!(app.focus, Focus::Review, "enter on a file hands the keys to the review");
    let last_file = app.open.as_ref().unwrap().review.files.len() - 1;
    assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::File { index, .. }) if *index == last_file));
    assert!(app.open.as_ref().unwrap().tree.is_some(), "the tree stays open beside the diff");
    press(&mut app, "t");
    assert!(app.open.as_ref().unwrap().tree.is_none());
}

#[test]
fn zv_marks_the_file_viewed_folds_it_and_saves_its_fingerprint() {
    let mut app = with_review();
    press(&mut app, "]cj");
    let actions = press(&mut app, "zv");
    let open = app.open.clone().unwrap();
    let path = open.review.files[0].new_path.clone();
    assert!(open.review.viewed.contains(&path));
    assert!(!open.review.fold.file_is_open(&path), "a viewed file folds");
    assert!(
        matches!(actions.as_slice(), [Action::SaveState { viewed, .. }] if viewed.get(&path) == Some(&open.review.files[0].fingerprint())),
        "{actions:?}"
    );
    press(&mut app, "zv");
    assert!(!app.open.as_ref().unwrap().review.viewed.contains(&path));
    assert!(app.open.as_ref().unwrap().review.fold.file_is_open(&path));
}

#[test]
fn snapshot_file_tree() {
    let mut app = with_review();
    press(&mut app, "]cjzvt");
    insta::assert_snapshot!("file_tree", render(&mut app, 120, 20));
}

#[test]
fn big_w_hides_whitespace_only_changes_and_back() {
    let mut app = with_review();
    press(&mut app, "W");
    assert!(app.open.as_ref().unwrap().review.quiet_whitespace);
    assert!(app.live_toast().unwrap().text.contains("hidden"));
    press(&mut app, "W");
    assert!(!app.open.as_ref().unwrap().review.quiet_whitespace);
}

#[test]
fn zz_reads_the_diff_alone_and_h_brings_the_queue_back() {
    let mut app = with_queue();
    press(&mut app, "zz");
    assert!(!app.reading, "nothing to read before an MR is open");
    let mut app = with_review();
    press(&mut app, "zz");
    assert!(app.reading);
    let screen = render(&mut app, 160, 20);
    assert!(!screen.contains("Queue"), "{screen}");
    press(&mut app, "h");
    assert!(!app.reading);
    assert_eq!(app.focus, Focus::Queue);
}

#[test]
fn snapshot_wrapped_lines() {
    let mut app = with_review();
    press(&mut app, "w");
    insta::assert_snapshot!("wrapped", render(&mut app, 80, 24));
}

#[test]
fn plus_loads_the_file_once_and_shows_ten_more_lines_around_the_hunk() {
    let mut app = with_review();
    press(&mut app, "]cj");
    let actions = press(&mut app, "+");
    let open = app.open.clone().unwrap();
    let path = open.review.files[0].new_path.clone();
    assert!(
        matches!(actions.as_slice(), [Action::LoadFile { path: p, sha, .. }] if *p == path && *sha == open.review.mr.refs.head),
        "{actions:?}"
    );
    let text: String = (1..=40).map(|n| format!("line {n}\n")).collect();
    app.apply(Incoming::File { key: mr_key(), path: path.clone(), text });
    let contexts = app.open.as_ref().unwrap().rows.iter().filter(|r| matches!(r, Row::Context { .. })).count();
    assert_eq!(contexts, 20, "ten above, ten below");
    assert_eq!(press(&mut app, "+"), vec![], "the file is read once");
    let screen = render(&mut app, 120, 40);
    assert!(screen.contains("line 2"), "{screen}");
}

#[test]
fn a_poll_that_brings_notes_says_so_until_the_next_key() {
    let mut app = with_review();
    let mut more = discussions();
    let extra = more[1].notes[0].clone();
    more[1].notes.push(crate::forge::Note { id: 999, ..extra });
    app.apply(Incoming::Discussions { key: mr_key(), discussions: more });
    assert_eq!(app.news.as_deref(), Some("● 1 new note"));
    assert!(render(&mut app, 120, 20).contains("● 1 new note"));
    press(&mut app, "j");
    assert_eq!(app.news, None);
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

fn view(path: &str, sha: &str, line: u32, note: Option<&str>) -> Action {
    Action::View { key: mr_key(), path: path.into(), sha: sha.into(), line, note: note.map(str::to_owned) }
}

#[test]
fn v_hands_the_head_file_at_the_cursors_line() {
    let mut app = with_review();
    on_line(&mut app);
    assert_eq!(press(&mut app, "v"), vec![view("src/pay/charge.rs", "bbbb", 12, None)]);
    assert!(app.live_toast().unwrap().text.contains("opening charge.rs"));
    press(&mut app, "j");
    assert_eq!(press(&mut app, "v"), vec![view("src/pay/charge.rs", "bbbb", 13, None)], "a removed line opens the next head line");
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
fn v_in_the_thread_pane_and_the_tree_uses_their_file() {
    let mut app = with_review();
    press(&mut app, "]n");
    app.handle_key(code(KeyCode::Enter));
    assert_eq!(app.focus, Focus::Side);
    assert_eq!(press(&mut app, "v"), vec![view("src/pay/charge.rs", "bbbb", 13, None)], "the old line 13 opens at head line 13");
    app.close_pane();
    press(&mut app, "t");
    assert_eq!(press(&mut app, "v"), vec![view("src/pay/charge.rs", "bbbb", 1, None)], "the tree opens on the cursor's file");
}

#[test]
fn deleted_and_binary_files_say_what_they_can() {
    let mut app = with_review();
    let deleted = DiffFile {
        diff: "@@ -1,2 +0,0 @@\n-a\n-b\n".into(),
        old_path: "src/gone.rs".into(),
        new_path: "src/gone.rs".into(),
        deleted_file: true,
        ..DiffFile::default()
    };
    let binary = DiffFile { old_path: "logo.png".into(), new_path: "logo.png".into(), ..DiffFile::default() };
    let review = Review::new(mr(), &[deleted, binary], vec![], &[]);
    app.apply(Incoming::Review { key: mr_key(), review: Box::new(review), cached: None });
    app.review_jump_to(|row| matches!(row, Row::Line { file: 0, .. }));
    assert_eq!(press(&mut app, "v"), vec![view("src/gone.rs", "aaaa", 1, Some("deleted in this MR · showing the old file"))]);
    app.review_jump_to(|row| matches!(row, Row::File { index: 1, .. }));
    assert_eq!(press(&mut app, "v"), vec![]);
    assert!(app.live_toast().unwrap().text.starts_with("binary file"));
}

fn ready(key: MrKey) -> Incoming {
    let view =
        crate::open::View { argv: vec!["hx".into(), "/tmp/charge.rs:12".into()], shown: "charge.rs:12".into(), note: None, _copy: None };
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

#[test]
fn a_range_comment_marks_its_other_lines_while_the_pane_is_on_it() {
    let mut app = with_review();
    on_line(&mut app);
    press(&mut app, "Vjjc");
    type_text(&mut app, "these three lines");
    let screen = render(&mut app, 160, 24);
    let line_12 = screen.lines().find(|l| l.contains("12   12")).expect("line 12 on screen");
    assert!(line_12.contains("│   12"), "the first line of the range shows the bar: {line_12}");
    let line_13 = screen.lines().find(|l| l.contains("13 +    let client")).expect("line 13 on screen");
    assert!(line_13.contains("◇"), "the last line carries the draft mark: {line_13}");
}

#[test]
fn below_120_columns_the_pane_is_a_page_and_h_goes_back_to_the_diff() {
    let mut app = with_review();
    press(&mut app, "]n");
    press(&mut app, "l");
    let page = render(&mut app, 100, 20);
    assert!(page.contains("charge.rs:-13") && !page.contains("Queue"), "the pane alone:\n{page}");
    press(&mut app, "h");
    let diff = render(&mut app, 100, 20);
    assert!(diff.contains("let client = Client::new()") && !diff.contains("charge.rs:-13 ·"), "the diff alone:\n{diff}");
    assert!(app.open.as_ref().unwrap().pane.is_some(), "the pane waits for l");
}

#[test]
fn snapshot_narrow_pane_and_a_three_line_box() {
    let mut app = with_review();
    press(&mut app, "]n");
    press(&mut app, "l");
    insta::assert_snapshot!("pane_narrow", render(&mut app, 100, 20));
    press(&mut app, "r");
    press(&mut app, "agreed");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
    press(&mut app, "keys are per card");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
    press(&mut app, "and per amount");
    insta::assert_snapshot!("compose_three_lines", render(&mut app, 160, 24));
}

fn triaging() -> App {
    let mut app = App::new(Settings { triage: true, ..settings() });
    app.today = today();
    app
}

fn verdict_for(mr: &crate::forge::QueueMr, urgency: f64, size: crate::ai::triage::Size) -> crate::ai::triage::Verdict {
    crate::ai::triage::Verdict { urgency, size, seen: mr.updated_at }
}

#[test]
fn a_fresh_queue_asks_jev_once_per_mr_and_not_at_all_when_it_is_off() {
    let mut off = app();
    off.apply(Incoming::Queue { scope: None, me: "nina".into(), sections: sections(), opened: HashMap::new(), cached: false });
    assert!(off.take_actions().is_empty(), "nothing leaves while Jev is off");
    let mut app = triaging();
    app.apply(Incoming::Queue { scope: None, me: "nina".into(), sections: sections(), opened: HashMap::new(), cached: true });
    assert!(app.take_actions().is_empty(), "a cached queue asks nothing");
    let fresh = || Incoming::Queue { scope: None, me: "nina".into(), sections: sections(), opened: HashMap::new(), cached: false };
    app.apply(fresh());
    let asked = app.take_actions();
    assert!(!asked.is_empty() && asked.iter().all(|a| matches!(a, Action::Triage(_))), "{asked:?}");
    app.apply(fresh());
    assert!(app.take_actions().is_empty(), "an MR being asked about is not asked twice");
}

#[test]
fn verdicts_mark_rows_and_lead_the_review_section_by_urgency() {
    let mut app = triaging();
    app.apply(Incoming::Queue { scope: None, me: "nina".into(), sections: sections(), opened: HashMap::new(), cached: false });
    let _ = app.take_actions();
    let to_review = app.sections.clone().unwrap().to_review;
    let last = to_review.last().unwrap().clone();
    app.apply(Incoming::Triaged { key: last.key(), verdict: verdict_for(&last, 2.9, crate::ai::triage::Size::Focused) });
    assert_eq!(app.mark(&last), Some(super::Mark::Urgent));
    let first_row = app.queue_rows().into_iter().find_map(|r| match r {
        QueueRow::Mr(mr) => Some(mr.key()),
        QueueRow::Section { .. } => None,
    });
    assert_eq!(first_row, Some(last.key()), "the urgent MR leads To review");
    let moved = crate::forge::QueueMr { updated_at: last.updated_at + chrono::TimeDelta::hours(1), ..last.clone() };
    assert_eq!(app.mark(&moved), None, "a verdict on an older state marks nothing");
    let screen = render(&mut app, 100, 16);
    assert!(screen.contains(&format!("! !{}", last.number)), "{screen}");
}

#[test]
fn a_fresh_review_asks_for_a_reading_once_per_head_and_it_tints_the_tree() {
    let mut app = triaging();
    app.apply(Incoming::Queue { scope: None, me: "nina".into(), sections: sections(), opened: HashMap::new(), cached: false });
    let _ = app.take_actions();
    app.queue_move(0);
    app.handle_key(code(KeyCode::Enter));
    app.apply(Incoming::Review { key: mr_key(), review: Box::new(review()), cached: None });
    let asked = app.take_actions();
    let [Action::Read { head, files, .. }] = asked.as_slice() else { panic!("{asked:?}") };
    assert!(!files.is_empty());
    let path = files[0].0.clone();
    let reading = crate::ai::triage::Reading {
        waits_on_me: true,
        risks: std::collections::BTreeMap::from([(path.clone(), crate::ai::triage::Risk::Security)]),
    };
    app.apply(Incoming::Read { key: mr_key(), head: head.clone(), reading });
    assert_eq!(app.risk(&path), Some(crate::ai::triage::Risk::Security));
    app.apply(Incoming::Review { key: mr_key(), review: Box::new(review()), cached: None });
    assert!(app.take_actions().is_empty(), "the same head is not read twice");
    let queued = app.sections.clone().unwrap().to_review.into_iter().find(|mr| mr.key() == mr_key()).unwrap();
    assert_eq!(app.mark(&queued), Some(super::Mark::WaitsOnMe), "waiting on me outranks every other mark");
    press(&mut app, "t");
    let buffer = cells(&mut app, 140, 30);
    let name = path.rsplit('/').next().unwrap();
    assert_eq!(cell_of(&buffer, name).fg, app.theme.danger, "a security file reads red in the tree");
}

#[test]
fn jev_failing_says_so_once_and_stops_asking() {
    let mut app = triaging();
    app.apply(Incoming::Failed { what: Failure::Triage, message: "⚠ typesafe unavailable: quota exhausted".into() });
    assert!(!app.triage);
    assert!(app.live_toast().is_some_and(|t| t.text.contains("quota")));
    app.apply(Incoming::Failed { what: Failure::Triage, message: "again".into() });
    assert!(app.live_toast().is_some_and(|t| t.text.contains("quota")), "the second failure stays quiet");
    app.apply(Incoming::Queue { scope: None, me: "nina".into(), sections: sections(), opened: HashMap::new(), cached: false });
    assert!(app.take_actions().is_empty());
}

fn asking() -> App {
    let mut app = App::new(Settings { ask: Some("claude-opus-5".into()), ..settings() });
    app.today = today();
    app.apply(Incoming::Queue { scope: None, me: "nina".into(), sections: sections(), opened: HashMap::new(), cached: false });
    app.queue_move(0);
    app.handle_key(code(KeyCode::Enter));
    app.apply(Incoming::Review { key: mr_key(), review: Box::new(review()), cached: None });
    app
}

fn the_ask(actions: &[Action]) -> (u64, crate::ai::anthropic::Ask, bool) {
    let [Action::Ask { id, request, fresh, .. }] = actions else { panic!("{actions:?}") };
    (*id, (**request).clone(), *fresh)
}

fn answer(app: &App) -> super::Answer {
    app.open.as_ref().unwrap().answer.clone().expect("an answer holds the pane")
}

fn done(model: &str) -> crate::ai::anthropic::Outcome {
    crate::ai::anthropic::Outcome {
        stop: crate::ai::anthropic::Stop::Done,
        usage: crate::ai::anthropic::Usage { cache_read: 1200, output: 40, ..Default::default() },
        model: model.into(),
    }
}

#[test]
fn a_says_claude_is_off_until_the_config_and_a_key_switch_it_on() {
    let mut app = with_review();
    on_line(&mut app);
    assert!(press(&mut app, "ae").is_empty());
    assert!(app.live_toast().is_some_and(|t| t.text.contains("[ai.anthropic] enabled = true")));
}

#[test]
fn a_e_explains_the_hunk_under_the_cursor_and_streams_into_the_pane() {
    let mut app = asking();
    on_line(&mut app);
    let (id, request, fresh) = the_ask(&press(&mut app, "ae"));
    assert!(!fresh);
    assert_eq!(request.system.len(), 3, "rules, the MR, the hunk");
    assert!(request.system[2].text.contains("@@"));
    assert_eq!(request.turns.last().unwrap().text, crate::ai::context::Prompt::Explain.question());
    assert_eq!(app.focus, Focus::Side);
    assert!(
        app.live_toast().is_some_and(|t| t.text.contains("claude-opus-5") && t.text.contains("Anthropic")),
        "the first question says where the MR goes"
    );
    app.apply(Incoming::Answer { key: mr_key(), id, part: super::Part::Text("It adds ".into()) });
    app.apply(Incoming::Answer { key: mr_key(), id, part: super::Part::Restart });
    app.apply(Incoming::Answer { key: mr_key(), id, part: super::Part::Text("a retry.".into()) });
    app.apply(Incoming::Answer { key: mr_key(), id: id + 7, part: super::Part::Text(" stale".into()) });
    assert_eq!(answer(&app).text, "a retry.", "a restart voids the partial text and an older stream is ignored");
    app.apply(Incoming::Answer { key: mr_key(), id, part: super::Part::Done { outcome: done("claude-opus-5"), cached_text: None } });
    assert_eq!(answer(&app).state, super::AnswerState::Done(done("claude-opus-5")));
    let screen = render(&mut app, 150, 24);
    assert!(screen.contains("Claude · explain") && screen.contains("a retry.") && screen.contains("1.2k cached"), "{screen}");
}

#[test]
fn an_answer_becomes_a_draft_on_the_lines_asked_about() {
    let mut app = asking();
    on_line(&mut app);
    let (id, ..) = the_ask(&press(&mut app, "ae"));
    app.apply(Incoming::Answer {
        key: mr_key(),
        id,
        part: super::Part::Done { outcome: done("claude-opus-5"), cached_text: Some("Rename this.".into()) },
    });
    assert!(answer(&app).cached);
    press(&mut app, "c");
    assert!(matches!(app.input, Some(Input::Comment { .. })), "{:?}", app.input);
    assert_eq!(app.buffer.text(), "Rename this.", "editable before it is saved");
    let actions = app.handle_key(code(KeyCode::Enter));
    assert!(matches!(actions.as_slice(), [Action::SaveDraft { .. }]), "{actions:?}");
}

#[test]
fn a_follow_up_carries_the_conversation_and_r_asks_again_fresh() {
    let mut app = asking();
    on_line(&mut app);
    let (id, ..) = the_ask(&press(&mut app, "ae"));
    app.apply(Incoming::Answer { key: mr_key(), id, part: super::Part::Text("First answer.".into()) });
    app.apply(Incoming::Answer { key: mr_key(), id, part: super::Part::Done { outcome: done("claude-opus-5"), cached_text: None } });
    app.handle_key(code(KeyCode::Enter));
    assert_eq!(app.input, Some(Input::FollowUp));
    let (next, request, _) = the_ask(&type_text(&mut app, "and the tests?"));
    assert!(next > id);
    let roles: Vec<_> = request.turns.iter().map(|t| (t.role, t.text.as_str())).collect();
    assert_eq!(
        roles[1..],
        [(crate::ai::anthropic::Role::Assistant, "First answer."), (crate::ai::anthropic::Role::User, "and the tests?")]
    );
    let (_, again, fresh) = the_ask(&press(&mut app, "R"));
    assert!(fresh && again == request, "R asks the same thing past the cache");
}

#[test]
fn a_c_asks_for_the_concern_then_drafts_a_comment_about_it() {
    let mut app = asking();
    on_line(&mut app);
    assert!(press(&mut app, "ac").is_empty());
    assert_eq!(app.input_label(), "comment about");
    let (_, request, _) = the_ask(&type_text(&mut app, "naming"));
    assert!(request.turns[0].text.ends_with("about: naming"));
    assert!(matches!(answer(&app).target, super::ask::Target::Lines(_)));
}

#[test]
fn a_t_summarises_the_thread_and_its_answer_becomes_a_reply() {
    let mut app = asking();
    press(&mut app, "]c");
    app.handle_key(code(KeyCode::Enter));
    app.handle_key(code(KeyCode::Enter));
    press(&mut app, "]n");
    let (id, request, _) = the_ask(&press(&mut app, "at"));
    assert!(request.system[2].text.contains("## Thread"));
    app.apply(Incoming::Answer {
        key: mr_key(),
        id,
        part: super::Part::Done { outcome: done("claude-opus-5"), cached_text: Some("Waits on nina.".into()) },
    });
    press(&mut app, "c");
    assert!(matches!(app.input, Some(Input::Reply { .. })), "{:?}", app.input);
}

#[test]
fn ai_off_stops_every_ai_call_for_the_session() {
    let mut app = asking();
    app.triage = true;
    on_line(&mut app);
    press(&mut app, ":ai off");
    app.handle_key(code(KeyCode::Enter));
    assert_eq!((app.ask_model.as_deref(), app.triage), (None, false));
    assert!(press(&mut app, "ae").is_empty());
    press(&mut app, ":ask why");
    assert!(app.handle_key(code(KeyCode::Enter)).is_empty());
}

#[test]
fn snapshot_answer_pane() {
    let mut app = asking();
    on_line(&mut app);
    let (id, ..) = the_ask(&press(&mut app, "ae"));
    let text = "It swaps the client for one keyed by the card, so a retry **reuses** the idempotency key.\n\n- `charge` now takes the key\n- nothing else moves";
    app.apply(Incoming::Answer {
        key: mr_key(),
        id,
        part: super::Part::Done { outcome: done("claude-opus-5"), cached_text: Some(text.into()) },
    });
    insta::assert_snapshot!("answer_pane", render(&mut app, 150, 20));
}

fn run() -> crate::forge::checks::Checks {
    use crate::forge::checks::{Checks, Found, Job, JobState};
    let job = |stage: &str, order: &str, name: &str, state: JobState, seconds: u64| Found {
        stage: stage.into(),
        order: order.into(),
        job: Job {
            name: name.into(),
            state,
            seconds: Some(seconds),
            web_url: format!("https://ci.example/{name}"),
            allowed_to_fail: false,
        },
    };
    Checks::from_jobs(
        Some("https://ci.example/run".into()),
        vec![
            job("check", "1", "lint", JobState::Passed, 7),
            job("test", "2", "unit", JobState::Passed, 63),
            job("test", "3", "flaky", JobState::Failed, 4),
        ],
    )
}

fn with_pipeline() -> App {
    let mut app = with_review();
    let head = app.open.as_ref().unwrap().review.mr.refs.head.clone();
    assert_eq!(press(&mut app, "p"), vec![Action::LoadChecks { key: mr_key(), head }]);
    app.apply(Incoming::Checks { key: mr_key(), checks: Some(run()) });
    app
}

#[test]
fn p_opens_the_pipeline_on_its_first_failure_and_o_opens_that_job() {
    let mut app = with_pipeline();
    assert_eq!(app.focus, Focus::Side);
    let pipeline = app.open.as_ref().unwrap().pipeline.clone().unwrap();
    assert_eq!(pipeline.jobs()[pipeline.selected].name, "flaky", "the cursor lands on the failure");
    assert_eq!(press(&mut app, "o"), vec![Action::OpenUrl("https://ci.example/flaky".into())]);
    press(&mut app, "j");
    assert_eq!(press(&mut app, "y"), vec![Action::Yank("https://ci.example/unit".into())]);
    press(&mut app, "p");
    assert!(app.open.as_ref().unwrap().pipeline.is_none());
    assert_eq!(app.focus, Focus::Review);
}

#[test]
fn a_run_still_going_is_asked_again_while_the_pane_shows_it() {
    use crate::forge::checks::JobState;
    let mut app = with_review();
    press(&mut app, "p");
    let mut going = run();
    going.stages[1].jobs[0].state = JobState::Running;
    app.apply(Incoming::Checks { key: mr_key(), checks: Some(going) });
    assert!(app.tick().iter().all(|a| !matches!(a, Action::LoadChecks { .. })), "not before its time");
    app.now += Duration::from_secs(16);
    assert!(app.tick().iter().any(|a| matches!(a, Action::LoadChecks { .. })));
    assert!(app.tick().iter().all(|a| !matches!(a, Action::LoadChecks { .. })), "asked once");
    app.apply(Incoming::Checks { key: mr_key(), checks: Some(run()) });
    app.now += Duration::from_secs(60);
    assert!(app.tick().iter().all(|a| !matches!(a, Action::LoadChecks { .. })), "a finished run is left alone");
}

#[test]
fn r_asks_again_and_a_failure_or_no_run_says_so_in_the_pane() {
    let mut app = with_pipeline();
    assert!(matches!(press(&mut app, "r").as_slice(), [Action::LoadChecks { .. }]));
    app.apply(Incoming::Failed { what: Failure::Checks, message: "HTTP 500".into() });
    assert_eq!(app.open.as_ref().unwrap().pipeline.as_ref().unwrap().run, Run::Failed("HTTP 500".into()));
    app.apply(Incoming::Checks { key: mr_key(), checks: None });
    assert_eq!(app.open.as_ref().unwrap().pipeline.as_ref().unwrap().run, Run::Nothing);
}

#[test]
fn the_pipeline_and_the_file_tree_share_the_right_pane() {
    let mut app = with_pipeline();
    press(&mut app, "h");
    press(&mut app, "t");
    let open = app.open.as_ref().unwrap();
    assert!(open.tree.is_some() && open.pipeline.is_none());
    app.handle_key(code(KeyCode::Esc));
    press(&mut app, "p");
    let open = app.open.as_ref().unwrap();
    assert!(open.pipeline.is_some() && open.tree.is_none());
}

#[test]
fn snapshot_pipeline_pane() {
    let mut app = with_pipeline();
    insta::assert_snapshot!("pipeline", render(&mut app, 150, 20));
}

/// The review with one more thread on added line 13, whose note carries `body` and GitLab's `suggestions`.
fn with_suggestion(body: &str, suggestions: serde_json::Value) -> App {
    let mut app = with_review();
    let mut note: serde_json::Value = serde_json::from_str(include_str!("../../forge/gitlab/fixtures/diff_note.json")).unwrap();
    note["id"] = json!("5ugg");
    note["notes"][0]["body"] = json!(body);
    note["notes"][0]["position"]["new_line"] = json!(13);
    note["notes"][0]["suggestions"] = suggestions;
    let mut all = discussions();
    all.push(fixture::discussion(&note.to_string()));
    app.apply(Incoming::Discussions { key: mr_key(), discussions: all });
    app.open_pane(Place::Line { file: 0, new: Some(13), old: None });
    app.focus = Focus::Side;
    app
}

#[test]
fn big_s_asks_first_and_commits_only_on_y() {
    let body = "Nit:\n```suggestion:-0+0\n    let client = Client::default();\n```";
    let mut app = with_suggestion(body, json!([{"id": 77, "applied": false, "appliable": true}]));
    assert_eq!(press(&mut app, "S"), vec![]);
    let asked = app.confirm.clone().expect("a question waits");
    assert_eq!((asked.suggestion.id, asked.suggestion.path.as_str(), asked.suggestion.line), (Some(77), "src/pay/charge.rs", 13));
    assert!(render(&mut app, 150, 20).contains("commit this suggestion to src/pay/charge.rs on feat/checkout?"));
    assert_eq!(press(&mut app, "n"), vec![], "any other key cancels");
    assert!(app.confirm.is_none());
    press(&mut app, "S");
    let actions = press(&mut app, "y");
    let [Action::Apply { branch, suggestion, .. }] = actions.as_slice() else { panic!("{actions:?}") };
    let branch = branch.clone();
    assert_eq!((branch.as_str(), suggestion.text.as_str()), ("feat/checkout", "    let client = Client::default();"));
    app.apply(Incoming::Applied { key: mr_key(), branch });
    assert_eq!(app.take_actions(), vec![Action::RefreshMr(mr_key())], "the diff reads the new commit");
    assert!(app.live_toast().unwrap().text.contains("committed on feat/checkout"));
}

#[test]
fn big_s_says_why_when_there_is_nothing_to_apply() {
    let mut plain = with_suggestion("Should this retry?", json!([]));
    press(&mut plain, "S");
    assert!(plain.confirm.is_none());
    assert_eq!(plain.live_toast().unwrap().text, "this note has no suggestion");
    let body = "```suggestion:-0+0\nx\n```";
    let mut done = with_suggestion(body, json!([{"id": 77, "applied": true, "appliable": false}]));
    press(&mut done, "S");
    assert_eq!(done.live_toast().unwrap().text, "this suggestion is already applied");
    let mut github = with_suggestion(body, json!([]));
    press(&mut github, "S");
    assert_eq!(github.confirm.as_ref().map(|c| c.suggestion.id), Some(None), "GitHub lists no id: revu builds the commit");
}
