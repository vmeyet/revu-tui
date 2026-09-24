//! Running a command the user wrote in the config: split into words, never through a shell,
//! with a time limit and capped output, so a slow or loud program cannot hang revu.
use anyhow::{Context, Result, bail};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const OUTPUT_CAP: u64 = 1024 * 1024;
const STDERR_CAP: u64 = 16 * 1024;

/// Runs `command` with `input` on its stdin (nothing when `None`) and returns what it printed.
/// `what` names the command in errors (`ready command`, `share command`); a failure, a timeout or a
/// missing program is an error saying which and why. Output cut at the cap counts as success.
pub async fn run(command: &str, input: Option<&str>, what: &str, limit: Duration) -> Result<String> {
    let words = shell_words::split(command).with_context(|| format!("{what} `{command}` does not split into words"))?;
    let Some((program, args)) = words.split_first() else { bail!("the {what} is empty") };
    let mut child = tokio::process::Command::new(program)
        .args(args)
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("{what}: cannot run `{program}`"))?;
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else { bail!("{what}: no output pipes") };
    let stdin = child.stdin.take();
    let run = async {
        let (written, out, err) = tokio::join!(feed(stdin, input), read_capped(stdout, OUTPUT_CAP), read_capped(stderr, STDERR_CAP));
        let status = child.wait().await?;
        written?;
        anyhow::Ok((status, out?, err?))
    };
    let (status, out, err) =
        tokio::time::timeout(limit, run).await.with_context(|| format!("{what} `{program}` took longer than {}s", limit.as_secs()))??;
    let cut_at_cap = out.len() as u64 == OUTPUT_CAP;
    if !status.success() && !cut_at_cap {
        let why = String::from_utf8_lossy(&err).lines().find(|l| !l.trim().is_empty()).map_or("no message", str::trim).to_owned();
        bail!("{what} `{program}` failed: {why}");
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// Writes the input and closes stdin, so the program sees the end of it. A program that exits
/// without reading everything closes the pipe first: that is its choice, not an error.
async fn feed(stdin: Option<tokio::process::ChildStdin>, input: Option<&str>) -> std::io::Result<()> {
    let (Some(mut stdin), Some(input)) = (stdin, input) else { return Ok(()) };
    match stdin.write_all(input.as_bytes()).await {
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        other => other,
    }
}

/// Up to `cap` bytes, then the pipe closes: a command printing more gets a broken pipe and ends,
/// instead of blocking on a pipe nobody reads while the other one waits for it to finish.
async fn read_capped(pipe: impl tokio::io::AsyncRead + Unpin, cap: u64) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    pipe.take(cap).read_to_end(&mut bytes).await?;
    Ok(bytes)
}

/// The command reads as words; checked when the config loads. `key` names it in the error.
pub fn check(command: &str, key: &str) -> Result<()> {
    let words = shell_words::split(command).with_context(|| format!("`{key}` does not split into words: {command}"))?;
    if words.is_empty() {
        bail!("`{key}` is empty");
    }
    Ok(())
}

#[cfg(test)]
pub mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    /// An executable script in a temp dir, and the command that runs it.
    pub fn script(body: &str) -> (tempfile::TempDir, String) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("program.sh");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let command = shell_words::quote(&path.to_string_lossy()).into_owned();
        (dir, command)
    }

    #[tokio::test]
    async fn the_input_reaches_stdin_whole() {
        let (_dir, echo) = script("cat");
        let message = "[!42 charge cards](https://gitlab.com/acme/widgets/-/merge_requests/42)\n_needs a second look_\n";
        assert_eq!(run(&echo, Some(message), "share command", Duration::from_secs(5)).await.unwrap(), message);
    }

    #[tokio::test]
    async fn a_program_that_ignores_its_input_still_succeeds() {
        let (_dir, quiet) = script("echo done");
        let big = "x".repeat(200_000);
        assert_eq!(run(&quiet, Some(&big), "share command", Duration::from_secs(5)).await.unwrap(), "done\n");
    }

    #[tokio::test]
    async fn errors_name_the_command_kind() {
        let (_dir, failing) = script("cat >/dev/null\necho 'channel not found' >&2\nexit 1");
        let err = run(&failing, Some("hi"), "share command", Duration::from_secs(5)).await.unwrap_err().to_string();
        assert!(err.starts_with("share command") && err.contains("channel not found"), "{err}");
    }
}
