//! The compact layout, for a phone: under `[tui] compact_width` one page shows at a time, full screen. The queue,
//! then the diff, then the right pane. What the wider layouts decide once, it decides again at every frame, so a
//! window that grows or shrinks changes layout without a restart.
use super::App;
use crate::config::QueueLayout;
use crate::review::Place;
use crate::tui::motion::Motion;
use std::time::Duration;

impl App {
    /// The screen moves on its own as little as a phone link likes in the compact layout, freely anywhere else.
    pub fn motion(&self) -> Motion {
        if self.compact { Motion::Calm } else { Motion::Lively }
    }

    /// How long revu has run, which is what every spinner and pulse counts from.
    pub fn elapsed(&self) -> Duration {
        self.now.duration_since(self.started)
    }

    pub fn spinner(&self) -> &'static str {
        self.motion().spinner(self.elapsed())
    }

    /// Long diff lines wrap: what `w` chose, else on in the compact layout, where a cut line hides most of itself.
    pub fn wraps(&self) -> bool {
        self.wrap.unwrap_or(self.compact)
    }

    /// How a queue row reads: two lines in the compact layout whatever `[tui] queue` says, one line leaving a
    /// title no room there.
    pub fn row_layout(&self) -> QueueLayout {
        if self.compact { QueueLayout::Comfortable } else { self.queue_layout }
    }

    /// The number of the line under the diff's cursor, or of the line its thread row hangs under: the new side's,
    /// the old one's on a line the new side does not have.
    pub fn cursor_line(&self) -> Option<u32> {
        let open = self.open.as_ref()?;
        match open.review.place_of(open.row()?)? {
            Place::Line { new, old, .. } => new.or(old),
            Place::Mr | Place::Outdated { .. } | Place::All => None,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crate::config::{QueueLayout, Widths};
    use crate::tui::app::test_support::*;
    use ratatui::layout::Rect;

    /// The phone with its keyboard up.
    const KEYBOARD_UP: (u16, u16) = (50, 14);

    /// The text of the screen row holding `text`.
    fn row_with<'s>(screen: &'s str, text: &str) -> &'s str {
        screen.lines().find(|line| line.contains(text)).unwrap_or_else(|| panic!("{text} on screen:\n{screen}"))
    }

    #[test]
    fn the_layout_follows_the_width_at_every_frame() {
        let mut app = with_review();
        render(&mut app, 69, 30);
        assert!(app.compact && app.narrow, "compact is narrow too: threads and the box read in the diff");
        render(&mut app, 70, 30);
        assert!(!app.compact && app.narrow);
        render(&mut app, 50, 30);
        assert!(app.compact, "and back, with no restart");
    }

    #[test]
    fn the_layout_setting_wins_over_the_width() {
        let mut forced = with_review();
        forced.widths = Widths { compact: u16::MAX, ..Widths::default() };
        render(&mut forced, 200, 40);
        assert!(forced.compact && forced.narrow);
        let mut never = with_review();
        never.widths = Widths { compact: 0, ..Widths::default() };
        render(&mut never, 50, 30);
        assert!(!never.compact);
    }

    #[test]
    fn snapshot_the_queue_page_on_a_phone() {
        let mut app = with_queue();
        insta::assert_snapshot!("compact_queue", phone(&mut app));
    }

    #[test]
    fn the_queue_is_the_whole_page_until_an_mr_opens_and_h_brings_it_back() {
        let mut app = with_review();
        let diff = phone(&mut app);
        assert!(!diff.contains("Queue ·") && diff.contains("src/pay/charge.rs"), "{diff}");
        assert_eq!((app.areas.queue, app.areas.review.width), (Rect::default(), 50));
        press(&mut app, "h");
        let queue = phone(&mut app);
        assert!(queue.contains("Queue ·") && !queue.contains("src/pay/charge.rs"), "{queue}");
        assert_eq!((app.areas.queue.width, app.areas.review), (50, Rect::default()));
        press(&mut app, "l");
        assert!(phone(&mut app).contains("src/pay/charge.rs"));
    }

    #[test]
    fn a_queue_row_takes_two_lines_with_its_marks_under_the_title() {
        let mut app = with_queue();
        app.queue_layout = QueueLayout::Compact;
        let screen = phone(&mut app);
        let lines: Vec<&str> = screen.lines().collect();
        let title = lines.iter().position(|line| line.contains("charge cards at checkout")).expect("the fixture's first MR");
        assert!(!lines[title].contains("!42"), "the title has its line to itself:\n{screen}");
        assert!(lines[title + 1].contains("!42") && lines[title + 1].contains("omar"), "who and which under it:\n{screen}");
        let wide = render(&mut app, 100, 30);
        assert!(row_with(&wide, "!42").contains("feat: charge"), "one line again from 70 columns:\n{wide}");
    }

    #[test]
    fn a_click_and_the_wheel_reach_the_two_line_rows() {
        let mut app = with_queue();
        phone(&mut app);
        let picked = app.queue_selected;
        let other = *app.list_rows.iter().find(|row| row.index != picked).expect("a second MR row");
        assert_eq!(other.area.height, 2);
        let under_the_title = (other.area.x + 4, other.area.y + 1);
        click(&mut app, under_the_title);
        assert_eq!(app.queue_selected, other.index);
        wheel(&mut app, false, 10, 10);
        assert_ne!(app.queue_selected, other.index, "the wheel walks the page under it");
    }

    #[test]
    fn pinning_still_shapes_the_page() {
        let mut app = with_queue();
        press(&mut app, "b");
        let pinned = phone(&mut app);
        assert!(pinned.contains("PINNED · 1") && pinned.contains("TO REVIEW · 0"), "{pinned}");
    }

    #[test]
    fn snapshot_the_diff_page_on_a_phone() {
        let mut app = with_review();
        on_line(&mut app);
        insta::assert_snapshot!("compact_diff", phone(&mut app));
    }

    #[test]
    fn the_gutter_keeps_the_sign_and_the_title_tells_the_cursor_line() {
        let mut app = with_review();
        on_line(&mut app);
        let line = app.cursor_line().expect("a line of code");
        let screen = phone(&mut app);
        let added = row_with(&screen, "let client = Client::with_key");
        assert!(added.contains("+    let client"), "the sign, then the code, no number between:\n{screen}");
        assert!(screen.lines().next().unwrap().contains(&format!(" L{line} ")), "{screen}");
        press(&mut app, "j");
        let next = app.cursor_line().expect("the next line");
        assert!(phone(&mut app).lines().next().unwrap().contains(&format!(" L{next} ")), "it follows the cursor");
        press(&mut app, "gg");
        assert!(!phone(&mut app).lines().next().unwrap().contains(" L"), "no number off a line of code");
        let wide = render(&mut app, 100, 30);
        assert!(!wide.lines().next().unwrap().contains(" L"), "from 70 columns the gutter tells it:\n{wide}");
    }

    #[test]
    fn the_cursor_line_stays_in_the_title_of_a_short_screen() {
        let mut app = with_review();
        on_line(&mut app);
        let line = app.cursor_line().unwrap();
        let screen = render(&mut app, KEYBOARD_UP.0, KEYBOARD_UP.1);
        assert!(screen.lines().next().unwrap().contains(&format!(" L{line} ")), "{screen}");
    }

    #[test]
    fn long_lines_wrap_on_a_phone_until_w_says_otherwise() {
        let mut app = with_review();
        assert!(!app.wraps(), "cut before any frame, as on a wide screen");
        phone(&mut app);
        assert!(app.wraps());
        press(&mut app, "w");
        assert!(!app.wraps() && app.toast.as_ref().is_some_and(|toast| toast.text == "long lines end in …"));
        press(&mut app, "w");
        assert!(app.wraps());
        let mut wide = with_review();
        render(&mut wide, 120, 30);
        assert!(!wide.wraps(), "off by default from 70 columns");
        press(&mut wide, "w");
        phone(&mut wide);
        assert!(wide.wraps(), "what w chose holds in every layout");
    }

    #[test]
    fn a_long_path_keeps_its_folder_and_its_name() {
        let path = "services/payments/src/checkout/cards/charge_card_at_checkout.rs";
        let file = DiffFile { diff: "@@ -1 +1 @@\n-a\n+b\n".into(), old_path: path.into(), new_path: path.into(), ..DiffFile::default() };
        let mut app = with_review();
        app.apply(Incoming::Review { key: mr_key(), review: Box::new(Review::new(mr(), &[file], vec![], &[])), cached: None });
        let screen = phone(&mut app);
        let path = row_with(&screen, "checkout.rs");
        assert!(path.contains("services/") && path.contains('…'), "both ends read:\n{screen}");
        let narrow = render(&mut app, 70, 30);
        assert!(!narrow.contains("checkout.rs"), "from 70 columns the cut stays at the end:\n{narrow}");
    }

    #[test]
    fn snapshot_a_short_screen_drops_the_header_and_the_status_row() {
        let mut app = with_review();
        on_line(&mut app);
        let screen = render(&mut app, KEYBOARD_UP.0, KEYBOARD_UP.1);
        assert!(app.short);
        assert!(!screen.contains("gitlab.com") && !screen.contains("approvals"), "{screen}");
        insta::assert_snapshot!("compact_diff_short", screen);
        render(&mut app, 50, 20);
        assert!(!app.short, "20 rows keep them");
        render(&mut app, 100, 14);
        assert!(!app.short, "and so does a short window from 70 columns");
    }

    #[test]
    fn a_short_screen_keeps_the_row_of_a_prompt_a_search_and_the_filter() {
        let (width, height) = KEYBOARD_UP;
        let mut app = with_review();
        press(&mut app, "/client");
        assert!(render(&mut app, width, height).contains("/client"), "the search stays on its row");
        app.handle_key(code(KeyCode::Esc));
        app.handle_key(code(KeyCode::Esc));
        press(&mut app, "q");
        assert!(render(&mut app, width, height).lines().last().unwrap().contains('q'), "the quit prompt shows");
        let mut queue = with_queue();
        press(&mut queue, "/pay");
        assert!(render(&mut queue, width, height).contains(" / pay"), "the filter keeps its row");
    }

    #[test]
    fn a_short_screen_says_a_toast_on_the_bottom_row_of_its_page() {
        let (width, height) = KEYBOARD_UP;
        let mut app = with_review();
        render(&mut app, width, height);
        press(&mut app, "w");
        let screen = render(&mut app, width, height);
        let page_bottom = usize::from(app.areas.review.bottom()) - 1;
        assert!(screen.lines().nth(page_bottom).unwrap().contains("long lines end in …"), "{screen}");
    }

    #[test]
    fn the_thread_pane_is_a_page_of_its_own() {
        let mut app = with_review();
        press(&mut app, "T");
        assert_eq!(app.focus, Focus::Side);
        phone(&mut app);
        assert_eq!((app.areas.side.width, app.areas.review, app.areas.queue), (50, Rect::default(), Rect::default()));
        press(&mut app, "h");
        phone(&mut app);
        assert_eq!((app.areas.review.width, app.areas.side), (50, Rect::default()));
    }

    #[test]
    fn the_threads_of_a_line_read_in_the_diff_under_it() {
        let mut app = with_review();
        walk_to_any_thread(&mut app, Toward::Next);
        let screen = phone(&mut app);
        assert!(app.open.as_ref().unwrap().review.reads_threads_in_diff());
        assert!(screen.contains("│ ✓ resolved · you"), "{screen}");
    }
}
