//! What the app tests share: an app on the fixture queue and MR, keys to press, and the screen it draws.
#![allow(clippy::unwrap_used, clippy::expect_used)]
pub(super) use super::*;
pub(super) use crate::forge::gitlab::fixture;
pub(super) use crate::forge::{DiffFile, Discussion, Emoji, Kind, Mr, PipelineStatus};
pub(super) use crate::review::{Place, Row};
pub(super) use crate::tui::help::Help;
pub(super) use crate::tui::theme::Theme;
pub(super) use crate::tui::ui;
pub(super) use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
pub(super) use ratatui::Terminal;
pub(super) use ratatui::backend::TestBackend;
pub(super) use ratatui::style::Color;
pub(super) use serde_json::json;
pub(super) use std::time::Duration;

pub(super) fn mr_key() -> MrKey {
    fixture::key()
}

pub(super) fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

pub(super) fn code(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

pub(super) fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

pub(super) fn press(app: &mut App, keys: &str) -> Vec<Action> {
    keys.chars().flat_map(|c| app.handle_key(key(c))).collect()
}

pub(super) fn today() -> DateTime<Utc> {
    "2026-09-22T12:00:00Z".parse().unwrap()
}

pub(super) fn settings() -> Settings {
    Settings {
        notify: true,
        hosts: crate::forge::Hosts::one("gitlab.com", Kind::GitLab),
        theme: Theme::default(),
        host: "gitlab.com".into(),
        me: "nina".into(),
        project: None,
        ground: None,
        triage: false,
        ask: None,
        keymap: crate::keymap::Keymap::default(),
        pictures: None,
        queue_layout: crate::config::QueueLayout::default(),
        zen_width: None,
        views: vec![],
        share: vec![],
        prefetch: 0,
        ascii: false,
        quit_confirm: true,
        usage: false,
    }
}

pub(super) fn app() -> App {
    let mut app = App::new(settings());
    app.today = today();
    app
}

pub(super) fn sections() -> Sections {
    fixture::queue(include_str!("../../forge/gitlab/fixtures/queue.json")).sections(&[])
}

pub(super) fn with_queue() -> App {
    let mut app = app();
    app.apply(Incoming::Queue { scope: None, me: "nina".into(), sections: sections(), opened: HashMap::new(), cached: false });
    app
}

pub(super) fn mr() -> Mr {
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

pub(super) fn diffs() -> Vec<DiffFile> {
    vec![
        DiffFile {
            diff: include_str!("../../review/fixtures/charge.diff").to_owned(),
            old_path: "src/pay/charge.rs".into(),
            new_path: "src/pay/charge.rs".into(),
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

pub(super) fn discussions() -> Vec<Discussion> {
    vec![
        fixture::discussion(include_str!("../../forge/gitlab/fixtures/discussions.json")),
        fixture::discussion(include_str!("../../forge/gitlab/fixtures/diff_note.json")),
        fixture::discussion(include_str!("../../review/fixtures/old_side_note.json")),
    ]
}

pub(super) fn review() -> Review {
    Review::new(mr(), &diffs(), discussions(), &["*.lock".into()])
}

pub(super) fn with_review() -> App {
    let mut app = with_queue();
    app.queue_move(0);
    assert_eq!(press(&mut app, "\r"), vec![]);
    let actions = app.handle_key(code(KeyCode::Enter));
    assert_eq!(actions, vec![Action::Open(mr_key())]);
    app.apply(Incoming::Review { key: mr_key(), review: Box::new(review()), cached: None });
    let asked = app.take_actions();
    assert!(asked.iter().any(|a| matches!(a, Action::LoadDeployments { branch, .. } if branch == "feat/checkout")), "{asked:?}");
    app
}

pub(super) fn render(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| ui::draw(f, app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..height)
        .map(|y| (0..width).map(|x| buffer[(x, y)].symbol().to_owned()).collect::<String>().trim_end().to_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn on_line(app: &mut App) {
    press(app, "]cj");
    assert!(matches!(app.open.as_ref().unwrap().row(), Some(Row::Line { .. })));
}

pub(super) fn type_text(app: &mut App, text: &str) -> Vec<Action> {
    press(app, text);
    app.handle_key(code(KeyCode::Enter))
}

/// The anchor column of the row under the cursor.
pub(super) fn marker_here(app: &App) -> Option<crate::review::Marker> {
    let open = app.open.as_ref().unwrap();
    open.review.marker_of(&open.review.markers(), open.row()?)
}

pub(super) fn with_saved_draft() -> App {
    let mut app = with_review();
    on_line(&mut app);
    press(&mut app, "c");
    let save = type_text(&mut app, "nit");
    app.apply(saved(&save, 9));
    app
}

/// The forge's answer to the one draft save in `actions`.
pub(super) fn saved(actions: &[Action], id: u64) -> Incoming {
    let [Action::SaveDraft { key, draft }] = actions else { panic!("{actions:?}") };
    Incoming::DraftSaved { key: key.clone(), draft: draft.clone(), id, body: draft.body.clone() }
}

pub(super) fn cells(app: &mut App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| ui::draw(f, app)).unwrap();
    terminal.backend().buffer().clone()
}

pub(super) fn scoped_app() -> App {
    let mut app = App::new(Settings { project: Some("acme/widgets".into()), ..settings() });
    app.today = today();
    app
}

pub(super) fn scoped_sections() -> Sections {
    fixture::queue_in(include_str!("../../forge/gitlab/fixtures/queue_scoped.json"), "acme/widgets").sections(&[])
}

pub(super) fn queue_answer(scope: Option<&str>, sections: Sections, cached: bool) -> Incoming {
    Incoming::Queue { scope: scope.map(str::to_owned), me: "nina".into(), sections, opened: HashMap::new(), cached }
}

/// The cover over the review, its threads replaced by `count` threads on a deep path.
pub(super) fn cover_with_threads(count: usize) -> App {
    let mut app = with_review();
    press(&mut app, "i");
    let threads = (0..count)
        .map(|i| ThreadRow {
            id: format!("t{i}"),
            place: format!("apps/backend/src/domain/integration/slack/useCases/sync/syncSlackUsers.ts:{}", 10 + i),
            short_place: format!("syncSlackUsers.ts:{}", 10 + i),
            author: "nina".into(),
            first_words: "should this retry on timeout, the provider says keys are safe to reuse".into(),
            replies: i,
        })
        .collect::<Vec<_>>();
    app.brief = app.brief.take().map(|b| Brief { threads: Some(threads), selected: Some(0), ..b });
    app
}

pub(super) fn sum_review() -> Review {
    let file = DiffFile {
        diff: include_str!("../../review/fixtures/sum.diff").to_owned(),
        old_path: "src/sum.rs".into(),
        new_path: "src/sum.rs".into(),
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

pub(super) fn with_sum_review() -> App {
    let mut app = with_queue();
    app.queue_move(0);
    app.handle_key(code(KeyCode::Enter));
    app.apply(Incoming::Review { key: mr_key(), review: Box::new(sum_review()), cached: None });
    app
}

/// Moves the cursor down until it sits on a row `wanted` accepts.
pub(super) fn walk_to(app: &mut App, wanted: impl Fn(&Row) -> bool) {
    for _ in 0..40 {
        if app.open.as_ref().unwrap().row().is_some_and(&wanted) {
            return;
        }
        press(app, "j");
    }
    panic!("no such row");
}

pub(super) fn on_pair(app: &mut App) {
    walk_to(app, |r| matches!(r, Row::Pair { .. }));
}

/// The cells holding the first character of `text` on screen.
pub(super) fn cell_of(buffer: &ratatui::buffer::Buffer, text: &str) -> ratatui::buffer::Cell {
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

pub(super) fn ctrl_k() -> KeyEvent {
    KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL)
}

/// Resolves the one unresolved thread on `charge.rs`.
pub(super) fn resolve_the_diff_note(app: &mut App) {
    let open = app.open.clone().unwrap();
    app.open = Some(Open { review: open.review.with_resolved("9f2c0aa1d4e5b6c7", true), ..open });
}

pub(super) fn asking() -> App {
    let mut app = App::new(Settings { ask: Some("claude-opus-5".into()), ..settings() });
    app.today = today();
    app.apply(Incoming::Queue { scope: None, me: "nina".into(), sections: sections(), opened: HashMap::new(), cached: false });
    app.queue_move(0);
    app.handle_key(code(KeyCode::Enter));
    app.apply(Incoming::Review { key: mr_key(), review: Box::new(review()), cached: None });
    app
}

pub(super) fn the_ask(actions: &[Action]) -> (u64, crate::ai::anthropic::Ask, bool) {
    let [Action::Ask { id, request, fresh, .. }] = actions else { panic!("{actions:?}") };
    (*id, (**request).clone(), *fresh)
}

pub(super) fn done(model: &str) -> crate::ai::anthropic::Outcome {
    crate::ai::anthropic::Outcome {
        stop: crate::ai::anthropic::Stop::Done,
        usage: crate::ai::anthropic::Usage { cache_read: 1200, output: 40, ..Default::default() },
        model: model.into(),
    }
}

pub(super) fn run() -> crate::forge::checks::Checks {
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

pub(super) fn with_pipeline() -> App {
    let mut app = with_review();
    let head = app.open.as_ref().unwrap().review.mr.refs.head.clone();
    assert_eq!(press(&mut app, "p"), vec![Action::LoadChecks { key: mr_key(), head }]);
    app.apply(Incoming::Checks { key: mr_key(), checks: Some(run()) });
    app
}

/// The review with one more thread on added line 13, whose note carries `body` and GitLab's `suggestions`.
pub(super) fn with_suggestion(body: &str, suggestions: serde_json::Value) -> App {
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

pub(super) fn keymap(toml: &str) -> crate::keymap::Keymap {
    let keys: crate::config::Keys = toml::from_str(toml).unwrap();
    crate::keymap::Keymap::new(&keys).unwrap()
}

/// The review with one more thread on added line 13 whose note carries `body`, its pane open.
pub(super) fn with_note(body: &str) -> App {
    with_suggestion(body, json!([]))
}

/// The scoped queue with a three-MR PDF chain by `romain.courtois` added to Open.
pub(super) fn stacked_sections() -> Sections {
    let mut sections = scoped_sections();
    let seed = sections.open[0].clone();
    let link = |number: u64, title: &str, source: &str, target: &str, pipeline: Option<PipelineStatus>| crate::forge::QueueMr {
        number,
        title: title.into(),
        author: "romain.courtois".into(),
        author_name: "Romain".into(),
        source_branch: source.into(),
        target_branch: target.into(),
        pipeline,
        draft: false,
        created_at: seed.created_at + chrono::TimeDelta::hours(number.try_into().unwrap()),
        web_url: format!("https://gitlab.com/acme/widgets/-/merge_requests/{number}"),
        ..seed.clone()
    };
    sections.open.extend([
        link(61, "feat: read shared PDFs from the drive", "pdf-read", "main", Some(PipelineStatus::Success)),
        link(62, "feat: read shared PDFs page by page", "pdf-pages", "pdf-read", Some(PipelineStatus::Failed)),
        link(63, "feat: read shared PDFs with OCR", "pdf-ocr", "pdf-pages", None),
    ]);
    sections
}

pub(super) fn stacked_app() -> App {
    let mut app = scoped_app();
    app.apply(queue_answer(Some("acme/widgets"), stacked_sections(), false));
    app
}

pub(super) fn stack_row(app: &App) -> usize {
    app.queue_rows().iter().position(|r| matches!(r, QueueRow::Stack { .. })).unwrap()
}

/// The scoped fixture judged by the rules on 23 September: 51 went stale, 42 already has two reviewers.
pub(super) fn ruled_sections() -> Sections {
    use crate::forge::QueueMr;
    use chrono::TimeZone;
    let queue = fixture::queue_in(include_str!("../../forge/gitlab/fixtures/queue_scoped.json"), "acme/widgets");
    let old = Utc.with_ymd_and_hms(2026, 9, 1, 12, 0, 0).unwrap();
    let open: Vec<QueueMr> = queue
        .open
        .iter()
        .cloned()
        .map(|mr| match mr.number {
            51 => QueueMr { updated_at: old, ..mr },
            _ => mr,
        })
        .chain([QueueMr { number: 52, notes: 4, commenters: vec!["omar".into(), "sam".into(), "kim".into()], ..queue.open[0].clone() }])
        .chain([QueueMr { number: 53, author: "kim".into(), ..queue.open[0].clone() }])
        .collect();
    let now = Utc.with_ymd_and_hms(2026, 9, 23, 12, 0, 0).unwrap();
    crate::forge::Queue { open, ..queue }.sections_with(&[], &crate::forge::rules::Rules::default(), now)
}

pub(super) fn share_target(name: Option<&str>) -> crate::share::Target {
    crate::share::Target {
        name: name.map(str::to_owned),
        command: format!("slack send '#{}'", name.unwrap_or("review")),
        template: crate::share::DEFAULT_TEMPLATE.into(),
    }
}

pub(super) fn sharing_queue(targets: Vec<crate::share::Target>) -> App {
    let mut app = with_queue();
    app.queue_move(0);
    app.share_targets = targets;
    app
}

/// Zen on the line with the resolved thread, its pane open and focused.
pub(super) fn zen_on_a_thread() -> App {
    let mut app = with_review();
    press(&mut app, "zz]N");
    app.handle_key(code(KeyCode::Enter));
    assert_eq!(app.focus, Focus::Side);
    app
}

/// One long file (two hunks of 30 added lines) and a short one, so the diff scrolls past a header.
pub(super) fn long_review() -> Review {
    let lines = |from: usize| (from..from + 30).map(|i| format!("+    let step_{i} = {i};\n")).collect::<String>();
    let long = DiffFile {
        diff: format!("@@ -0,0 +1,30 @@ fn charge\n{}@@ -40,0 +71,30 @@ fn refund\n{}", lines(1), lines(71)),
        old_path: "src/pay/charge.rs".into(),
        new_path: "src/pay/charge.rs".into(),
        ..DiffFile::default()
    };
    let short = DiffFile {
        diff: "@@ -1 +1 @@\n-a\n+b\n".into(),
        old_path: "src/pay/mod.rs".into(),
        new_path: "src/pay/mod.rs".into(),
        ..DiffFile::default()
    };
    Review::new(mr(), &[long, short], vec![], &[])
}

pub(super) fn with_long_review() -> App {
    let mut app = with_review();
    app.apply(Incoming::Review { key: mr_key(), review: Box::new(long_review()), cached: None });
    app.focus = Focus::Review;
    app
}

/// Puts the cursor on the `n`th added line of the long file.
pub(super) fn to_step(app: &mut App, n: usize) {
    let open = app.open.as_ref().unwrap();
    let at = open.rows.iter().enumerate().filter(|(_, r)| matches!(r, Row::Line { file: 0, .. })).nth(n).unwrap().0;
    app.open = Some(Open { selected: at, ..open.clone() });
}

/// The open review, its MR changed by `change`: mine, approved, and so on.
pub(super) fn with_mr(change: impl FnOnce(Mr) -> Mr) -> App {
    let mut app = with_review();
    let review = Review::new(change(mr()), &diffs(), discussions(), &["*.lock".into()]);
    app.apply(Incoming::Review { key: mr_key(), review: Box::new(review), cached: None });
    app
}

pub(super) fn approved_and_mine(mr: Mr) -> Mr {
    let approvals = crate::forge::Approvals { approved: true, approvals_left: 0, ..mr.approvals.clone() };
    Mr { mine: true, approvals, merge: crate::forge::MergePlan { method: crate::forge::MergeMethod::Squash, remove_branch: true }, ..mr }
}

/// The review of `with_long_review`, counting.
pub(super) fn counting() -> App {
    let mut app = with_long_review();
    app.usage = Some(crate::usage::Tally::default());
    app
}

pub(super) fn wheel(app: &mut App, down: bool, column: u16, row: u16) -> Vec<Action> {
    let kind = if down { crossterm::event::MouseEventKind::ScrollDown } else { crossterm::event::MouseEventKind::ScrollUp };
    app.handle_mouse(crossterm::event::MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE })
}

pub(super) fn listed_place(app: &App) -> Option<Place> {
    app.open.as_ref().unwrap().pane.as_ref().map(|p| p.place.clone())
}

pub(super) fn focused_thread(app: &App) -> Option<String> {
    app.open.as_ref().unwrap().focused_thread()
}

pub(super) fn mouse(app: &mut App, kind: crossterm::event::MouseEventKind, (column, row): (u16, u16)) -> Vec<Action> {
    app.handle_mouse(crossterm::event::MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE })
}

/// Draws, then presses the left button on the last cell of the row of `list` with cursor stop `index`, and lets go.
pub(super) fn click_row(app: &mut App, list: super::List, index: usize) -> Vec<Action> {
    use crossterm::event::{MouseButton, MouseEventKind};
    render(app, 160, 30);
    let row =
        app.list_rows.iter().find(|row| row.list == list && row.index == index).unwrap_or_else(|| panic!("{list:?} row {index} on screen"));
    let at = (row.area.right() - 1, row.area.y);
    mouse(app, MouseEventKind::Down(MouseButton::Left), at);
    mouse(app, MouseEventKind::Up(MouseButton::Left), at)
}

/// Presses the left button at `from`, drags to `to` and lets go, drawing between each step as the loop does.
pub(super) fn drag(app: &mut App, from: (u16, u16), to: (u16, u16), width: u16, height: u16) -> Vec<Action> {
    use crossterm::event::{MouseButton, MouseEventKind};
    mouse(app, MouseEventKind::Down(MouseButton::Left), from);
    render(app, width, height);
    mouse(app, MouseEventKind::Drag(MouseButton::Left), to);
    render(app, width, height);
    mouse(app, MouseEventKind::Up(MouseButton::Left), to)
}

/// The cell `text` starts at on screen.
pub(super) fn spot(app: &mut App, text: &str, width: u16, height: u16) -> (u16, u16) {
    let screen = render(app, width, height);
    let found = screen.lines().enumerate().find_map(|(y, row)| row.find(text).map(|i| (row[..i].chars().count(), y)));
    let (x, y) = found.unwrap_or_else(|| panic!("{text} is on screen:\n{screen}"));
    (x as u16, y as u16)
}
