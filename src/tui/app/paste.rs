//! A paste arrives whole, not key by key: it lands as one edit in the box that has the keys,
//! so a pasted newline is text and never `enter`.
use super::App;
use super::receiver::Receiver;
use vtui::field::Field;

impl App {
    /// The box that would take the next key takes the paste; anywhere else it does nothing.
    pub(crate) fn handle_paste(&mut self, text: &str) {
        self.count_input();
        match self.receiver() {
            Receiver::Compose => self.buffer.paste(text),
            Receiver::Share => self.paste_share_note(text),
            Receiver::Palette => {
                if let Some(palette) = &mut self.palette {
                    palette.type_str(text);
                }
            }
            Receiver::Filter => {
                self.filter.push_str(&one_line(text));
                self.view = None;
                self.queue_settle();
            }
            Receiver::Search => self.paste_search(&one_line(text)),
            Receiver::Help(_)
            | Receiver::Confirm
            | Receiver::React
            | Receiver::Publish
            | Receiver::Brief
            | Receiver::NewsPage
            | Receiver::Panes => {}
        }
    }
}

/// A paste cleaned as the compose box cleans it, its lines joined by a space for a box of one line.
fn one_line(text: &str) -> String {
    let mut field = Field::default();
    field.paste(text);
    field.text().replace('\n', " ").trim().to_owned()
}

#[cfg(test)]
mod tests {
    use crate::tui::app::share::Stage as ShareStage;
    use crate::tui::app::test_support::*;

    #[test]
    fn a_paste_in_the_compose_box_keeps_its_newlines_and_sends_nothing() {
        let mut app = with_review();
        on_line(&mut app);
        press(&mut app, "c");
        app.handle_paste("first line\r\nsecond\tline");
        assert!(app.input.is_some(), "a pasted newline is not enter");
        assert_eq!(app.buffer.text(), "first line\nsecond    line");
    }

    #[test]
    fn a_paste_in_the_share_note_lands_in_the_note() {
        let mut app = sharing_queue(vec![share_target(None)]);
        press(&mut app, "Y");
        app.handle_paste("needs eyes\n");
        let Some(ShareStage::Note { note, .. }) = app.sharing.as_ref().map(|s| &s.stage) else { panic!("no note") };
        assert_eq!(note.text(), "needs eyes\n");
    }

    #[test]
    fn a_paste_in_the_palette_types_on_its_line() {
        let mut app = with_queue();
        app.handle_key(ctrl_k());
        app.handle_paste("go acme/widgets!42\n");
        assert_eq!(app.palette.as_ref().map(|p| p.input.as_str()), Some("go acme/widgets!42"));
    }

    #[test]
    fn a_paste_in_the_queue_filter_narrows_on_one_line() {
        let mut app = with_queue();
        press(&mut app, "/");
        app.handle_paste("runner\n");
        assert!(app.filtering);
        assert_eq!(app.filter, "runner");
        assert_eq!(app.selected_mr().map(|m| m.number), Some(35));
    }

    #[test]
    fn a_paste_in_the_diff_search_joins_its_lines() {
        let mut app = with_review();
        press(&mut app, "/");
        app.handle_paste("charge\r\nclient");
        assert_eq!(app.search.as_ref().map(|s| (s.query.as_str(), s.typing)), Some(("charge client", true)));
    }

    #[test]
    fn a_paste_with_no_box_open_changes_nothing() {
        let mut app = with_queue();
        app.handle_paste("jjjq");
        assert!(!app.should_quit && app.filter.is_empty() && app.palette.is_none());
    }

    #[test]
    fn a_paste_under_the_help_reaches_no_box_below() {
        let mut app = with_queue();
        press(&mut app, "/");
        app.help = Some(Help::default());
        app.handle_paste("runner");
        assert_eq!(app.filter, "");
    }
}
