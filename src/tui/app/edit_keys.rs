//! The editing keys every box of several lines shares: the compose box and the share note read them here,
//! so a key that works in one works in the other.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use vtui::field::Field;

/// `field` once `key` has edited it; a key that edits nothing gives it back as it was.
pub(super) fn field_after_key(mut field: Field, key: KeyEvent) -> Field {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        _ if adds_a_line(key) => field.insert('\n'),
        KeyCode::Left | KeyCode::Char('b') if alt => field.word_left(),
        KeyCode::Right | KeyCode::Char('f') if alt => field.word_right(),
        KeyCode::Backspace if alt => field.delete_word(),
        KeyCode::Left => field.left(),
        KeyCode::Right => field.right(),
        KeyCode::Up => field.up(),
        KeyCode::Down => field.down(),
        KeyCode::Home => field.start(),
        KeyCode::End => field.end(),
        KeyCode::Char('a') if ctrl => field.start(),
        KeyCode::Char('e') if ctrl => field.end(),
        KeyCode::Char('w') if ctrl => field.delete_word(),
        KeyCode::Backspace => field.backspace(),
        KeyCode::Delete => field.delete(),
        KeyCode::Char(c) if !ctrl => field.insert(c),
        _ => {}
    }
    field
}

/// `alt-enter`, `shift-enter` and `ctrl-j` start a new line, so a box tells them from the `enter` that sends.
pub(super) fn adds_a_line(key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Enter => key.modifiers.intersects(KeyModifiers::ALT | KeyModifiers::SHIFT),
        KeyCode::Char('j') => key.modifiers.contains(KeyModifiers::CONTROL),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::test_support::{alt, code, ctrl, key};

    fn text_after(text: &str, keys: &[KeyEvent]) -> String {
        keys.iter().fold(Field::new(text), |field, key| field_after_key(field, *key)).text().to_owned()
    }

    #[test]
    fn alt_enter_shift_enter_and_ctrl_j_each_add_a_line() {
        let shift_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT);
        for adds_line in [alt(KeyCode::Enter), shift_enter, ctrl('j')] {
            assert_eq!(text_after("fix", &[adds_line, key('x')]), "fix\nx", "{adds_line:?}");
        }
    }

    #[test]
    fn a_plain_enter_edits_nothing() {
        assert_eq!(text_after("fix", &[code(KeyCode::Enter), key('x')]), "fixx");
    }

    #[test]
    fn alt_b_jumps_a_word_back_and_types_no_letter() {
        assert_eq!(text_after("fix the bug", &[alt(KeyCode::Char('b')), key('x')]), "fix the xbug");
    }

    #[test]
    fn alt_f_jumps_a_word_on_and_types_no_letter() {
        assert_eq!(text_after("fix the bug", &[code(KeyCode::Home), alt(KeyCode::Char('f')), key('x')]), "fixx the bug");
    }

    #[test]
    fn a_ctrl_letter_with_no_meaning_types_nothing() {
        assert_eq!(text_after("fix", &[ctrl('o'), key('x')]), "fixx");
    }
}
