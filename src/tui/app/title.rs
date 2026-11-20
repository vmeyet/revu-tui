//! `e` in the queue and `:title`: a new title for my own open MR, typed in the palette.
use super::{Action, App, Focus};
use crate::forge::{MrKey, Title};
use crate::tui::palette::{Mode, Palette};

/// The MR a new title goes to, and the title it has now.
struct Retitled {
    key: MrKey,
    title: String,
}

impl App {
    /// `e`: the palette on `:title` and the MR's title, to edit in place.
    pub(super) fn ask_title(&mut self) {
        let Some(Retitled { title, .. }) = self.retitled() else { return };
        let mut palette = Palette::new(Mode::Commands, self.palette_history.clone());
        format!("title {title}").chars().for_each(|c| palette.type_char(c));
        self.palette = Some(palette);
    }

    /// `:title …`: the forge gives the MR under the cursor its new title.
    pub(super) fn set_title(&mut self, title: Title) -> Vec<Action> {
        self.retitled().map(|Retitled { key, .. }| vec![Action::SetTitle { key, title }]).unwrap_or_default()
    }

    /// The forge took it: say so, and read the MR and the queue again.
    pub(super) fn title_set(&mut self, key: &MrKey) -> Vec<Action> {
        self.toast(format!("{}{} has its new title", self.hosts.kind_of(key).sigil(), key.number));
        self.reread(key)
    }

    /// The MR under the cursor when it is mine to retitle; else says why not.
    fn retitled(&mut self) -> Option<Retitled> {
        let Some((retitled, refusal)) = self.under_cursor() else {
            self.toast("select an MR first");
            return None;
        };
        if let Some(reason) = refusal {
            self.warn(format!("cannot change {}{}: {reason}", self.hosts.kind_of(&retitled.key).sigil(), retitled.key.number));
            return None;
        }
        Some(retitled)
    }

    /// The open MR when the review has the focus, else the queue's row, with why I may not change it.
    /// A queue row is always open: the queue holds no other.
    fn under_cursor(&self) -> Option<(Retitled, Option<String>)> {
        if let (Some(open), Focus::Review | Focus::Side) = (&self.open, self.focus) {
            let mr = &open.review.mr;
            return Some((Retitled { key: open.key.clone(), title: mr.title.clone() }, mr.change_refusal()));
        }
        let mr = self.selected_mr()?;
        let mine = self.me.on_row(mr).is(&mr.author);
        Some((Retitled { key: mr.key(), title: mr.title.clone() }, (!mine).then(|| "it is not yours".to_owned())))
    }
}

#[cfg(test)]
mod tests {
    use crate::tui::app::test_support::*;

    /// The queue, read by the author of its first row, !42.
    fn as_author() -> App {
        let mut app = with_queue();
        app.me = "omar".into();
        app
    }

    #[test]
    fn e_on_my_mr_offers_its_title_and_enter_sets_it() {
        let mut app = as_author();
        assert_eq!(press(&mut app, "e"), vec![]);
        assert_eq!(app.palette.as_ref().unwrap().input(), "title feat: charge cards at checkout");
        let title = Title::parse("feat: charge cards at checkout").unwrap();
        assert_eq!(app.handle_key(code(KeyCode::Enter)), vec![Action::SetTitle { key: mr_key(), title }]);
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
        assert_eq!(app.handle_key(code(KeyCode::Enter)), vec![Action::SetTitle { key: mr_key(), title }]);
    }

    #[test]
    fn an_empty_title_never_reaches_the_forge() {
        let mut app = as_author();
        press(&mut app, ":title ");
        assert_eq!(app.handle_key(code(KeyCode::Enter)), vec![]);
        assert_eq!(app.live_toast().unwrap().text, ":title needs the new title");
    }
}
