//! Who takes the next key or paste: the topmost modal or box open, else the panes.
use super::App;
use vtui::help::Help;

/// What the next key or paste goes to, the topmost first; a modal hides every box below it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Receiver {
    Help(Help),
    Confirm,
    React,
    Compose,
    Share,
    Palette,
    Publish,
    Brief,
    NewsPage,
    Filter,
    Search,
    Panes,
}

impl App {
    pub(super) fn receiver(&self) -> Receiver {
        if let Some(open) = self.help {
            return Receiver::Help(open);
        }
        let open = [
            (self.confirm.is_some(), Receiver::Confirm),
            (self.react.is_some(), Receiver::React),
            (self.input.is_some(), Receiver::Compose),
            (self.sharing.is_some(), Receiver::Share),
            (self.palette.is_some(), Receiver::Palette),
            (self.publish.is_some(), Receiver::Publish),
            (self.brief.is_some(), Receiver::Brief),
            (self.news_page.is_some(), Receiver::NewsPage),
            (self.filtering, Receiver::Filter),
            (self.search.as_ref().is_some_and(|search| search.typing), Receiver::Search),
        ];
        open.into_iter().find_map(|(is_open, receiver)| is_open.then_some(receiver)).unwrap_or(Receiver::Panes)
    }
}
