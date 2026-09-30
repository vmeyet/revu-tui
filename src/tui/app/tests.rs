#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::test_support::*;

#[test]
fn a_first_queue_failure_stays_in_the_pane_until_r_retries() {
    let mut app = app();
    app.now = app.started;
    app.apply(Incoming::Failed { what: Failure::Queue, message: "HTTP 502".into() });
    app.now += Duration::from_secs(5);
    insta::assert_snapshot!("queue_failed", render(&mut app, 100, 14));
    assert_eq!(press(&mut app, "r"), vec![Action::LoadQueue { scope: None, from_cache: false }]);
    assert!(!app.queue_failed, "the skeleton comes back while it loads");
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
    press(&mut app, "]N");
    app.handle_key(code(KeyCode::Enter));
    insta::assert_snapshot!("thread_open", render(&mut app, 120, 24));
}

#[test]
fn snapshot_help_for_the_queue() {
    let mut app = with_queue();
    press(&mut app, "?");
    insta::assert_snapshot!("help_queue", render(&mut app, 160, 45));
}

#[test]
fn snapshot_help_for_the_diff() {
    let mut app = with_review();
    press(&mut app, "?");
    insta::assert_snapshot!("help_diff", render(&mut app, 100, 30));
}

#[test]
fn snapshot_help() {
    let mut app = with_queue();
    press(&mut app, "??");
    insta::assert_snapshot!("help", render(&mut app, 160, 45));
}

#[test]
fn snapshot_help_medium() {
    let mut app = with_review();
    press(&mut app, "??");
    insta::assert_snapshot!("help_medium", render(&mut app, 100, 30));
}

#[test]
fn snapshot_help_scrolls_on_a_small_terminal() {
    let mut app = with_queue();
    press(&mut app, "??jjj");
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
    press(&mut app, "]N");
    app.handle_key(code(KeyCode::Enter));
    press(&mut app, "r");
    let save = type_text(&mut app, "agreed, keys are per card");
    app.apply(saved(&save, 9));
    insta::assert_snapshot!("thread_draft_reply", render(&mut app, 120, 24));
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
fn an_unfocused_box_stays_quieter_than_the_focused_one() {
    let mut app = with_review();
    app.theme = Theme::named("catppuccin").unwrap();
    let buffer = cells(&mut app, 160, 30);
    let corners: Vec<Color> = (0..160).filter(|&x| buffer[(x, 0)].symbol() == "╭").map(|x| buffer[(x, 0)].fg).collect();
    assert_eq!(corners, vec![app.theme.border, app.theme.border_focus], "the queue's box, then the diff's");
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

#[test]
fn a_thread_reads_its_comment_first_and_its_place_second() {
    let mut app = cover_with_threads(2);
    let screen = render(&mut app, 100, 40);
    let comment = screen.lines().position(|l| l.contains("nina · should this retry on timeout")).expect(&screen);
    let place = screen.lines().nth(comment + 1).unwrap();
    assert!(place.contains("syncSlackUsers.ts:10") && !place.contains("useCases"), "the file name only:\n{screen}");
    assert!(screen.contains("syncSlackUsers.ts:11 · 1 reply"), "{screen}");
    let bottom = screen.lines().find(|l| l.contains("enter go there")).expect(&screen);
    assert!(bottom.contains("sync/syncSlackUsers.ts:10"), "the place of the selected thread, its end kept:\n{bottom}");
    let wide = render(&mut app, 160, 40);
    assert!(wide.contains("apps/backend/src/domain/integration/slack/useCases/sync/syncSlackUsers.ts:10"), "whole when it fits");
    insta::assert_snapshot!("cover_threads", screen);
}

#[test]
fn snapshot_description_modal() {
    let mut app = with_review();
    app.open = app.open.clone().map(|o| {
        let mut review = o.review.clone();
        std::sync::Arc::make_mut(&mut review.mr).description =
            "## Why\n\nCards were charged twice.\n\n- retry with `idempotency_key`\n- log the attempt".into();
        o.with_review(review)
    });
    press(&mut app, "i");
    insta::assert_snapshot!("description_modal", render(&mut app, 120, 40));
    insta::assert_snapshot!("description_modal_small", render(&mut app, 80, 24));
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
fn snapshot_review_inline() {
    let mut app = with_sum_review();
    on_pair(&mut app);
    insta::assert_snapshot!("review_inline", render(&mut app, 100, 18));
}

#[test]
fn snapshot_review_side_by_side() {
    let mut app = with_sum_review();
    press(&mut app, "D");
    on_pair(&mut app);
    insta::assert_snapshot!("review_side_by_side", render(&mut app, 200, 18));
}

#[test]
fn snapshot_review_side_by_side_in_a_narrow_window() {
    let mut app = with_sum_review();
    press(&mut app, "D");
    insta::assert_snapshot!("review_side_by_side_narrow", render(&mut app, 100, 18));
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
fn snapshot_palette_in_each_mode() {
    let mut app = with_review();
    press(&mut app, ":pu");
    insta::assert_snapshot!("palette_commands", render(&mut app, 100, 24));
    app.handle_key(code(KeyCode::Esc));
    app.handle_key(ctrl_k());
    press(&mut app, "@omar");
    insta::assert_snapshot!("palette_mrs", render(&mut app, 100, 24));
    app.handle_key(code(KeyCode::Esc));
    app.handle_key(ctrl_k());
    press(&mut app, "/ch");
    insta::assert_snapshot!("palette_files", render(&mut app, 100, 24));
}

#[test]
fn snapshot_file_tree() {
    let mut app = with_review();
    press(&mut app, "]cjzvt");
    insta::assert_snapshot!("file_tree", render(&mut app, 120, 20));
}

#[test]
fn snapshot_wrapped_lines() {
    let mut app = with_review();
    press(&mut app, "w");
    insta::assert_snapshot!("wrapped", render(&mut app, 80, 24));
}

#[test]
fn snapshot_narrow_pane_and_a_three_line_box() {
    let mut app = with_review();
    press(&mut app, "]N");
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

#[test]
fn snapshot_pipeline_pane() {
    let mut app = with_pipeline();
    insta::assert_snapshot!("pipeline", render(&mut app, 150, 20));
}

#[test]
fn snapshot_help_with_the_azerty_preset_and_a_binding() {
    let mut app = with_queue();
    app.keymap = keymap(
        r#"
        layout = "azerty"
        [bind]
        next_thread = "F"
        "#,
    );
    press(&mut app, "??");
    insta::assert_snapshot!("help_azerty", render(&mut app, 160, 45));
}

#[test]
fn snapshot_thread_with_a_picture_the_terminal_cannot_draw() {
    let mut app = with_note("Look:\n![the chart](/uploads/0123456789abcdef0123456789abcdef/chart.png)\nthanks");
    let screen = render(&mut app, 150, 24);
    assert!(screen.contains("[image: the chart]"), "{screen}");
    let link = app.links.iter().find(|l| l.text == "[image: the chart]").expect("the line links to the picture");
    assert_eq!(link.url, "https://gitlab.com/acme/widgets/uploads/0123456789abcdef0123456789abcdef/chart.png");
    insta::assert_snapshot!("thread_picture_fallback", screen);
}

#[test]
fn a_ready_picture_is_painted_in_the_pane_and_hidden_under_a_modal() {
    let mut app = with_note("Look:\n![chart](/uploads/0123456789abcdef0123456789abcdef/chart.png)");
    app.thumbs = crate::tui::images::tests::test_thumbs();
    let url = "/uploads/0123456789abcdef0123456789abcdef/chart.png".to_owned();
    let gradient = image::RgbImage::from_fn(200, 100, |_, y| image::Rgb([y as u8 * 2, y as u8 * 2, y as u8 * 2]));
    app.apply(Incoming::Image { url, image: Some(image::DynamicImage::ImageRgb8(gradient)) });
    let painted = |screen: &str| screen.lines().filter(|l| l.contains('▀') || l.contains('▄')).count();
    assert_eq!(painted(&render(&mut app, 150, 30)), 5, "200x100 at 10x20 cells is 20x5");
    press(&mut app, "?");
    assert_eq!(painted(&render(&mut app, 150, 30)), 0, "a modal covers no picture");
}

#[test]
fn a_picture_arriving_after_the_pane_was_drawn_is_painted_on_the_next_frame() {
    let mut app = with_note("Look:\n![chart](/uploads/0123456789abcdef0123456789abcdef/chart.png)");
    app.thumbs = crate::tui::images::tests::test_thumbs();
    let url = "/uploads/0123456789abcdef0123456789abcdef/chart.png".to_owned();
    app.thumbs.wanted([url.clone()]);
    assert!(render(&mut app, 150, 30).contains("… loading image"));
    app.apply(Incoming::Image { url, image: Some(image::DynamicImage::new_rgb8(200, 100)) });
    let screen = render(&mut app, 150, 30);
    assert!(!screen.contains("… loading image"), "the layout of the last frame is not kept: {screen}");
}

#[test]
fn snapshot_queue_compact() {
    let mut app = with_queue();
    app.queue_layout = crate::config::QueueLayout::Compact;
    app.now = app.started;
    insta::assert_snapshot!("queue_compact", render(&mut app, 100, 16));
}

#[test]
fn snapshot_queue_drafts_open_and_grouped_by_author() {
    let mut app = scoped_app();
    app.apply(queue_answer(Some("acme/widgets"), scoped_sections(), false));
    press(&mut app, "Gkzo");
    press(&mut app, "S");
    insta::assert_snapshot!("queue_drafts_grouped", render(&mut app, 120, 24));
}

#[test]
fn snapshot_queue_with_a_stack_at_120_and_170_columns() {
    let mut app = stacked_app();
    insta::assert_snapshot!("queue_stack_120", render(&mut app, 120, 26));
    insta::assert_snapshot!("queue_stack_170", render(&mut app, 170, 26));
    app.queue_selected = stack_row(&app);
    app.handle_key(code(KeyCode::Enter));
    insta::assert_snapshot!("queue_stack_open_170", render(&mut app, 170, 30));
    app.queue_layout = crate::config::QueueLayout::Compact;
    insta::assert_snapshot!("queue_stack_compact_120", render(&mut app, 120, 20));
    insta::assert_snapshot!("queue_stack_compact_170", render(&mut app, 170, 20));
}

#[test]
fn snapshot_queue_with_other() {
    let mut app = scoped_app();
    app.apply(queue_answer(Some("acme/widgets"), ruled_sections(), false));
    insta::assert_snapshot!("queue_with_other", render(&mut app, 120, 30));
}

#[test]
fn snapshot_share_preview() {
    let mut app = sharing_queue(vec![share_target(Some("review"))]);
    press(&mut app, "Y");
    type_text(&mut app, "needs a second pair of eyes");
    insta::assert_snapshot!("share_preview", render(&mut app, 120, 24));
}

#[test]
fn zen_draws_no_frames_no_status_line_and_a_centred_column() {
    let mut app = with_review();
    press(&mut app, "zz");
    let screen = render(&mut app, 160, 45);
    assert!(!screen.contains('╭') && !screen.contains("Queue") && !screen.contains("gitlab.com ·"), "{screen}");
    assert!(app.links.is_empty(), "no link is printed over zen's one-line header: {:?}", app.links);
    let first = screen.lines().find(|l| !l.trim().is_empty()).unwrap();
    assert!(first.trim_start().starts_with("!42 feat: charge cards at checkout"), "{first}");
    let indent = first.len() - first.trim_start().len();
    assert_eq!(indent, (160 - 120) / 2 + 1, "120 columns at least in the middle, then its padding");
    insta::assert_snapshot!("zen_wide", screen);
    insta::assert_snapshot!("zen_medium", render(&mut app, 100, 30));
}

#[test]
fn zen_opens_the_thread_pane_under_the_diff_behind_a_rule() {
    let mut app = zen_on_a_thread();
    let screen = render(&mut app, 138, 40);
    assert!(!screen.contains('╭') && !screen.contains('╰'), "no frame:\n{screen}");
    assert!(screen.contains("▎✓   13      -    let client = Client::new();"), "the diff keeps its cursor line:\n{screen}");
    let rows = screen.lines().collect::<Vec<_>>();
    let rule = rows.iter().position(|row| row.trim_start().starts_with("─ charge.rs:-13 · 1 thread ──")).expect("a rule over the pane");
    assert!(rows[rule + 3].contains("Why drop the plain client?"), "the thread right under it:\n{screen}");
    let (diff, pane) = (app.areas.review, app.areas.side);
    assert_eq!((diff.x, diff.width, diff.bottom()), (pane.x, pane.width, pane.y), "one column, the pane right under the diff");
    assert_eq!(pane.height, 4, "no more rows than the thread takes");
    insta::assert_snapshot!("zen_split", screen);
}

#[test]
fn a_short_zen_column_opens_the_thread_pane_as_a_page_of_its_own() {
    let mut app = zen_on_a_thread();
    let screen = render(&mut app, 100, 24);
    assert!(!screen.contains("let client = Client::new()") && screen.contains("charge.rs:-13 · 1 thread"), "{screen}");
    let first = screen.lines().find(|l| !l.trim().is_empty()).unwrap();
    assert!(first.starts_with(" charge.rs:-13"), "the title starts where the zen header does:\n{screen}");
    assert!(!app.areas.both_shown());
    insta::assert_snapshot!("zen_thread_short", screen);
}

#[test]
fn in_zen_the_compose_box_lets_the_pane_under_the_diff_grow() {
    let mut app = zen_on_a_thread();
    render(&mut app, 138, 40);
    let reading = app.areas.side.height;
    press(&mut app, "r");
    for line in ["agreed", "keys are per card", "and per amount"] {
        press(&mut app, line);
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
    }
    let screen = render(&mut app, 138, 40);
    assert!(screen.contains("reply to nina") && screen.contains("agreed") && screen.contains("and per amount"), "{screen}");
    assert_eq!((reading, app.areas.side.height), (4, 10), "past the quarter: the thread and a box of four lines");
    insta::assert_snapshot!("zen_split_compose", screen);
    press(&mut app, &"one more line ".repeat(60));
    render(&mut app, 138, 40);
    assert_eq!(app.areas.side.height, 1 + 3 + 8 + 2, "the box stops at eight lines");
    let mut list = with_review();
    press(&mut list, "zzTragreed");
    render(&mut list, 138, 40);
    assert_eq!(list.areas.side.height, 39 / 2, "every thread and a box: half the column, no more");
}

#[test]
fn a_long_file_keeps_its_name_and_hunk_pinned_above_the_diff() {
    let mut app = with_long_review();
    to_step(&mut app, 55);
    let screen = render(&mut app, 120, 30);
    let open = app.open.as_ref().unwrap();
    assert!(open.pinned_file.is_some(), "{screen}");
    let body: Vec<&str> = screen.lines().skip_while(|l| !l.contains("charge.rs")).collect();
    assert!(body.first().is_some_and(|l| l.contains("src/pay/charge.rs")), "the file is the first diff row:\n{screen}");
    assert!(body.get(1).is_some_and(|l| l.contains("fn refund")), "the cursor's hunk follows it:\n{screen}");
    insta::assert_snapshot!("review_pinned", screen);
}

#[test]
fn zen_pins_the_file_and_hunk_inside_its_column() {
    let mut app = with_long_review();
    to_step(&mut app, 55);
    press(&mut app, "zz");
    let screen = render(&mut app, 160, 45);
    assert!(app.open.as_ref().unwrap().pinned_file.is_some(), "{screen}");
    let pinned = screen.lines().find(|l| l.contains("src/pay/charge.rs")).expect(&screen);
    let indent = pinned.len() - pinned.trim_start().len();
    assert!(indent >= (160 - 120) / 2, "inside the centred column:\n{screen}");
    assert!(screen.contains("fn refund"), "the cursor's hunk too:\n{screen}");
    insta::assert_snapshot!("zen_pinned", screen);
    let _ = render(&mut app, 160, 18);
    assert_eq!(app.open.as_ref().unwrap().pinned_file, None, "short screens still pin nothing, in zen too");
}

#[test]
fn a_short_view_pins_nothing() {
    let mut app = with_long_review();
    to_step(&mut app, 40);
    let screen = render(&mut app, 120, 18);
    assert_eq!(app.open.as_ref().unwrap().pinned_file, None);
    insta::assert_snapshot!("review_pinned_hidden", screen);
}

#[test]
fn progress_shows_in_the_header_and_on_the_queue_row() {
    let mut app = with_review();
    let started = crate::review::Progress { viewed: 1, files: 3, folded: 0 };
    app.apply(Incoming::Progress(HashMap::from([(MrKey::new("acme/widgets", 41), started)])));
    press(&mut app, "zv");
    assert_eq!(app.progress_of(&mr_key()).map(|p| p.viewed), Some(1), "the open MR counts what I just marked");
    let screen = render(&mut app, 160, 24);
    assert!(screen.contains("viewed 1/1 · 1 folded ━"), "the lock file waits apart: {screen}");
    assert!(screen.contains("41 · 1/3"), "{screen}");
    insta::assert_snapshot!("review_progress", screen);
}

#[test]
fn snapshot_every_thread() {
    let mut app = with_review();
    press(&mut app, "T");
    let wide = render(&mut app, 160, 30);
    assert!(wide.contains("src/pay/charge.rs:-13") && wide.contains("let client = Client::new();"), "{wide}");
    insta::assert_snapshot!("every_thread_wide", wide);
    press(&mut app, "Tzz");
    press(&mut app, "T");
    insta::assert_snapshot!("every_thread_zen", render(&mut app, 160, 30));
}

/// The screen with each run of reversed cells between `⟦` and `⟧`.
fn render_selected(app: &mut App, width: u16, height: u16) -> String {
    let buffer = cells(app, width, height);
    let reversed = |x: u16, y: u16| x < width && buffer[(x, y)].modifier.contains(ratatui::style::Modifier::REVERSED);
    let row = |y: u16| {
        let mut row = String::new();
        for x in 0..width {
            if reversed(x, y) && (x == 0 || !reversed(x - 1, y)) {
                row.push('⟦');
            }
            row.push_str(buffer[(x, y)].symbol());
            if reversed(x, y) && !reversed(x + 1, y) {
                row.push('⟧');
            }
        }
        row.trim_end().to_owned()
    };
    (0..height).map(row).collect::<Vec<_>>().join("\n")
}

#[test]
fn a_drag_in_the_diff_copies_only_the_code_text_and_lights_it() {
    let mut app = with_review();
    let (x, y) = spot(&mut app, "let client = Client::new()", 120, 24);
    let actions = drag(&mut app, (x, y), (x + 9, y + 1), 120, 24);
    let text = "let client = Client::new();\n    let client".to_owned();
    assert_eq!(actions, vec![Action::Copy { text, done: "copied 2 lines".into() }]);
    insta::assert_snapshot!("diff_selection", render_selected(&mut app, 120, 24));
    press(&mut app, "j");
    assert!(!render_selected(&mut app, 120, 24).contains('⟦'), "the next key clears the highlight");
}

#[test]
fn a_drag_in_the_thread_pane_copies_the_note_as_written_without_its_header() {
    let mut app = with_review();
    press(&mut app, "]N");
    app.handle_key(code(KeyCode::Enter));
    let open = app.open.as_mut().unwrap();
    let mut threads = open.review.threads.to_vec();
    let thread = threads.iter_mut().find(|t| t.notes[0].body == "Why drop the plain client?").unwrap();
    thread.notes[0].body = "Why drop the `plain` client?\n- keep `Client::new`".into();
    open.review.threads = threads.into();
    let (x, y) = spot(&mut app, "plain", 120, 24);
    let word = drag(&mut app, (x, y), (x + 4, y), 120, 24);
    assert_eq!(word, vec![Action::Copy { text: "plain".into(), done: "copied 1 line".into() }]);
    let (x, y) = spot(&mut app, "Why drop", 120, 24);
    let actions = drag(&mut app, (x, y), (x + 60, y + 5), 120, 24);
    let text = "Why drop the `plain` client?\n- keep `Client::new`".to_owned();
    assert_eq!(actions, vec![Action::Copy { text, done: "copied 2 lines".into() }]);
    insta::assert_snapshot!("thread_selection", render_selected(&mut app, 120, 24));
}

#[test]
fn in_a_short_zen_column_enter_in_every_thread_shows_the_diff_and_l_or_t_bring_the_list_back() {
    let mut app = with_review();
    press(&mut app, "zzT");
    assert_eq!((listed_place(&app), app.focus), (Some(Place::All), Focus::Side));
    let list = render(&mut app, 138, 24);
    assert!(list.contains("the whole MR · 3 threads") && !list.contains("@@ -12,4"), "the list takes the diff's column:\n{list}");
    press(&mut app, "JJ");
    app.handle_key(code(KeyCode::Enter));
    let open = app.open.as_ref().unwrap();
    assert_eq!(open.row().and_then(|row| open.review.place_of(row)), Some(Place::Line { file: 0, new: None, old: Some(13) }));
    assert_eq!((listed_place(&app), app.focus, app.zen), (Some(Place::All), Focus::Review, true), "the diff shows the line, in zen");
    assert!(render(&mut app, 138, 24).contains("▎✓   13      -    let client = Client::new();"));
    press(&mut app, "l");
    assert_eq!((listed_place(&app), app.focus), (Some(Place::All), Focus::Side), "l on the marked line goes back to the list");
    assert_eq!(focused_thread(&app).as_deref(), Some("c0ffee00c0ffee00"), "on the thread it left");
    insta::assert_snapshot!("every_thread_zen_back", render(&mut app, 138, 24));
    press(&mut app, "hT");
    assert_eq!((listed_place(&app), app.focus), (Some(Place::All), Focus::Side), "T from the diff shows the hidden list");
    press(&mut app, "T");
    assert_eq!((listed_place(&app), app.focus, app.zen), (None, Focus::Review, true), "T on the list closes it");
}

#[test]
fn snapshot_every_thread_mine() {
    let mut app = with_review();
    press(&mut app, "zzTm");
    let mine = render(&mut app, 200, 50);
    let pane = mine.split("─ the whole MR · mine · 1 of 3 threads").nth(1).unwrap_or_else(|| panic!("the title on the rule:\n{mine}"));
    assert!(!pane.contains("on the MR"), "{mine}");
    insta::assert_snapshot!("every_thread_zen_mine", mine);
    app.me = "omar".into();
    press(&mut app, "mm");
    let none = render(&mut app, 100, 12);
    assert!(none.contains("the whole MR · mine · 0 of 3 threads") && none.contains("you take part in no thread · m shows them all"));
    insta::assert_snapshot!("every_thread_mine_empty", none);
}
