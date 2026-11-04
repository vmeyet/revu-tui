//! Questions asked on the terminal, on stderr so that stdout stays the command's output.
use anyhow::{Result, bail};
use std::io::{BufRead, IsTerminal, Write};

/// A yes typed on the terminal; with no terminal, fails with `no_terminal`, which tells a script what to pass instead.
pub(super) fn confirmed(question: &str, no_terminal: &str) -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        bail!("{no_terminal}");
    }
    eprint!("{question} [y/N] ");
    std::io::stderr().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    Ok(is_yes(&answer))
}

fn is_yes(answer: &str) -> bool {
    let answer = answer.trim();
    answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes")
}

/// Asks for `label` and reads one line with echo off, restoring the terminal even when the read fails.
pub(super) fn read_hidden(label: &str) -> Result<String> {
    eprint!("{label} (hidden): ");
    std::io::stderr().flush()?;
    crossterm::terminal::enable_raw_mode()?;
    let line = read_raw_line();
    crossterm::terminal::disable_raw_mode()?;
    let line = line?;
    eprintln!();
    Ok(line)
}

fn read_raw_line() -> Result<String> {
    use crossterm::event::{Event, KeyCode, KeyModifiers, read};
    let mut line = String::new();
    loop {
        if let Event::Key(key) = read()? {
            match key.code {
                KeyCode::Enter => return Ok(line),
                KeyCode::Backspace => {
                    line.pop();
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => bail!("cancelled"),
                KeyCode::Char(c) => line.push(c),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_yes;

    #[test]
    fn y_or_yes_in_any_case_is_a_yes() {
        for answer in ["y\n", "Y\n", "yes\n", "YES\n", " Yes \n"] {
            assert!(is_yes(answer), "{answer:?}");
        }
    }

    #[test]
    fn anything_else_is_a_no() {
        for answer in ["\n", "n\n", "no\n", "yep\n", ""] {
            assert!(!is_yes(answer), "{answer:?}");
        }
    }
}
