//! A program given a time to end in: past it, the program and whatever it started are stopped.
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Output, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How often a running program is checked for its end.
const CHECK_EVERY: Duration = Duration::from_millis(20);

/// What `command` printed once it ended, or `None` when it was still running after `limit` and was stopped.
pub(super) fn output_within(mut command: Command, limit: Duration) -> std::io::Result<Option<Output>> {
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0).spawn()?;
    let printed = drain(child.stdout.take());
    let complained = drain(child.stderr.take());
    let deadline = Instant::now() + limit;
    while child.try_wait()?.is_none() {
        if Instant::now() >= deadline {
            stop(&mut child)?;
            return Ok(None);
        }
        std::thread::sleep(CHECK_EVERY);
    }
    let status = child.wait()?;
    Ok(Some(Output { status, stdout: drained(printed)?, stderr: drained(complained)? }))
}

fn drain(pipe: Option<impl Read + Send + 'static>) -> JoinHandle<std::io::Result<Vec<u8>>> {
    std::thread::spawn(move || {
        let mut bytes = vec![];
        if let Some(mut pipe) = pipe {
            pipe.read_to_end(&mut bytes)?;
        }
        Ok(bytes)
    })
}

/// What a drain read; one that failed or panicked is an error, never an empty output passed off as the program's.
fn drained(drain: JoinHandle<std::io::Result<Vec<u8>>>) -> std::io::Result<Vec<u8>> {
    drain.join().map_err(|_panic| std::io::Error::other("reading the program's output panicked"))?
}

/// The whole process group, since a stuck `git fetch` is often the `ssh` it started.
fn stop(child: &mut Child) -> std::io::Result<()> {
    if let Ok(group) = libc::pid_t::try_from(child.id()) {
        // SAFETY: `kill` only sends a signal; the group is the child's own, made by `process_group(0)`.
        unsafe { libc::kill(-group, libc::SIGKILL) };
    }
    child.wait().map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str) -> Command {
        let mut command = Command::new("sh");
        command.args(["-c", script]);
        command
    }

    #[test]
    fn a_program_that_ends_in_time_gives_what_it_printed() {
        let output = output_within(sh("echo said; echo complained >&2; exit 3"), Duration::from_secs(5)).unwrap().unwrap();
        assert_eq!((output.stdout.as_slice(), output.stderr.as_slice()), (&b"said\n"[..], &b"complained\n"[..]));
        assert_eq!(output.status.code(), Some(3));
    }

    #[test]
    fn a_drain_that_fails_or_panics_is_an_error() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("pipe broke"))
            }
        }
        assert_eq!(drained(drain(Some(Broken))).unwrap_err().to_string(), "pipe broke");
        let panicked = std::thread::spawn(|| -> std::io::Result<Vec<u8>> { panic!("reader died") });
        assert_eq!(drained(panicked).unwrap_err().to_string(), "reading the program's output panicked");
        assert_eq!(drained(drain(None::<&[u8]>)).unwrap(), b"");
    }

    #[test]
    fn a_program_still_running_at_its_limit_is_stopped_with_what_it_started() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        let started = Instant::now();
        let script = format!("sleep 30 & echo $! > '{}'; wait", pid_file.display());
        assert!(output_within(sh(&script), Duration::from_millis(300)).unwrap().is_none());
        assert!(started.elapsed() < Duration::from_secs(5), "it did not wait for the program: {:?}", started.elapsed());
        let pid = std::fs::read_to_string(&pid_file).unwrap();
        let is_alive = || Command::new("kill").args(["-0", pid.trim()]).stderr(Stdio::null()).status().unwrap().success();
        let reaped_by = Instant::now() + Duration::from_secs(3);
        while is_alive() && Instant::now() < reaped_by {
            std::thread::sleep(CHECK_EVERY);
        }
        assert!(!is_alive(), "the program it started was stopped too");
    }
}
