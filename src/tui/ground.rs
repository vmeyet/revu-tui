//! The terminal's own background colour, asked once at startup, so a theme that does not know its
//! ground can still tint changed lines instead of guessing a fill.
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

/// How long startup waits for the answer; terminals answer in a few milliseconds.
const WAIT: Duration = Duration::from_millis(50);

/// OSC 11 asks for the background; the primary device attributes (DA1) query after it is
/// answered by every terminal, so reading stops there even when OSC 11 is not supported.
const QUERY: &[u8] = b"\x1b]11;?\x1b\\\x1b[c";

/// Asks the terminal for its background as `0xRRGGBB`; `None` when there is no terminal,
/// no answer within [`WAIT`], or an answer that is not a colour.
pub fn ask() -> Option<u32> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return None;
    }
    crossterm::terminal::enable_raw_mode().ok()?;
    let ground = read_answer();
    let _ = crossterm::terminal::disable_raw_mode();
    ground
}

/// Reads the tty against a deadline: nothing outlives the wait, so no keystroke
/// typed afterwards is ever swallowed, even on a terminal that never answers.
fn read_answer() -> Option<u32> {
    let mut tty = std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty").ok()?;
    tty.write_all(QUERY).ok()?;
    tty.flush().ok()?;
    let deadline = Instant::now() + WAIT;
    let mut answer = Vec::new();
    let mut chunk = [0u8; 64];
    while !attributes_seen(&answer) {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() || !readable(&tty, left) {
            break;
        }
        let read = tty.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        answer.extend_from_slice(&chunk[..read]);
    }
    parse(&answer)
}

/// `select(2)`, not `poll(2)`: macOS answers `poll` on a tty device at once, whatever is there.
fn readable(tty: &std::fs::File, within: Duration) -> bool {
    let fd = tty.as_raw_fd();
    let mut timeout = libc::timeval {
        tv_sec: libc::time_t::try_from(within.as_secs()).unwrap_or(libc::time_t::MAX),
        tv_usec: libc::suseconds_t::try_from(within.subsec_micros()).unwrap_or(0),
    };
    // SAFETY: the set is zeroed then holds one open descriptor below FD_SETSIZE, and select only
    // writes into the set and the timeout it is handed.
    unsafe {
        let mut set: libc::fd_set = std::mem::zeroed();
        libc::FD_ZERO(&raw mut set);
        libc::FD_SET(fd, &raw mut set);
        libc::select(fd + 1, &raw mut set, std::ptr::null_mut(), std::ptr::null_mut(), &raw mut timeout) > 0
            && libc::FD_ISSET(fd, &raw const set)
    }
}

/// The DA1 answer, `ESC [ ? … c`, closes what the terminal sends back.
fn attributes_seen(answer: &[u8]) -> bool {
    answer.ends_with(b"c") && answer.windows(3).any(|w| w == b"\x1b[?")
}

/// Reads `ESC ] 11 ; rgb:R/G/B` ended by BEL or ST, each channel 1 to 4 hex digits scaled to 8 bits.
pub fn parse(answer: &[u8]) -> Option<u32> {
    let text = std::str::from_utf8(answer).ok()?;
    let start = text.find("\x1b]11;rgb:")? + "\x1b]11;rgb:".len();
    let rest = &text[start..];
    let end = rest.find(['\x07', '\x1b'])?;
    let channels: Vec<u32> = rest[..end].split('/').map(channel).collect::<Option<_>>()?;
    let [r, g, b] = channels.as_slice() else { return None };
    Some((r << 16) | (g << 8) | b)
}

fn channel(hex: &str) -> Option<u32> {
    if hex.is_empty() || hex.len() > 4 {
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    let max = (1u32 << (4 * hex.len())) - 1;
    Some((value * 255 + max / 2) / max)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn answers_parse_whatever_the_terminator_and_the_digits() {
        let cases: [(&[u8], Option<u32>); 9] = [
            (b"\x1b]11;rgb:1e1e/1e1e/2e2e\x1b\\\x1b[?62;22c", Some(0x1e1e2e)),
            (b"\x1b]11;rgb:ffff/ffff/ffff\x07", Some(0xffffff)),
            (b"\x1b]11;rgb:0000/0000/0000\x1b\\", Some(0x000000)),
            (b"\x1b]11;rgb:28/2a/36\x07", Some(0x282a36)),
            (b"\x1b]11;rgb:f/0/8\x07", Some(0xff0088)),
            (b"\x1b[?62;4c", None),
            (b"\x1b]11;rgb:1e1e/1e1e\x07", None),
            (b"\x1b]11;rgb:1e1e/1e1e/2e2", None),
            (b"garbage\x1b]11;rgb:zz/00/00\x07", None),
        ];
        for (answer, ground) in cases {
            assert_eq!(parse(answer), ground, "{:?}", String::from_utf8_lossy(answer));
        }
    }

    #[test]
    fn reading_stops_at_the_device_attributes() {
        assert!(!attributes_seen(b"\x1b]11;rgb:0/0/0\x07"));
        assert!(attributes_seen(b"\x1b]11;rgb:0/0/0\x07\x1b[?62;22c"));
        assert!(attributes_seen(b"\x1b[?1;2c"));
    }
}
