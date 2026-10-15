# 12 · Changelog

Goal: someone who updates revu learns what changed for them, in plain words, without leaving the terminal.

## The page

`CHANGELOG.md` at the repo root is the single source.
It is written for users: no commit words, no internal names, only what they can see or press.
`src/changelog.rs` builds it into the binary with `include_str!`, so it works offline and always matches the binary that prints it.

```markdown
## Unreleased

### New

- **Keep an MR on top of the queue.**
  In the queue, under PINNED, until the MR is merged or closed.
  `b` pins or unpins it, and `:pin` and `:unpin` do the same from the diff.

## 0.6.0 - 2026-10-06

### Fixed

- **Scrolling a big MR no longer lags.**
  In the thread pane, the file tree and the pipeline pane.
```

- `## Unreleased` comes first and holds what is on `main` after the last release; then one `## <version> - <date>` per release, newest first.
- Three groups per section, in this order: `### New`, `### Fixed` and `### Removed`; any may be missing.
- An entry is three lines, one sentence each: **what** in bold after `- `, then **where** it applies, then **how** to use it, both indented by two spaces.
- A fix or a removal may stop at where, when there is nothing to press; a removal's how says what to use instead.
- Text before the first section is the page's introduction on the forge; the binary skips it.
- Anything else is refused: the parser fails with the entry or heading it stopped on, and a test parses the shipped page.

The heading `## 0.6.0 - 2026-10-06` is one cargo-dist reads (through `parse-changelog`): on a release it puts that section above the install steps of the GitHub release, with no change to the workflow.
This is what its documentation says; no release ran with it yet.

## What is new for a reader

A reader stands at a **position**: a version, plus a count of `Unreleased` entries, written `0.6.0+2`.
A build from `main` keeps the number of the last release, so only that count tells two such builds apart.
A released binary carries an empty `Unreleased`, so its position is its version alone.

- A released section is new when its version is above the position's.
- `Unreleased` is new when the binary's version is above the position's, or equal with more entries than the position counted.
- On the same version the count of new things is the entries added since; which ones they are is not known, so the whole section is shown.
- What was unreleased comes back as new under its version once released.
- With nothing remembered, the reader stands on the release before the running one: an update from a version older than this feature still announces one release, not the whole history.

## Where it shows

- `revu news` prints every section; `--since <version>` only what came after it, or `Nothing new since <version>.`; `--json` prints the sections with `version`, `date`, `new`, `fixed` and `removed`, each entry as `what`, `where`, `how`.
- `revu update` ends with `New since <version>`, the new sections and a line on how to read them again; nothing more than before when nothing is new.
  The running process is the old binary, so it runs the installed one (`current_exe`, which brew and cargo both replaced in place) with the hidden `--after-update` and its own position as `--since`.
  A failure to show it only warns: the update itself worked.
- The TUI toasts `revu 0.7.0 · 4 new things · :news` on the first start after the position moved, in the usual toast row, never as a modal.
- `:news` opens the page over the panes, newest first, `● new for you` beside each section new to the reader; `j` `k`, `^d` `^u`, `^f` `^b`, `g` `G` scroll, `esc` or `q` close.

The position last seen is kept in the shared cache as `news.json`, never in the config.
It is written at start, as soon as it is read: the toast shows once, and the mark on the page lasts for that session.

## Keeping it current

`AGENTS.md` § Releases holds the rule; two tests in `src/changelog.rs` hold it up.

- Every entry of the shipped page parses in the shape above.
- When `Cargo.toml` names a version with no section while `Unreleased` still holds entries, the test fails: a bump moves them first.

`tests/docs.rs` reads the page with the user docs: one sentence per line, 25 words at most, no filler.
