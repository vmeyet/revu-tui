//! The clipboard of the terminal the reader looks at, which over ssh or mosh is not the machine revu runs on.
use base64::Engine as _;
use std::io::Write as _;

/// OSC 52: the terminal puts `text` on its own clipboard. Inside tmux it goes out twice, since tmux hands on
/// the plain one only with `set-clipboard on` and the wrapped one only with `allow-passthrough on`.
pub fn escape(text: &str, in_tmux: bool) -> String {
    let plain = format!("\x1b]52;c;{}\x07", base64::engine::general_purpose::STANDARD.encode(text));
    if in_tmux { format!("\x1bPtmux;{}\x1b\\{plain}", plain.replace('\x1b', "\x1b\x1b")) } else { plain }
}

/// Written from the loop, between two frames, so the escape never lands inside one.
pub fn print(text: &str) {
    let mut out = std::io::stdout();
    let _ = out.write_all(escape(text, std::env::var_os("TMUX").is_some()).as_bytes());
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_is_one_osc_52_escape_with_the_text_in_base64() {
        assert_eq!(escape("https://gitlab.com/acme/widgets/-/merge_requests/42", false).as_bytes(), {
            b"\x1b]52;c;aHR0cHM6Ly9naXRsYWIuY29tL2FjbWUvd2lkZ2V0cy8tL21lcmdlX3JlcXVlc3RzLzQy\x07"
        });
        assert_eq!(escape("", false), "\x1b]52;c;\x07");
        assert_eq!(escape("héllo\n", false), "\x1b]52;c;aMOpbGxvCg==\x07", "any text, as its utf-8 bytes");
    }

    #[test]
    fn inside_tmux_the_escape_goes_out_wrapped_for_a_passthrough_then_plain() {
        assert_eq!(escape("hi", true).as_bytes(), b"\x1bPtmux;\x1b\x1b]52;c;aGk=\x07\x1b\\\x1b]52;c;aGk=\x07");
    }
}
