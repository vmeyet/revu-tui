//! `v`: the file under the cursor, handed whole to the reader's own program at the line they look at.
//! Everything here but [`run`], [`write_private`] and [`Checkout`] is pure.
use crate::config;
use crate::diff::{Hunk, Line};
use crate::review::{Anchor, Review, Row, Side};
use anyhow::{Context, Result, bail};
use glob::{MatchOptions, Pattern};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

/// Above this, a file is for the browser, not a terminal program.
pub const MAX_BYTES: usize = 10 * 1024 * 1024;
const MAX_PATH: usize = 4096;
const FALLBACK: &str = "less";

/// The line of `side`'s file a row stands for: its own number when it has one, the nearest line
/// that exists on that side when it does not (a removed line has no head number).
pub fn line_for(review: &Review, row: &Row, side: Side) -> u32 {
    match row {
        Row::Line { file, hunk, index } => nearest(&review.files[*file].hunks[*hunk], *index, side),
        Row::Pair { file, hunk, removed, added } => {
            let index = if side == Side::New { *added } else { *removed };
            nearest(&review.files[*file].hunks[*hunk], index, side)
        }
        Row::Context { old, new, .. } => pick(side, Some(*old), Some(*new)).unwrap_or(1),
        Row::Hunk { file, index, .. } => {
            let hunk = &review.files[*file].hunks[*index];
            hunk.lines.iter().find_map(|l| number(l, side)).unwrap_or_else(|| start(hunk, side))
        }
        Row::Header | Row::File { .. } | Row::Gap => 1,
    }
}

/// Where an anchor sits in `side`'s file: its own line on that side, else the nearest one there.
pub fn line_of_anchor(review: &Review, anchor: &Anchor, side: Side) -> u32 {
    if anchor.side == side {
        return anchor.line;
    }
    review
        .files
        .iter()
        .filter(|f| f.new_path == anchor.path || f.old_path == anchor.path)
        .flat_map(|f| &f.hunks)
        .find_map(|hunk| {
            let index = hunk.lines.iter().position(|l| number(l, anchor.side) == Some(anchor.line))?;
            Some(nearest(hunk, index, side))
        })
        .unwrap_or(1)
}

fn nearest(hunk: &Hunk, index: usize, side: Side) -> u32 {
    let after = hunk.lines[index..].iter().find_map(|l| number(l, side));
    let before = || hunk.lines[..index].iter().rev().find_map(|l| number(l, side));
    after.or_else(before).unwrap_or_else(|| start(hunk, side))
}

fn number(line: &Line, side: Side) -> Option<u32> {
    pick(side, line.old, line.new)
}

fn pick(side: Side, old: Option<u32>, new: Option<u32>) -> Option<u32> {
    match side {
        Side::New => new,
        Side::Old => old,
    }
}

fn start(hunk: &Hunk, side: Side) -> u32 {
    let start = if side == Side::New { hunk.new_start } else { hunk.old_start };
    start.max(1)
}

/// The command for `path`: the longest matching `[open.files]` glob, then `[open] default`,
/// then `$VISUAL`, `$EDITOR`, and `less`.
pub fn command_for(path: &str, open: &config::Open, visual: Option<&str>, editor: Option<&str>) -> String {
    let matching = open.files.iter().filter(|(glob, _)| matches(glob, path)).max_by_key(|(glob, _)| glob.len());
    matching
        .map(|(_, command)| command.as_str())
        .or(open.default.as_deref())
        .or(visual.filter(|v| !v.trim().is_empty()))
        .or(editor.filter(|e| !e.trim().is_empty()))
        .unwrap_or(FALLBACK)
        .to_owned()
}

/// A glob with a `/` matches the whole path; one without matches the basename, as `.gitignore` does.
fn matches(glob: &str, path: &str) -> bool {
    let options = MatchOptions { require_literal_separator: true, ..MatchOptions::new() };
    let target = if glob.contains('/') { path } else { basename(path) };
    Pattern::new(glob).is_ok_and(|pattern| pattern.matches_with(target, options))
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The argv to run: the command's own words with `{file}` and `{line}` filled in when it names
/// `{file}`, else its program's built-in template; `read_only` adds the program's read-only flag.
/// No shell ever sees it: quotes group words, nothing expands.
pub fn argv(command: &str, file: &str, line: u32, read_only: bool) -> Result<Vec<String>> {
    let words = shell_words::split(command).with_context(|| format!("`{command}` does not split into words"))?;
    let Some(program) = words.first() else { bail!("the command is empty") };
    for word in &words {
        check_placeholders(word)?;
    }
    let fill = |word: &str| word.replace("{file}", file).replace("{line}", &line.to_string());
    if words.iter().any(|w| w.contains("{file}")) {
        return Ok(words.iter().map(|w| fill(w)).collect());
    }
    let (template, flag) = template(basename(program));
    let mut argv: Vec<String> = words.clone();
    if read_only {
        argv.extend(flag.iter().map(|f| (*f).to_owned()));
    }
    argv.extend(template.iter().map(|w| fill(w)));
    Ok(argv)
}

/// Checks a command the way [`argv`] will read it, for the config loader.
pub fn check(command: &str) -> Result<()> {
    argv(command, "file", 1, false).map(drop)
}

fn check_placeholders(word: &str) -> Result<()> {
    let mut rest = word;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else { return Ok(()) };
        let name = &rest[open + 1..open + close];
        if !matches!(name, "file" | "line") && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !name.is_empty() {
            bail!("unknown placeholder {{{name}}}: only {{file}} and {{line}} exist");
        }
        rest = &rest[open + close + 1..];
    }
    Ok(())
}

/// How a known program is told the file and the line, and its read-only flag.
fn template(program: &str) -> (&'static [&'static str], &'static [&'static str]) {
    match program {
        "hx" | "helix" => (&["{file}:{line}"], &[]),
        "vim" | "nvim" | "vi" => (&["+{line}", "{file}"], &["-R"]),
        "nano" => (&["+{line}", "{file}"], &["-v"]),
        "micro" => (&["{file}:{line}"], &["-readonly", "true"]),
        "emacs" | "emacsclient" | "kak" => (&["+{line}", "{file}"], &[]),
        "less" => (&["+{line}g", "{file}"], &[]),
        "bat" => (&["--paging=always", "--highlight-line", "{line}", "{file}"], &[]),
        _ => (&["{file}"], &[]),
    }
}

/// A path the forge sent, checked before anything touches the disk: relative, no `..`, no NUL.
pub fn safe_path(path: &str) -> Result<()> {
    let trusted = !path.is_empty()
        && path.len() <= MAX_PATH
        && !path.starts_with('/')
        && !path.contains('\0')
        && path.split('/').all(|part| part != ".." && !part.is_empty());
    if !trusted {
        bail!("the forge sent a path revu does not trust: {}", path.escape_debug());
    }
    Ok(())
}

/// Where the bytes come from: the reader's own checkout when it sits on the very commit, else the forge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Checkout,
    Forge,
}

impl Source {
    /// `checkout` is the project of the checkout and the commit its `HEAD` points at.
    pub fn pick(project: &str, sha: &str, checkout: Option<(&str, &str)>) -> Self {
        match checkout {
            Some((there, head)) if there == project && head == sha => Source::Checkout,
            _ => Source::Forge,
        }
    }
}

/// The checkout `revu` runs in: its root, and the project its `origin` names on the forge's host.
#[derive(Clone, Debug)]
pub struct Checkout {
    pub root: PathBuf,
    pub project: String,
}

impl Checkout {
    pub fn find(dir: &Path, host: &str) -> Option<Self> {
        let project = crate::mrref::checkout_project(dir, host)?;
        let root = git(dir, &["rev-parse", "--show-toplevel"])?;
        Some(Self { root: PathBuf::from(root), project })
    }

    /// Asked each time: the reader may have checked out another commit since `revu` started.
    pub fn head(&self) -> Option<String> {
        git(&self.root, &["rev-parse", "HEAD"])
    }
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git").arg("-C").arg(dir).args(args).output().ok()?;
    let text = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && !text.is_empty()).then_some(text)
}

/// A file ready to hand over: the argv, what to say about it, and the private copy it may live in.
#[derive(Clone, Debug)]
pub struct View {
    pub argv: Vec<String>,
    /// `charge.rs:42`, for the status line on the way back.
    pub shown: String,
    /// Said while the program runs: whose file this is.
    pub note: Option<String>,
    /// The private directory of a fetched file, held only so that dropping the view removes it.
    pub _copy: Option<Arc<tempfile::TempDir>>,
}

impl PartialEq for View {
    fn eq(&self, other: &Self) -> bool {
        self.argv == other.argv && self.shown == other.shown && self.note == other.note
    }
}

impl View {
    pub fn program(&self) -> &str {
        self.argv.first().map_or("", |p| basename(p))
    }
}

/// `bytes` under `name` in a fresh private directory (0700), the file itself read-only (0400).
pub fn write_private(name: &str, bytes: &[u8]) -> Result<(tempfile::TempDir, PathBuf)> {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::Builder::new().prefix("revu-view-").tempdir().context("creating a private directory")?;
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))?;
    let file = dir.path().join(basename(name));
    std::fs::write(&file, bytes).with_context(|| format!("writing {}", file.display()))?;
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o400))?;
    Ok((dir, file))
}

/// Runs the program on the terminal as it is and waits; the error says what the reader can do.
pub fn run(argv: &[String]) -> Result<(), String> {
    let Some((program, args)) = argv.split_first() else { return Err("nothing to run".into()) };
    let name = basename(program);
    let status = Command::new(program).args(args).status().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => format!("{name} not found · set [open] default in config"),
        _ => format!("could not run {name}: {e}"),
    })?;
    if status.success() {
        return Ok(());
    }
    Err(match (status.code(), std::os::unix::process::ExitStatusExt::signal(&status)) {
        (Some(code), _) => format!("{name} exited with {code}"),
        (None, Some(signal)) => format!("{name} was stopped by {}", signal_name(signal)),
        (None, None) => format!("{name} failed"),
    })
}

fn signal_name(signal: i32) -> String {
    match signal {
        1 => "SIGHUP".into(),
        2 => "SIGINT".into(),
        9 => "SIGKILL".into(),
        15 => "SIGTERM".into(),
        other => format!("signal {other}"),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::forge::DiffFile;

    fn review_of(diff: &str) -> Review {
        let mr = crate::review::tests::mr();
        let file = DiffFile { diff: diff.into(), old_path: "src/a.rs".into(), new_path: "src/a.rs".into(), ..DiffFile::default() };
        Review::new(mr, &[file], vec![], &[])
    }

    const DIFF: &str = "@@ -10,4 +10,4 @@\n keep\n-gone\n+came\n tail\n@@ -30,2 +30,1 @@\n stay\n-last\n";

    fn line(hunk: usize, index: usize) -> Row {
        Row::Line { file: 0, hunk, index }
    }

    #[test]
    fn every_row_maps_to_a_line_of_the_head_file() {
        let review = review_of(DIFF);
        let head = |row: &Row| line_for(&review, row, Side::New);
        assert_eq!(head(&line(0, 0)), 10, "context: its new number");
        assert_eq!(head(&line(0, 2)), 11, "added: its new number");
        assert_eq!(head(&line(0, 1)), 11, "removed: the next line that exists at head");
        assert_eq!(head(&line(1, 1)), 30, "removed at the end of a hunk: the previous one");
        assert_eq!(head(&Row::Hunk { file: 0, index: 1, open: true }), 30);
        assert_eq!(head(&Row::Pair { file: 0, hunk: 0, removed: 1, added: 2 }), 11);
        assert_eq!(head(&Row::Context { file: 0, hunk: 0, old: 7, new: 8 }), 8);
        assert_eq!(head(&Row::File { index: 0, open: true }), 1);
    }

    #[test]
    fn the_old_file_uses_old_numbers() {
        let review = review_of(DIFF);
        assert_eq!(line_for(&review, &line(0, 1), Side::Old), 11);
        assert_eq!(line_for(&review, &line(0, 2), Side::Old), 12, "added: the next line the old file has");
    }

    #[test]
    fn an_old_side_anchor_lands_on_the_nearest_head_line() {
        let review = review_of(DIFF);
        let anchor = Anchor { path: "src/a.rs".into(), side: Side::Old, line: 31 };
        assert_eq!(line_of_anchor(&review, &anchor, Side::New), 30);
        let own = Anchor { path: "src/a.rs".into(), side: Side::New, line: 12 };
        assert_eq!(line_of_anchor(&review, &own, Side::New), 12);
    }

    fn open_config(default: Option<&str>, files: &[(&str, &str)]) -> config::Open {
        config::Open { default: default.map(str::to_owned), files: files.iter().map(|(g, c)| ((*g).to_owned(), (*c).to_owned())).collect() }
    }

    #[test]
    fn the_longest_matching_glob_wins_then_default_then_the_environment() {
        let config = open_config(Some("hx"), &[("*.md", "glow -p"), ("docs/**/*.md", "bat"), ("docs/**", "less")]);
        assert_eq!(command_for("README.md", &config, None, None), "glow -p");
        assert_eq!(command_for("src/deep/notes.md", &config, None, None), "glow -p", "a glob without / matches the basename");
        assert_eq!(command_for("docs/guide/intro.md", &config, None, None), "bat");
        assert_eq!(command_for("docs/logo.svg", &config, None, None), "less");
        assert_eq!(command_for("src/a.rs", &config, None, None), "hx");
        let empty = config::Open::default();
        assert_eq!(command_for("a.rs", &empty, Some("nvim"), Some("vi")), "nvim");
        assert_eq!(command_for("a.rs", &empty, Some(" "), Some("vi")), "vi");
        assert_eq!(command_for("a.rs", &empty, None, None), "less");
    }

    fn run_argv(command: &str, read_only: bool) -> Vec<String> {
        argv(command, "/tmp/x/charge.rs", 42, read_only).unwrap()
    }

    #[test]
    fn known_programs_get_their_template() {
        assert_eq!(run_argv("hx", true), ["hx", "/tmp/x/charge.rs:42"]);
        assert_eq!(run_argv("nvim", true), ["nvim", "-R", "+42", "/tmp/x/charge.rs"]);
        assert_eq!(run_argv("vim", false), ["vim", "+42", "/tmp/x/charge.rs"]);
        assert_eq!(run_argv("nano", true), ["nano", "-v", "+42", "/tmp/x/charge.rs"]);
        assert_eq!(run_argv("micro", true), ["micro", "-readonly", "true", "/tmp/x/charge.rs:42"]);
        assert_eq!(run_argv("emacsclient -t", false), ["emacsclient", "-t", "+42", "/tmp/x/charge.rs"]);
        assert_eq!(run_argv("kak", false), ["kak", "+42", "/tmp/x/charge.rs"]);
        assert_eq!(run_argv("less", true), ["less", "+42g", "/tmp/x/charge.rs"]);
        assert_eq!(run_argv("bat", true), ["bat", "--paging=always", "--highlight-line", "42", "/tmp/x/charge.rs"]);
        assert_eq!(run_argv("glow -p", true), ["glow", "-p", "/tmp/x/charge.rs"]);
        assert_eq!(run_argv("/opt/bin/code -w", true), ["/opt/bin/code", "-w", "/tmp/x/charge.rs"]);
    }

    #[test]
    fn a_command_with_its_own_file_is_used_as_written() {
        assert_eq!(run_argv("nvim -c 'set ro' {file} +{line}", true), ["nvim", "-c", "set ro", "/tmp/x/charge.rs", "+42"]);
        assert_eq!(run_argv("\"my editor\" --goto={line} {file}", false), ["my editor", "--goto=42", "/tmp/x/charge.rs"]);
    }

    #[test]
    fn bad_commands_are_refused() {
        assert!(argv("", "f", 1, false).unwrap_err().to_string().contains("empty"));
        assert!(argv("hx {path}", "f", 1, false).unwrap_err().to_string().contains("{path}"));
        assert!(argv("hx 'open", "f", 1, false).is_err());
        assert!(check("bat --style={file}").is_ok());
        assert!(check("jq '{a: 1}' {file}").is_ok(), "braces around something else are text");
    }

    #[test]
    fn only_plain_relative_paths_are_trusted() {
        assert!(safe_path("src/pay/charge.rs").is_ok());
        assert!(safe_path(".github/ci.yml").is_ok());
        for bad in ["", "/etc/passwd", "../x", "a/../../x", "a//b", "a/\0b"] {
            assert!(safe_path(bad).is_err(), "{bad:?}");
        }
        assert!(safe_path(&"a/".repeat(2049)).is_err());
    }

    #[test]
    fn the_checkout_is_used_only_on_the_very_commit() {
        assert_eq!(Source::pick("acme/widgets", "b1", Some(("acme/widgets", "b1"))), Source::Checkout);
        assert_eq!(Source::pick("acme/widgets", "b1", Some(("acme/widgets", "c2"))), Source::Forge);
        assert_eq!(Source::pick("acme/widgets", "b1", Some(("acme/other", "b1"))), Source::Forge);
        assert_eq!(Source::pick("acme/widgets", "b1", None), Source::Forge);
    }

    #[test]
    fn the_private_copy_is_read_only_in_a_private_directory_and_goes_away() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, file) = write_private("src/pay/charge.rs", b"fn main() {}\n").unwrap();
        assert_eq!(file.file_name().unwrap(), "charge.rs", "the basename is kept, the directories are not");
        assert_eq!(std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o400);
        let root = dir.path().to_owned();
        let view = View { argv: vec!["true".into()], shown: "charge.rs:1".into(), note: None, _copy: Some(Arc::new(dir)) };
        assert_eq!(run(&view.argv), Ok(()));
        drop(view);
        assert!(!root.exists(), "the copy is removed once the view is done");
    }

    #[test]
    fn the_hand_off_reports_what_went_wrong() {
        assert_eq!(run(&["false".into()]), Err("false exited with 1".into()));
        assert_eq!(run(&["revu-no-such-program".into()]), Err("revu-no-such-program not found · set [open] default in config".into()));
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("argv");
        run(&["touch".into(), out.display().to_string()]).unwrap();
        assert!(out.exists(), "the program got the path as an argument");
    }
}
