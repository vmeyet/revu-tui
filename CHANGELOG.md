# What's new in revu

Every change you can see or use, newest first.
`revu news` prints this page, and `:news` opens it inside revu.

Each entry says what changed, then where, then how to use it.

## Unreleased

## 0.8.0 - 2026-10-14

### New

- **Your most used reactions come first.**
  In the reaction picker, on each host apart, with GitLab emoji you found by name among them.
  `+` lists the three you give most, then the usual eight.
- **Copy a link to the device in your hand.**
  Over ssh or mosh, and inside tmux with `set-clipboard on`.
  `y` copies the link of the MR, line, thread or job, and every copy reaches your terminal's clipboard.
- **Copy the first link of a thread.**
  In the thread pane and on a thread row of the diff.
  `U` copies what `u` opens, or the thread's first picture when it has no link.
- **Copy a link from the command line.**
  In the queue and in the diff.
  `:copy` copies the link `:open` would show in the browser.
- **Review from a phone, one page at a time.**
  In a window under 70 columns: the queue, the diff and the right pane each take the whole screen.
  `enter` opens the diff, `h` goes back a page, and `[tui] layout = "compact"` forces it at any width.
- **A diff that fits a phone.**
  In the compact layout: long lines wrap, only the sign shows, and the title tells the cursor's line.
  `w` cuts long lines again, and a long file path keeps its folder and its name.
- **More rows for the page when the keyboard is up.**
  In the compact layout under 20 rows: the status row and the MR header hide.
  A question, the search and the queue filter keep their row, and a toast shows on the bottom row.
- **Tap a label instead of typing a key.**
  In the compact layout, on the bottom row: the main actions of the page, and `more` for every command.
  Tap a label to press its key, and set `[tui] tap_bar = false` to hide the row.
- **A short key list for the page you are on.**
  In the compact layout, on the first `?`: the keys of the tap bar.
  `?` again lists every key.
- **Open the search with a comma.**
  In the queue, the diff and the right pane, wherever `ctrl-k` works.
  `,` opens it on MRs, then `/` finds files and `>` runs commands.
- **Open the Claude Code session of an MR.**
  In the queue, the diff and zen, once `[agent] enabled = true` is in the config.
  `a` then `g`, or `:session`, opens it beside the diff in a tmux, Ghostty, iTerm2 or WezTerm split, or starts one on the MR's branch.

### Fixed

- **A screen at rest sends nothing to a slow link.**
  In the compact layout: the queue marks stay lit, and a spinner turns four steps a second.
- **Usage time counts reading, not a screen left open.**
  In the TIME of `revu usage`: a screen stops counting a minute after your last key, click or wheel, or when the terminal loses focus.
  Days counted before keep their actions, and the report leaves out their time and hints.
- **Usage hints fire only where a faster key would have helped.**
  In the diff and the queue, with `[usage] enabled = true`.
  The first three times a hint fires, a toast names the key to try.
- **Usage counts a key by what it did.**
  In `revu usage`: `q`, `esc` or `x` closing a pane counts as `close_pane`, and `enter` on a file or hunk as `fold_toggle`.
- **A config written for a newer revu no longer stops this one.**
  In every command and the TUI: a setting revu does not know is ignored, and a warning names it once at start.
  An old `[ai] enabled` or `provider` now warns and points at `[ai.anthropic]` and `[ai.typesafe]`.
- **Refreshing comes back after a sleep or a dropped VPN.**
  In the queue and an open MR: a request that gets no answer gives up, the status line says offline, and polling tries again.
- **Opening an agent session no longer waits forever.**
  On `a g` and `:session`: a stuck `git fetch`, tmux or terminal is stopped and the toast says which.
- **A draft you deleted or edited is never published as it was.**
  When deleting or editing a draft fails, for example while you are offline.
  The draft comes back as the forge holds it, your new text waits in the box, and `P` waits for `r`.
- **A comment survives an editor that exits with an error.**
  When you write in `$EDITOR` with `E` or `ctrl-o`, and vim quits with `:cq` or after an error.
  The text waits in the comment box, and `enter` saves it.
- **A refused approval no longer reads as a failed publish.**
  On GitLab, when you publish and approve with `P`, `a`, `enter`.
  The comments are published and leave the list, and the warning says why the approval was refused.
- **Notes saved together on a GitHub PR all reach it.**
  On GitHub, when several drafts save at once, as `r` does after you were offline.
  Each draft now waits for the one before it, so none overwrites another.
- **A big MR opens faster.**
  In the diff: folded files are coloured only once unfolded, and an unchanged MR opens without fetching its diff again.
- **An open MR at rest stays quiet.**
  In the thread pane and under lines, where threads are laid out again only when a thread or the age of a note changes.

## 0.7.0 - 2026-10-12

### New

- **See what changed in revu.**
  In the terminal and inside revu.
  `revu news` prints it, `:news` opens it, and `revu update` ends with what is new for you.
- **Read the threads of a line in the diff, under that line.**
  In zen and in a window under 120 columns, one row per thread.
  `enter` on the line unfolds its threads, and `enter` on a thread folds or unfolds that one.
- **Write a comment in the diff, where it will land.**
  In zen and in a window under 120 columns, for a new comment, a reply and an edited draft.
  `c` on a line, `r` on a thread or `e` on your draft opens the box under it.
- **Reply to a thread with the comment key.**
  On a thread, in the thread pane and in the diff.
  `c` replies as `r` does, and still starts a new comment on a line of code.
- **A narrow window gives its whole width to the diff.**
  In a window under 120 columns, with an MR open: the queue hides and one line number shows.
  `h` or `esc` brings the queue back, and `l` or `enter` returns to the diff.
- **Choose the widths where the layout changes.**
  In the config, under `[tui]`.
  `narrow_width` sets where a window counts as narrow, 120 by default, and `wide_width` where three columns fit, 150 by default.
- **Pick and open a row with the mouse.**
  In the queue, the file tree, the outline and the list of every thread.
  Click a row to pick it, and click it again to open it.

## 0.6.0 - 2026-10-06

### New

- **Read a Markdown file as a page, with its changes marked.**
  In the diff, on a Markdown file.
  `v` opens the page, `v` again goes back, and `zR` shows the parts that did not change.
- **See the old page beside the new one.**
  In the Markdown page that `v` opens.
  `D` puts them side by side, and `D` again goes back to one column.
- **Move a full page at a time.**
  In the queue, the diff, the thread pane, the cover and the key list.
  `ctrl-f` goes down and `ctrl-b` goes up, as `PgDn` and `PgUp` do.

### Fixed

- **A signature that only moved between one line and several no longer shows as changed.**
  In the outline that `O` opens.

## 0.5.0 - 2026-10-02

### New

- **Keep an MR on top of the queue.**
  In the queue, under PINNED, until the MR is merged or closed.
  `b` pins or unpins it, and `:pin` and `:unpin` do the same from the diff.
- **Code in comments takes the colours of the diff.**
  In the thread pane, in code blocks and suggestions.
  Name the language at the top of a code block, as on the forge.

### Fixed

- **Scrolling a big MR no longer lags.**
  In the thread pane, the file tree and the pipeline pane.

## 0.4.0 - 2026-10-02

### New

- **See the functions and classes an MR changes, and how they call each other.**
  In the right pane, for TypeScript, JavaScript and Python files.
  `O` or `:outline` opens it, `enter` goes to the code, and `t` switches to a flat list.

## 0.3.0 - 2026-09-30

### New

- **Read only the new code, or only the old code.**
  In the diff.
  `>` shows the code after the change, `>` again the code before, and `<` goes the other way.

## 0.2.0 - 2026-09-30

### New

- **Install revu with Homebrew.**
  On macOS.
  Run `brew install vmeyet/tap/revu`, and `revu update` upgrades it later.
- **One queue for GitLab and GitHub.**
  In the left pane, with every host you logged in to.
  `revu login --from-glab` and `revu login github.com --from-gh` reuse the tokens you already have.
- **The queue keeps to what needs you.**
  In the queue, where stale, approved and unready MRs move to OTHER.
  `[queue.rules] enabled = false` shows them all again.
- **Filter, sort and group the queue.**
  In the queue.
  `/` filters with words like `@nina` or `~infra`, `s` sorts, `S` groups by author, and `*` shows every project.
- **Save a filter as a view.**
  In the queue, from `[queue.views]` in the config.
  `'` then the first letter of a view applies it.
- **Read a diff with folds and syntax colours.**
  In the diff.
  `]c` and `[c` jump between changes, `tab` goes to the next file, and `za` folds.
- **See the old file beside the new one.**
  In the diff.
  `D` switches between inline and side by side.
- **Wrap long lines, hide whitespace changes and show more lines around a change.**
  In the diff.
  `w` wraps, `W` hides whitespace changes, and `=` shows more lines.
- **Find text anywhere in the MR.**
  In the diff, in folded files too.
  `/` searches, then `n` and `N` go to the next and previous match.
- **Comment on a line, a range or with a suggestion.**
  In the diff, as drafts only you can see.
  `c` comments, `V` selects lines first, and `s` suggests a change.
- **Publish every draft as one review.**
  In the diff and the thread pane.
  `P` lists your drafts, then `a` approves with it and `enter` publishes.
- **Post a comment at once, without a draft.**
  In the comment box.
  `⌘enter` posts it, or `ctrl-s` where the terminal keeps `⌘enter`.
- **Read and answer threads beside the code.**
  In the right pane, from a line with a mark.
  `enter` opens the threads, `r` replies and `R` resolves.
- **Walk the threads of an MR.**
  In the diff and the right pane.
  `]n` and `[n` jump between open threads, `T` lists every thread, and `m` keeps yours.
- **React to a comment.**
  In the thread pane, and in the diff on a line with a thread.
  `+` opens the reactions, then `1` to `8` picks one.
- **Apply a suggestion.**
  In the thread pane, on a comment that holds a suggestion.
  `S` applies it after a `y`.
- **Approve, merge, and mark your MR draft or ready.**
  In the diff, and from the shell.
  `A` approves, `M` merges yours, `H` switches draft and ready, like `revu approve`, `revu merge` and `revu ready`.
- **See an MR at a glance on its cover page.**
  Over the queue or the diff.
  `i` opens the description, the checks, the reviewers and the open threads.
- **See the pipeline and its failed jobs.**
  In the right pane.
  `p` opens it, and `o` opens a job in the browser.
- **Browse the files as a tree and mark them viewed.**
  In the right pane and the diff.
  `t` opens the tree, and `zv` marks a file viewed.
- **Zen shows the diff alone.**
  In the diff, and at start with `revu 42`.
  `zz` enters and leaves it, and `]m` and `[m` change MR without leaving.
- **Go through the MRs that need you like an inbox.**
  In the queue and the diff.
  `]r` opens the next MR that needs you, and `[r` the previous one.
- **One search for MRs, files and commands.**
  Anywhere in revu.
  `⌘K` or `ctrl-k` opens it, `/` first finds files, and `:` runs a command.
- **Open a file in your own editor.**
  In the diff, at the line under the cursor.
  `ctrl-v` opens it, and `:view old` opens it as it was before the change.
- **Ask Claude about the diff.**
  In the diff and the thread pane, once `[ai.anthropic]` is switched on.
  `a` then `e` explains, `a` then `r` lists risks, and `a` then `s` sums up the MR.
- **Share an MR to a place you choose.**
  In the queue and the diff, through a `[share]` command.
  `Y` or `:share` shows the message first, then `y` sends it.
- **Scroll with the wheel and copy by dragging.**
  In every pane, with the mouse.
  Drag over code or over a comment to copy it.
- **Get a notification when an MR needs your review.**
  On macOS, while revu runs.
  `[notify] enabled = false` turns it off.
- **Add your own keys, or use an AZERTY keyboard.**
  In the config, under `[keys]`.
  `[keys.bind]` adds keys, and `layout = "azerty"` makes `(` and `)` work like `[` and `]`.
- **Pick a theme.**
  Anywhere in revu.
  `:set theme=nord` changes it and saves it.
- **See the keys of the pane you are in.**
  Anywhere in revu.
  `?` shows them, and `?` again shows every key.
- **Review from the shell.**
  In the terminal, for scripts too.
  `revu list`, `revu show`, `revu diff`, `revu comment` and `revu publish` work without the screen, and `--json` prints data.
- **See which keys you use.**
  In the terminal, once `[usage] enabled = true` is in the config.
  `revu usage` prints what you use most and what you never use.
