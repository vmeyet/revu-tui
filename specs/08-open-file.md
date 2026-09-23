# 08 · Open the file

A diff shows what changed; sometimes the reviewer needs the whole file around it.
`v` hands the file, as it is after the change, to the reviewer's own terminal program, at the line under the cursor.
Quitting that program comes straight back to the review, exactly where it was.

The user's own setup is the reference case: Helix for everything, Glow for Markdown.

```toml
[open]
default = "hx"

[open.files]
"*.md" = "glow -p"
```

## The keys

| Key | Where | Opens |
|---|---|---|
| `v` | review pane, thread pane, file tree | the file after the change (head), at the cursor's line |
| `:view old` | command line | the file before the change (base), at the matching old line |
| `:view <path>[:<line>]` | command line | any file of the MR, head version |

`v` is free in every pane, reads as "view", and sits next to `V` (select lines), which already means "look at these lines".
The base version is rare enough for the command line; a second key would cost more to learn than it saves.
On a deleted file there is no head, so `v` opens the base version and says so in the status line.
`?` and the README list `v`; `03-ui-ux.md` gains one row per table when this lands.

### Which line

The program opens at a line of the head file, picked from the row under the cursor:

| Row | Line |
|---|---|
| added or context line | its new number |
| inline pair (`~`) | its new number |
| removed line | the new number of the next context or added line in the hunk, else the previous one |
| hunk header | the hunk's first new line |
| file row | 1 |
| thread or draft row | the line it is anchored to, mapped as above when it sits on the old side |
| outdated thread | 1 |

The thread pane uses the open thread's anchor.
The file tree uses line 1.

## How it runs

The TUI gives the whole terminal to the program and takes it back on exit, the way `E` already hands a comment to `$EDITOR` (`compose_inline` in `src/tui/mod.rs`).

1. `v` returns `Action::View { key, path, side, line }` from the pure `App`.
2. The content is resolved off the loop (see below); a spinner toast says `fetching charge.rs…` when it takes more than 150 ms.
3. `Incoming::ViewReady { file, line }` arrives; the loop suspends the TUI (`ratatui::restore`, keyboard enhancement flags popped), runs the program with inherited stdin, stdout and stderr, and waits.
4. On exit the loop re-inits the terminal, clears it and redraws.
   Cursor, scroll, folds, selection and the open thread are untouched: the `App` never changed.
5. The status line says `back from hx · charge.rs:42` for 4 s.

A `ViewReady` that arrives after the user opened another MR is dropped, so a slow fetch never throws a program at a screen that moved on.
While the program runs, polling keeps going in the background; its answers are applied on return.
The panic hook `ratatui::init` installs already restores the terminal; the hand-off adds nothing that would skip it.

## Where the content comes from

The real file wins when it is the right file.
Otherwise the forge serves the file at the MR's head commit.

1. **Checkout.** When the command runs inside a checkout of the MR's project (`mrref::checkout_project`) and `git rev-parse HEAD` equals the MR's head sha, `v` opens `<checkout>/<path>` itself.
   The status line says `your checkout · edits are real`, because they are.
   Uncommitted local changes are the user's business: the file opens as it is on disk.
2. **Forge.** Otherwise `Forge::file(key, path, sha) -> Result<Vec<u8>>` fetches the bytes:
   - GitLab: `GET /projects/:project/repository/files/:path/raw?ref=:sha` with the path URL-encoded.
   - GitHub: `GET /repos/:owner/:repo/contents/:path?ref=:sha` with `Accept: application/vnd.github.raw+json`.
   The bytes go to a private temp file, then to the program.
   Fetched files are kept in memory for the session, keyed by `(project, sha, path)`, so a second `v` is instant.

### The temp file

- A fresh private directory per view (`tempfile::TempDir`, mode 0700) holding the file under its own basename, so `charge.rs` stays `charge.rs` and every viewer picks the right syntax.
- Written, then set to mode 0400: an editor that tries to save gets refused and says so, which is the honest answer for a file that is not the checkout.
- Removed when the program exits, on error, and on unwind; the `TempDir` guard lives across the hand-off.
- The program's own read-only flag is added where one exists (table below), so the refusal comes before the edit rather than at save.

## Config

```toml
[open]
default = "hx"                 # the program for every file no glob below matches

[open.files]
"*.md" = "glow -p"             # glob on the path; the most specific glob wins
"docs/**" = "bat --paging=always"
```

Every key is optional.
The program for a file is picked in this order:

1. The `[open.files]` globs that match the file's path (or its basename, for globs without `/`); the longest pattern wins, so `docs/**/*.md` beats `*.md`.
2. `[open] default`.
3. `$VISUAL`, then `$EDITOR`.
4. `less`.

### Commands

A command is a program and its arguments, split like a shell would split words (`shell-words` crate: quotes work, nothing is expanded, nothing runs through `sh -c`).
Two placeholders go anywhere in it: `{file}` (the path handed to the program) and `{line}` (the line number).

A command without `{file}` gets the built-in template of its program, looked up by the basename of its first word:

| Program | Template | Read-only flag |
|---|---|---|
| `hx`, `helix` | `{file}:{line}` | none, the 0400 mode stands in |
| `vim`, `nvim`, `vi` | `+{line} {file}` | `-R` |
| `nano` | `+{line} {file}` | `-v` |
| `micro` | `{file}:{line}` | `-readonly true` |
| `emacs`, `emacsclient -t` | `+{line} {file}` | none |
| `kak` | `+{line} {file}` | none |
| `less` | `+{line}g {file}` | viewer |
| `bat` | `--paging=always --highlight-line {line} {file}` | viewer |
| `glow` | `{file}` | viewer |
| anything else | `{file}` | none |

So `default = "hx"` runs `hx /tmp/…/charge.rs:42`, and `"*.md" = "glow -p"` runs `glow -p /tmp/…/README.md`.
A command that carries its own `{file}` is used exactly as written, which is the escape hatch for anything the table gets wrong.
The read-only flag is added only for forge files, never for the checkout.

Config errors surface at startup like every other `revu` config error: the file, the key, and what is wrong (an unknown placeholder, an empty command, a glob that does not parse).

## Edge cases

| Case | What happens |
|---|---|
| Binary file | Toast `binary file · o opens it in the browser`; nothing runs |
| Deleted file | Opens the base version, status line says `deleted in this MR · showing the old file` |
| Renamed file | Opens the new path at head; `:view old` opens the old path at base |
| Larger than 10 MB | Toast `too large to open here (24 MB) · o opens it in the browser` |
| Forge answers 404 | Toast with the path and the sha, retry key `v` |
| Program not found | Toast `hx not found · set [open] default in config`, naming the program that failed |
| Program exits non-zero | Toast `hx exited with 1`; the review comes back as usual |
| Program killed by a signal | Same, with the signal name |
| Not a terminal (piped run) | `v` is not offered; `:view` says it needs a terminal |

## Security

- **Paths come from the forge**, so they are checked before any file is created: relative, no `..` component, no NUL, no leading `/`, at most 4096 bytes.
  A path that fails the check never reaches the disk, and the toast says the forge sent a path revu does not trust.
- **No shell.** The command is split into argv and run with `std::process::Command`; the path and the line are arguments, never text inside a shell string.
- **Temp hygiene** as above: private directory, 0400 file, removed on every exit path, basename only (the forge path's directories are not recreated under the temp dir).
- **Tokens stay home.** The program inherits the user's environment as it is; revu adds no variable to it, and the forge token is never written to the temp file or its directory.
- **Checkout detection trusts git only** (`git rev-parse HEAD`, `git remote get-url origin`), never a path from the forge, so a crafted path cannot redirect `v` into a different file of the checkout.

## Considered

- **An embedded pane** (a read-only viewer inside the TUI) was rejected: it would reimplement, badly, what `hx`, `glow` and `bat` already do well, and the user asked for their own program.
- **A floating window or a terminal split** (tmux, kitty, Ghostty) was rejected for v1: it depends on the terminal, and the full hand-off already costs one key to come back.
  A later `[open] split = "tmux"` can add it without changing the keys.
- **`e` for edit** was rejected: `e` edits a note in the thread pane, and most opens are for reading.
- **A second key for the base version** (`gv`, `alt-v`) was rejected: rare use, and `g` already moves to the top.
- **Always fetching from the forge** was rejected: in a checkout at the right commit, the real file gives the user their LSP, git blame and project config for free.
- **Opening the checkout at any commit** was rejected: a checkout on another commit would show code the MR does not contain, silently.

## Tests

In the order to write them, failing first:

1. `open::line_for(row, review)`: one table test per row kind in the "Which line" table, including a removed line at the end of a hunk and an old-side thread.
2. `open::command_for(path, config, env)`: glob precedence (longest wins, path vs basename), `default`, `$VISUAL`, `$EDITOR`, `less`.
3. `open::argv(command, file, line, read_only)`: every row of the templates table, a command with its own `{file}`, quotes in arguments, an unknown placeholder rejected, the read-only flag only for forge files.
4. `open::safe_path(path)`: `..`, absolute, NUL, over-long and valid paths.
5. `Forge::file` for GitLab and GitHub with wiremock: path encoding, the raw accept header, 404, size over the limit.
6. `Source::pick(checkout, head_sha, git_head)`: checkout at head, checkout elsewhere, no checkout.
7. Temp file: 0700 directory, 0400 file, basename kept, removed after a run of `true` and after a run of `false`.
8. `App`: `v` on each pane emits `Action::View` with the right line; a late `ViewReady` for another MR is dropped; the toasts for binary, deleted, too large and missing program.
9. The hand-off itself with a fake program (a test binary that records its argv and exits), run through the same function the loop uses, without a real terminal.

## Build order

One slice, under 500 lines without tests: `open.rs` (pure: line, command, argv, safe path), `Forge::file` on both forges, the loop's hand-off next to `compose_inline`, then the keys and the help.
