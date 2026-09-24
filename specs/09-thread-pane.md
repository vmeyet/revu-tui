# 09 · Threads in the right pane

Comments stop living inside the code.
The diff only marks the lines that carry a conversation; the conversation itself opens in a pane on the right, like a thread in slack-tui.
Replies and new comments are written there too, in a compose box under the thread, never in the middle of the diff.

## Why

Today every thread and draft is a row inserted under its line (`Row::Thread`, `Row::Draft`, `Row::Outdated`).
On a busy MR the code reads like a chat log: hunks stretch, line numbers stop being contiguous, and `j` walks through comments to reach code.
The one-row input under the panes is far from the thread it answers and cannot hold more than a line.
Slack-tui already solved this shape: the channel on the left, the thread on the right, the answer typed under the thread.

## Layout

```
┌ Queue ─────────────────────┐┌ acme/widgets!42 · feat: charge cards ──────────────┐┌ charge.rs:57 · 2 threads ────────────┐
│ TO REVIEW               3  ││ ▾ src/pay/charge.rs                  +120 −4  ◆2 ◇1 ││ ◆ unresolved                          │
│ ▎!42 charge cards          ││   ▾ @@ -40,6 +42,20 @@ fn idempotency_key         ││ nina · 2h                             │
│   !40 rename Invoice…   ✗  ││      54   56   let key = format!("{}-{}",         ││ Should this retry on timeout? Stripe  │
│   !39 bump rustls       ●  ││ ◆2   55   57 ▎ let response = client.charge(…  ││ says idempotency keys are safe to     │
│ MINE                    2  ││      56   58   audit::record(&response);          ││ reuse.                                │
│   !41 fix flaky cache      ││ ◇    57      - retry(3);                          ││                                       │
│   !37 docs: review guide ✓ ││           59 + retry(Policy::default());          ││ you · draft ◇                         │
│                            ││ ✓         60 + log::info!("charged");             ││ Yes, keys are per card and amount.    │
│                            ││                                                   ││ ───────────────────────────────────── │
│                            ││ ▸ src/pay/mod.rs                   +3 −1  viewed  ││ ✓ resolved by nina · 1 note           │
│                            ││                                                   ││ ╭─ reply to nina ─────────────────────╮│
│                            ││                                                   ││ │ And the amount is in cents, so a    ││
│                            ││                                                   ││ │ retry never double charges.▏        ││
│                            ││                                                   ││ ╰─ enter save draft · ⌥enter newline ─╯│
└────────────────────────────┘└───────────────────────────────────────────────────┘└───────────────────────────────────────┘
 gitlab.com · vivien · 3 drafts · P to publish                                                              ? help
```

## The diff: one anchor column, no inserted rows

- `Review::rows()` never emits `Row::Thread`, `Row::Draft` or `Row::Outdated` again; those variants are deleted.
- Every line row gets a two-cell **anchor column** at its far left, before the old line number.
- The first cell is a glyph for the most pressing thing on that line, in this order:

| Glyph | Colour | Means |
|---|---|---|
| `◆` | `warn` | at least one unresolved thread |
| `◇` | `accent` | at least one draft of mine, no unresolved thread |
| `✓` | `faded` | only resolved threads |
| ` ` | | nothing |

- The second cell is the count of threads and drafts on the line when it is more than one (`2`…`9`, then `+`); otherwise a space.
- A draft that has not reached the forge yet (`id: None`) shows its `◇` in `danger`, so a failed save is visible without opening the pane.
- A range comment marks its **end line**, where both forges anchor it; while its thread is focused in the pane, the other lines of the range show `│` in `accent` in the anchor column.
- An inline pair (`Row::Pair`) carries the marker of both its lines: it is one row, so it shows the most pressing of the two and the sum of the counts.
- File rows keep their `◆n ◇n` counts and add `· n outdated` when some threads no longer match the diff.
- The MR header shows the count of MR-level threads (`◆ 2 on the MR`).

The anchor column is always drawn, even when empty, so gutters never shift when a comment arrives.

## The right pane

### When it opens

- It never opens by itself: a pane that pops up while scrolling makes the code jump.
- `enter` or `l` on a marked line opens it on that line's threads, focused.
- `c` on any line opens it with the compose box ready for a new thread (see Composing).
- `enter` on the MR header opens the MR-level threads; `enter` on a file row with outdated threads opens those.

### While it is open

- It **follows the cursor**: moving onto another marked line swaps the pane to that line's threads without moving focus.
- On an unmarked line it keeps showing the last threads, with its title in `faded` and a `↑ line 57` hint, so reading the code around a thread keeps the thread in view.
- `esc` or `x` from the diff or the pane closes it; `esc` in the diff with the pane closed goes back to the queue, as today.

### Width

- Slack-tui's rule: 40 % of the width, at least 36 columns.
- 150 columns and more: queue, diff and pane side by side.
- 120 to 149 columns: the queue hides while the pane is open, and comes back when it closes.
- Under 120 columns: the pane takes the whole review area, like a page; `h` or `esc` returns to the diff at the same line.
- Reading mode (`z`): no queue and no pane beside the diff; `enter` on a marked line opens the pane the narrow way, full width.

### What it shows

- One line at a time: every thread and draft anchored on the current line (both sides of a context line, both lines of an inline pair).
- Title: `charge.rs:57 · 2 threads` (`charge.rs:-57` for the old side, `on the MR`, or `charge.rs · outdated`).
- Threads in order: unresolved first, then drafts that start a new thread, then resolved; inside a group, oldest first.
- A thread is a status line (`◆ unresolved`, `✓ resolved by nina`, `◇ draft`), then its notes: `author · age` in `muted` (the author in `theme.user(name)`, `you` for me), then the body in the light markdown `thread_view.rs` already renders (bold, code spans and fences, lists, quotes, tables; links show their label, `u` opens the first one).
- A suggestion block renders as a small diff (`-` old lines, `+` new lines) with the diff colours, not as a fenced block.
- A picture (`![alt](url)` or GitHub's `<img src>`) sits under the line it was written in: a thumbnail up to 60 × 14 cells on terminals that draw pictures (Kitty, Ghostty, WezTerm, iTerm2), else one `[image: alt]` line that clicks through to it. `[tui] images = false` always shows the line.
- Only the forge's own pictures are fetched: GitLab uploads through `GET /projects/:id/uploads/:secret/:file` with the token; GitHub attachments on the web host with the token, then the signed `*.githubusercontent.com` or S3 link it redirects to with a client that carries no token; repo files through the contents API. Anything else (badges, other sites) stays a line, so a comment cannot make revu call out. Pictures are capped at 5 MB and 15 s, cached per link on disk (0600), and never drawn under a modal.
- My draft replies sit at the tail of their thread as `you · draft ◇`, `unsaved` in `danger` until the forge holds them.
- Resolved threads fold to their status line and first note; `enter` on one unfolds it.
- A thin `─` rule in `border` separates threads.
- The footer of the list says what else exists in the file: `3 more threads in this file · ]n`.

### Moving inside it

- `h` / `l` move focus between the diff and the pane, as between slack-tui's channel and thread.
- `j` / `k` move the cursor bar note by note; `J` / `K` jump to the next or previous thread; `ctrl-d` / `ctrl-u` scroll half a page; `g` / `G` first and last note.
- The focused thread is the one under the cursor bar; `r`, `R`, `e`, `d`, `o`, `y` act on it.

## Composing

All writing happens in one compose box at the bottom of the pane; the global input row under the panes is only for `/`, `:` and the publish modal.

- The box shows its target in its top border: `new thread · charge.rs:57`, `new thread · charge.rs:55–57`, `reply to nina`, `edit draft`.
- It starts one row high and grows with the text up to 8 rows or 40 % of the pane, then scrolls.
- `enter` saves the draft; `alt-enter`, `shift-enter` (Kitty protocol) or `ctrl-j` insert a newline.
- `E` moves the text to `$EDITOR` (then back into the box, or straight to a draft when the editor exits with content), as today.
- `s` on a line or a `V` range opens the box prefilled with the suggestion block, and `E` still works on it.
- `esc` leaves the box without losing the text: each target keeps its unsent text for the session, so going back to the line finds it; an empty box simply closes.
- Drafts still go through the forge's draft mechanism (GitLab draft notes, the GitHub pending review) and `P` publishes them all; nothing in this spec changes publishing.

## Keys

| Key | Where | Action | Today |
|---|---|---|---|
| `enter`, `l` | diff, marked line | open the pane on the line's threads, focused | `enter` on a thread row |
| `c` | diff | open the pane with a new thread on the line or the `V` range | input row |
| `C` | diff, inline pair | same, on the old side | input row |
| `s` | diff | new thread prefilled with a suggestion | editor only |
| `E` | diff or box | write in `$EDITOR` | same |
| `r` | pane | reply to the focused thread | input row |
| `R` | pane, or diff on a marked line | resolve, unresolve the focused thread | pane only |
| `e` | pane | edit my draft (or my note, M3) | `enter` on a draft row |
| `d` | pane | delete my draft | `d` on a draft row |
| `J` `K` | pane | next, previous thread on the line | none |
| `]n` `[n` | diff | next, previous marked line, across files; the pane follows when open | next thread row |
| `x`, `esc` | diff or pane | close the pane | `esc` |
| `u`, `o`, `y` | pane | open the first link, open the thread in the browser, copy its link | `u` only |

Muscle memory holds: `c` comments, `r` replies, `R` resolves, `]n` walks the conversations.
`r` keeps meaning refresh in the diff and the queue; it means reply only when the pane has focus, which is where it meant reply before.
The `?` help and the README key table change in the same MR: the thread rows go, the pane keys come in.

## Forges

The pane works on `review::Thread` and `review::Draft` only, so both forges fit without special cases.

- GitLab: a discussion is a thread; an `individual_note` is a one-note thread on the MR; resolvable comes from the first note.
- GitHub: a review thread is a thread (resolved through its node id); a PR conversation comment is a one-note thread on the MR, not resolvable.
- A thread that cannot be resolved does not offer `R`; the hint line in the pane says so instead of failing.

## Tests and TDD order

1. `Review::markers()`: an index `(path, side, line) → Marker { glyph, count }` built once per review; table tests for the glyph order, counts, unsaved drafts, range end lines, both sides of a context line, inline pairs, outdated and MR-level threads kept out of line markers.
2. `Review::rows()` without thread, draft or outdated rows: update the existing row tests; the row count of a review equals files, hunks and lines only.
3. App state machine: `enter`/`l` on a marked line opens and focuses the pane; the pane follows `j`/`k` onto another marked line and keeps the last threads on an unmarked one; `]n`/`[n` across files; `esc`/`x` close; `h`/`l` focus.
4. Compose: `c` opens a box targeted at the line, at a `V` range, at the old side with `C`; `enter` emits `Action::SaveDraft` with the same position as today; `alt-enter` inserts a newline; `esc` keeps the text per target; `E` emits `Action::Compose`; `r` replies to the focused thread; `R` flips and reverts on failure, as today.
5. Layout: the width rules at 160, 130 and 100 columns, and reading mode.
6. Snapshots on `TestBackend`: diff with markers and no pane, pane open on a line with two threads and a draft reply, compose box with three lines, narrow layout at 100 columns, MR-level threads.
7. Theme test: every glyph colour comes from the theme (`no_raw_colors_outside_the_theme` lists the new files).

Delete the tests of the removed rows rather than keeping them alive with shims.

## Considered

- **Keep threads inline and add the pane**: the code would still read like a chat log; the point is that the diff shows code.
- **Open the pane automatically when the cursor lands on a marked line**: scrolling would resize the diff every few lines.
- **List every thread of the file or the MR in the pane**: it loses the line the reader is looking at; `]n` already walks them all, and the file row counts give the overview.
- **A tooltip under the line**: too small for a conversation and nowhere to type.
- **Keep the one-row input for replies**: one line of text, far from the thread it answers.
