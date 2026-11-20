//! `e` in the queue and `:title`: a new title for my own open MR, typed in the palette.
use super::{Action, App, Focus};
use crate::forge::{MrKey, Readiness, Title};
use crate::tui::palette::{Mode, Palette};

/// The MR a new title goes to, as it stood when the title was asked for.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TitleTarget {
    key: MrKey,
    /// Without the draft marker GitLab writes in it, as the reader edits it.
    title: String,
    readiness: Readiness,
}

impl App {
    /// `e`: the palette on `:title` and the MR's title, to edit in place; `enter` sends it to this MR.
    pub(super) fn ask_title(&mut self) {
        let Some(target) = self.title_target() else { return };
        let mut palette = Palette::new(Mode::Commands, self.palette_history.clone());
        format!("title {}", target.title).chars().for_each(|c| palette.type_char(c));
        self.palette = Some(palette);
        self.title_target = Some(target);
    }

    /// `:title …`: the forge gives the MR `e` picked, else the one under the cursor, its new title.
    pub(super) fn set_title(&mut self, title: Title) -> Vec<Action> {
        let Some(target) = self.title_target.take().or_else(|| self.title_target()) else { return vec![] };
        if self.hosts.kind_of(&target.key).bare_title(title.as_str()) == target.title {
            self.toast("title unchanged");
            return vec![];
        }
        vec![Action::SetTitle { key: target.key, title, readiness: target.readiness }]
    }

    /// The forge took it: say so, and read the MR and the queue again.
    pub(super) fn title_set(&mut self, key: &MrKey) -> Vec<Action> {
        self.toast(format!("{} has its new title", self.hosts.reference(key)));
        self.reread(key)
    }

    /// The MR under the cursor when it is mine to retitle; else says why not.
    fn title_target(&mut self) -> Option<TitleTarget> {
        let Some((target, refusal)) = self.title_target_under_cursor() else {
            self.toast("select an MR first");
            return None;
        };
        if let Some(reason) = refusal {
            self.cannot_change(&target.key, &reason);
            return None;
        }
        Some(target)
    }

    /// The open MR when the review has the focus, else the queue's row, with why I may not change it.
    fn title_target_under_cursor(&self) -> Option<(TitleTarget, Option<String>)> {
        if let (Some(open), Focus::Review | Focus::Side) = (&self.open, self.focus) {
            let (key, mr) = (&open.key, &open.review.mr);
            let title = self.hosts.kind_of(key).bare_title(&mr.title).to_owned();
            return Some((TitleTarget { key: key.clone(), title, readiness: mr.readiness() }, mr.change_refusal()));
        }
        let mr = self.selected_mr()?;
        let key = mr.key();
        let title = self.hosts.kind_of(&key).bare_title(&mr.title).to_owned();
        let refusal = mr.change_refusal(self.me.on_row(mr));
        Some((TitleTarget { key, title, readiness: mr.readiness() }, refusal))
    }
}

#[cfg(test)]
mod tests {
    use crate::tui::app::test_support::*;

    /// The queue, read by omar, the author of its first row: !42, "feat: charge cards at checkout".
    fn as_author() -> App {
        let mut app = with_queue();
        app.me = "omar".into();
        app
    }

    /// Replaces the title `e` offered in the palette with `title`.
    fn retype(app: &mut App, title: &str) {
        let offered = app.palette.as_ref().unwrap().input().len() - "title ".len();
        (0..offered).for_each(|_| _ = app.handle_key(code(KeyCode::Backspace)));
        press(app, title);
    }

    #[test]
    fn e_on_my_mr_offers_its_title_in_the_palette() {
        let mut app = as_author();
        assert_eq!(press(&mut app, "e"), vec![]);
        assert_eq!(app.palette.as_ref().unwrap().input(), "title feat: charge cards at checkout");
    }

    #[test]
    fn enter_on_a_new_title_sends_it_with_the_readiness_of_the_mr() {
        let mut app = as_author();
        press(&mut app, "e");
        retype(&mut app, "Count the widgets");
        let title = Title::parse("Count the widgets").unwrap();
        assert_eq!(app.handle_key(code(KeyCode::Enter)), vec![Action::SetTitle { key: mr_key(), title, readiness: Readiness::Ready }]);
    }

    #[test]
    fn enter_on_the_offered_title_writes_nothing() {
        let mut app = as_author();
        press(&mut app, "e");
        assert_eq!(app.handle_key(code(KeyCode::Enter)), vec![]);
        assert_eq!(app.live_toast().unwrap().text, "title unchanged");
    }

    #[test]
    fn a_queue_that_moves_under_the_palette_keeps_the_mr_e_picked() {
        let mut app = as_author();
        press(&mut app, "e");
        let mut moved = sections();
        moved.to_review.retain(|mr| mr.number != 42);
        app.apply(queue_answer(None, moved, false));
        assert_ne!(app.selected_mr().map(|mr| mr.number), Some(42), "the cursor stands on another MR");
        retype(&mut app, "Count the widgets");
        let sent = app.handle_key(code(KeyCode::Enter));
        assert!(matches!(sent.as_slice(), [Action::SetTitle { key, .. }] if *key == mr_key()), "{sent:?}");
    }

    #[test]
    fn a_draft_is_offered_without_its_marker_and_stays_a_draft() {
        let mut app = with_queue();
        let mut drafted = sections();
        for mr in drafted.to_review.iter_mut().filter(|mr| mr.number == 42) {
            *mr = QueueMr { title: "Draft: Count the widgets".into(), draft: true, ..mr.clone() };
        }
        app.apply(Incoming::Queue { scope: None, me: "omar".into(), sections: drafted, opened: HashMap::new(), cached: false });
        press(&mut app, "e");
        assert_eq!(app.palette.as_ref().unwrap().input(), "title Count the widgets");
        retype(&mut app, "Count every widget");
        let sent = app.handle_key(code(KeyCode::Enter));
        assert!(matches!(sent.as_slice(), [Action::SetTitle { readiness: Readiness::Draft, .. }]), "{sent:?}");
    }

    #[test]
    fn a_title_the_forge_took_is_told_and_read_again() {
        let mut app = as_author();
        app.apply(Incoming::TitleSet { key: mr_key() });
        let follow = app.take_actions();
        assert!(follow.contains(&Action::RefreshMr(mr_key())), "{follow:?}");
        assert!(follow.iter().any(|a| matches!(a, Action::LoadQueue { .. })), "the queue row reads again: {follow:?}");
        assert_eq!(app.live_toast().unwrap().text, "!42 has its new title");
    }

    #[test]
    fn a_title_the_forge_refused_names_the_mr() {
        let mut app = as_author();
        app.apply(Incoming::Failed { what: Failure::SetTitle { key: mr_key() }, message: "403 Forbidden".into() });
        assert_eq!(app.live_toast().unwrap().text, "title of !42 not changed: 403 Forbidden");
    }

    #[test]
    fn e_says_why_on_an_mr_that_is_not_mine() {
        let mut app = with_queue();
        assert_eq!(press(&mut app, "e"), vec![]);
        assert!(app.palette.is_none());
        assert_eq!(app.live_toast().unwrap().text, "cannot change !42: it is not yours");
    }

    #[test]
    fn the_title_command_in_the_review_goes_to_my_open_mr() {
        let mut app = with_mr(|mr| Mr { mine: true, ..mr });
        press(&mut app, ":title Count the widgets");
        let title = Title::parse("Count the widgets").unwrap();
        assert_eq!(app.handle_key(code(KeyCode::Enter)), vec![Action::SetTitle { key: mr_key(), title, readiness: Readiness::Ready }]);
    }

    #[test]
    fn the_title_command_in_the_review_says_why_on_a_closed_mr() {
        let mut app = with_mr(|mr| Mr { mine: true, state: crate::forge::MrState::Closed, ..mr });
        press(&mut app, ":title Count the widgets");
        assert_eq!(app.handle_key(code(KeyCode::Enter)), vec![]);
        assert_eq!(app.live_toast().unwrap().text, "cannot change !42: this MR is closed");
    }

    #[test]
    fn an_empty_title_never_reaches_the_forge() {
        let mut app = as_author();
        press(&mut app, ":title ");
        assert_eq!(app.handle_key(code(KeyCode::Enter)), vec![]);
        assert_eq!(app.live_toast().unwrap().text, ":title needs the new title");
    }
}
