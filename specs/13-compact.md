# 13 · Compact layout

Goal: review from a phone.
revu runs inside tmux on the Mac and is reached from a phone terminal over mosh or ssh, often in a session shared with the desk.
A phone held upright gives 45 to 60 columns, and 12 to 20 rows once its keyboard is up.

## When it applies

A third width tier, under narrow and wide (`09-thread-pane.md` § Width).

- `[tui] compact_width` (70): below it the window is compact. It is at least 1 and never above `narrow_width`; unset, it follows a `narrow_width` set under 70.
- `[tui] layout`: `auto` (default) follows the width, `compact` forces the layout at any width, `full` never uses it. Narrow and wide still follow the width under `full`.
- The tier is decided at every frame, like narrow: turning the phone, or a desk client taking the tmux window over, changes layout with no restart.
- `Tui::widths()` folds `layout` into `Widths.compact`: `u16::MAX` under `compact`, 0 under `full`. Nothing else knows the setting.
- Compact is narrow too: the threads of a line read in the diff and the compose box opens there (`09-thread-pane.md` § Threads in the diff).
- Zen keeps its own column and its one-number gutter at any width; the rest of this page is outside zen.

## One page at a time

Never two columns.

- The queue is the page while it has the keys, over the whole width, with or without an MR open.
- Opening an MR (`enter`, `l`, a second click on its row) makes the diff the page; `h` or `esc` brings the queue back.
- The right pane (threads, tree, pipeline, outline, answer) is the page while it has the keys, as under narrow.
- The frames stay: the title of a pane says which page this is, and it costs two rows and four columns.

## The queue

Two lines per MR, whatever `[tui] queue` says: one line leaves a title no room at 45 columns.

```
│ ▎ feat charge cards at checkout                │
│ ▎ omar · !42 · +412 −38                 2h     │
```

- Line one: the kind chip and the title, alone.
- Line two: who, the number, the size, the age, then the marks a wider queue puts after the title (Jev's, the approval, the badge).
- A stack follows the same rule.
- The cursor, click to pick, the wheel, grouping and PINNED work as on a wide screen: a row's click area is its two lines.

## The diff

- Long lines wrap by default. `w` still flips it, and what `w` chose holds in every layout until revu quits; before any `w`, compact wraps and the other tiers cut.
- The gutter is the sign alone (`Gutter::Sign`): no line number, so code starts four columns in.
- The cursor's line number reads at the right of the pane's title, `L12`: the new side's, the old one's on a removed line, the line a thread row hangs under. Nothing shows on a file, a hunk or the MR row.
- The title is on the frame, so the number survives a short screen and never hides code.
- A file path too long for its row is cut in its middle (`services/pay…/charge.rs`), so the folder and the name both read; the other tiers keep cutting its end.

## A short screen

Under `SHORT_BELOW` (20) rows, in the compact layout only.

- The status row is dropped, as in zen. A question (a confirmation, the quit prompt, the reaction picker, the `'` views, the offer of the next MR) and the search bring it back while they wait; the queue filter keeps its own row.
- A fresh toast shows on the bottom row of the page for two seconds, as in zen.
- The MR header (two rows) is dropped from the diff.
- Not shown there: the host and user, the count of unresolved threads and drafts, `offline`, the rate limit and `loading`.

## The tap bar

Shifted keys, `ctrl-` keys, `esc`, `[` and `]` are slow on a phone keyboard, and a tap arrives as a mouse click.
One bottom row of labels, under the status row, in the compact layout only; `[tui] tap_bar = false` hides it.

| Page | Labels and the keys they press |
|---|---|
| Queue | open `enter` · filter `/` · refresh `r` · copy `y` · more `:` |
| Diff | file `tab` · thread `]n` · viewed `zv` · comment `c` · back `h` · more `:` |
| Thread: a thread row in the diff, or the thread pane | reply `r` · resolve `R` · react `+` · copy `y` · back `h` · more `:` |
| Pane: tree, pipeline, outline, answer | back `h` · more `:` |

- A label is a key list (`src/tui/app/taps.rs`). A click on it goes through `handle_key`, key after key, so it does what the keys do, user bindings and usage counts included.
- `more` is the last label of every page: it opens the palette on commands, where `approve`, `publish` and the rest are. Approve and publish get no label of their own: seven letters each, they do not fit beside the others at 45 columns.
- `copy` is `y`, the link of the MR or the thread: a link opens on the Mac, so the phone takes it by the clipboard. The diff has no room for it at 45 columns and leaves it to `more` (`:copy`).
- `back` is `h`: one page back. On a thread row in the diff that is the queue, as on any other row of the diff.
- Each label sits in a cell one blank wider on each side, and the cells share the columns left over, so a thumb has more to hit. The whole cell answers.
- A row too narrow for the labels drops them from the end, keeping `more`.
- It costs one row, kept on a short screen: there it is the only row around the page.
- It gives its row up, and its labels answer nothing, while a question waits (a confirmation, the quit prompt, the reaction picker, the `'` views, the next MR offered), while the search or the filter is typed or kept, while a comment is written, and under an overlay.
- It shows in zen too: `revu 42` starts there, and a phone needs the labels as much.

## The short key list

In the compact layout the first `?` lists one group, named after the page: the keys of its tap bar and what each does.
The second `?` lists every key, as everywhere; no key is taken away, the list is only shorter.

## A plain key for the search

`,` opens the palette on MRs, as `ctrl-k` does, in every layout.
It is the free key a phone keyboard reaches soonest: every lowercase letter but `f` is taken, and `f` is the key the tests and the docs give to the user.
It is a second key of the `jump` action: a key bound to `jump` under `[keys.bind]` stands for both, and `,` can no longer be bound to something else.

## Motion

A phone link pays for every repaint, so the compact layout moves less than the others.

| | Wide, narrow, zen | Compact |
|---|---|---|
| Spinner step | 80 ms | 250 ms |
| Next look at a screen that changed | 100 ms | 250 ms, one spinner step |
| Queue marks `●` `◆` | pulse once a second | steady |
| Running pipeline badge | spins | steady `⠋` |

- One rule: `Motion` (`src/tui/motion.rs`), `Lively` or `Calm`, which `App::motion()` reads from the tier of the last frame. The spinner, the loop's tick and the queue marks ask it; none of them looks at the tier.
- It follows the tier at every draw: a window crossing 70 columns changes how it moves with no restart, and `layout = "compact"` is calm at any width.
- A compact queue at rest draws the same buffer second after second, so the loop falls back to its one-second look and sends nothing.
- A screen still at rest is looked at once a second in every tier.

## Unchanged

From 70 columns up, wide, narrow and zen draw exactly as before: their snapshots do not move, but for the `,` beside `^k` in the key list.

## Tests

- `columns()`: the queue page, the diff page, the pane page, zen, and both ends of `layout`.
- Config: the default, a compact width above the narrow one, zero, and the three values of `layout`.
- Snapshots on `TestBackend` at 50×30 (queue, diff) and 50×14 (diff, short).
- The tier follows the width frame after frame; two-line rows answer a click and the wheel; the cursor's line in the title; wrap by default and `w`; the path cut; the prompts a short screen keeps.
- The tap bar: the labels of each page, a click on each label against its keys typed, the row given up to a question and to typed text, the setting, a bar too wide, and every set fitting 45 columns with keys that parse.
- The short `?` list and the full one after it; `,` against `ctrl-k`.
- Motion: both values of the spinner step and the tick; a wide and a narrow queue pulse and spin; a compact one, by width and forced, draws one buffer over three seconds; crossing the width switches.

## Considered

- **No frames in compact**, as zen: four more columns of code and two more rows. Rejected for now: the title carries the page's name, the MR and the cursor's line, and every pane would need a second look.
- **The cursor's line at the end of its own row**: it hides the end of the code it numbers.
- **A separate `wrap` setting**: `w` already says it, and a default that follows the tier needs no key in the config.
