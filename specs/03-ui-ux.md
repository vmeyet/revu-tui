# 03 · UI, UX, DX

This is the document a designer reads first.
The terminal has no pixels to spare, so the design is typographic: alignment, rhythm, weight and colour carry everything.

## Design rules

1. **One accent.** The theme's `accent` marks the selection bar, the focused pane title and counts that matter. Nothing else uses it.
2. **Three text weights.** `fg` for content, `muted` for metadata (times, counts, hints), `faded` for structure (section headers, unfocused panes). Bold is for the selected row's title and a file name, never for a paragraph.
3. **Alignment before colour.** Every column has a fixed width; numbers are right aligned; names are padded, never truncated mid-glyph. A misaligned gutter is a bug.
4. **Whitespace is a feature.** One empty row between files, one between hunks, a `Padding::horizontal(1)` in every pane. Cramped is not dense.
5. **Colour means one thing.** `success` = passing or approved, `warn` = pending or draft, `danger` = failing, conflicting or removed. Added is `success`, removed is `danger`, and both keep their `+`/`-` sign so colour is never the only cue.
6. **Unfocused panes fade** (`fade()` from slack-tui) so the eye finds focus without a border colour change.
7. **The terminal keeps its background.** Themes set foregrounds and two surfaces only, exactly as in slack-tui, so the app matches any terminal.
8. **Glyphs have fallbacks.** `[tui] ascii = true` swaps `▎ ◆ ◇ ● ▸ ▾ ✓ ✗ ⠋` for `| * o * > v + x -`. Nerd Font icons are not used.
9. **Motion is calm.** Spinners at 80 ms, the `●` pulse at 1 s, nothing else moves. No sliding, no blinking text.
10. **Nothing modal without a way out on `esc`.** Every overlay closes on `esc`, and the status line says so.

The screenshot test: open `acme/widgets!42` in Ghostty with `tokyonight`, resize to 120×40, take a screenshot, put it next to GitHub's and delta's diff views.
It has to look like it belongs there.

## Layout

```
┌ Queue ──────────────────────────┐┌ acme/widgets!42 · feat: charge cards at checkout ────────────────────┐┌ Thread ──────────────────┐
│ TO REVIEW                    3  ││ nina · feat/checkout → main · 2h · +412 −38 · 9 files · ✓ pipeline   ││ src/pay/charge.rs:57     │
│ ▎!42 charge cards at checkout   ││ ▸ 2 of 2 approvals · 3 threads, 1 unresolved · 2 drafts               ││                          │
│   !40 rename Invoice→Bill    ✗  ││                                                                        ││ nina · 2h                │
│   !39 bump rustls             ● ││ ▾ src/pay/charge.rs                              +120 −4  ◆2 ◇1     ││ Should this retry on     │
│ MINE                         2  ││   ▾ @@ -12,7 +12,9 @@ pub async fn charge                              ││ timeout? Stripe says     │
│   !41 fix flaky cache test      ││    12  12   pub async fn charge(card: &Card, amount: Money) -> Result<  ││ idempotency keys are     │
│   !37 docs: review guide     ✓  ││    13       -    let client = Client::new();                          ││ safe to reuse.           │
│ WATCHING                     1  ││        13   +    let client = Client::with_key(idempotency_key(&card)); ││                          │
│   !35 infra: new runner         ││        14   +    let attempt = Attempt::first();                        ││ you · draft         ◇    │
│                                 ││    14  15       let response = client.charge(amount).await?;           ││ Yes, keys are per card   │
│                                 ││   ◆ nina · Should this retry on timeout? · 1 reply                    ││ and amount, see line 13. │
│                                 ││    15  16       audit::record(&response);                              ││                          │
│                                 ││   ▸ @@ -40,6 +42,20 @@ fn idempotency_key           (14 lines)         ││                          │
│                                 ││                                                                        ││                          │
│                                 ││ ▸ src/pay/mod.rs                                  +3 −1     viewed     ││                          │
│                                 ││ ▸ Cargo.lock                                     +200 −20   folded     ││                          │
└─────────────────────────────────┘└────────────────────────────────────────────────────────────────────────┘└──────────────────────────┘
 gitlab.com · vivien · 2 drafts · P to publish                                                     ⠋ refreshing · ? help
```

- Left pane: 34 columns, the queue. Hidden in reading mode (`z`).
- Middle: the review, `Min(60)`.
- Right: 32 columns or 40 % of the width, only when a thread, the MR overview, the file tree or an AI answer is open.
- Row above the status line: the input row, only while typing.
- Status line: host, user, pending drafts, then right aligned the poll state and `? help`. Toasts replace the left part for 4 s.

Focus moves with `h` `l` between the three panes, like slack-tui's channels, messages and thread; `Tab` is reserved for the next file inside the review.

## The queue

Inside a GitLab checkout the queue shows that project only, named in the pane title (`Queue · acme/widgets`); outside one, or after `*`, it shows every project (`Queue · all`).
Each scope has its own cache file, so a switch paints the right list at once and never the other one.
Scoped, an `OPEN` section lists the project's other open MRs, between `WATCHING` and `DONE`.

Sections are uppercase `faded` headers with a right aligned count.
A row is `▎` bar when selected, `!iid` in `muted`, the title, then one badge column at the right edge:

| Badge | Means |
|---|---|
| `✗` danger | pipeline failed or conflicts |
| `⠋` muted | pipeline running (spinner) |
| `✓` success | approved by me |
| `●` accent | activity since I last opened it |
| `◆` warn | waits on me (Jev `waits_on_me`, M4) |
| `D` muted | draft MR |

Rows sort by `updated_at` desc inside a section; M4 adds the Jev urgency sort in `To review`.
`Done` is the last section, folded, with a count; `zo` on its header opens it.
Each `!iid` is a terminal hyperlink (OSC 8) to the MR: the loop prints it again over the drawn cells after every frame, only where the cells still spell it, and never over a modal.
`i` opens the description modal: `!iid title`, author, branches, labels, then the description as light markdown; `j k ^d ^u g G` scroll, `o` opens the MR, `esc` `i` `q` close.
The filter `/` narrows rows by title, author and iid, live.

Empty queue:

```
           ┌──────────┐
           │  ✓       │
           └──────────┘
        nothing waits on you
     r to refresh · / to filter
```

Loading: the sections render with three skeleton rows of `▁▁▁▁▁` in `faded`.

## The review pane

### Header (2 rows, `▸` folds it to 1)

Row 1: author, `source → target`, age, `+adds −dels`, file count, pipeline glyph and word.
Row 2: approvals `n of m`, thread counts, drafts count. Anything at zero is omitted.
When folded: `nina · +412 −38 · ✓ · 2 drafts`.

### Files

A file row is `▾`/`▸`, the path with the directory in `muted` and the basename in `fg` bold, right aligned `+adds −dels`, then anchors: `◆n` published threads, `◇n` drafts, and a state word: `viewed`, `folded`, `binary`, `too large`, `renamed from x`, `deleted`.
Renames show `old → new`; a pure mode change shows `mode 644 → 755`.

### Hunks

A hunk row is `▾ @@ -a,b +c,d @@ context`, `muted`, with the function context in `fg`; folded it appends `(n lines)`.
Between two hunks the elided lines show as `· · ·  38 lines` in `faded`; `+` on that row expands 10 more lines of context above and below (fetched from the file blob, M3).

### Lines

```
 old  new   text
  12   12   pub async fn charge(card: &Card, amount: Money) -> Result<
  13        -    let client = Client::new();
       13   +    let client = Client::with_key(idempotency_key(&card));
```

- Two 4-column right aligned gutters in `faded`; the selected line's gutters turn `fg`.
- Sign column, 1 char, coloured; the text keeps the sign colour at 100 % for changed words and at 60 % (mixed toward the ground) for the rest of the line.
- Removed and added lines get a fill edge to edge: `mix(ground, danger, 10)` and `mix(ground, success, 10)`, with the terminal's own text colour on top; changed words get `mix(ground, colour, 25)` in the sign colour.
- The ground is the theme's `base` when it is RGB. Otherwise revu asks the terminal at startup (OSC 11, then DA1 so a silent terminal still ends the read, 50 ms cap, `select(2)` on `/dev/tty`) and `Theme::with_ground` computes the same fills from its answer.
- No answer (tmux without passthrough, older terminals): no fill at all. Only the sign column is coloured, the text keeps the terminal colour and changed words go bold, which leaves the text free for syntax colours.
- Every palette carries these six colours (`added`, `removed`, the two fills, the two word fills), so a theme decides the diff look, not the renderer. A key typed in the first ~50 ms after launch may be read with the answer and lost.
- Context lines are `fg` with no surface.
- Tabs render as `→   `, trailing whitespace as `·` in `warn`, both only on changed lines.
- Long lines are cut with `…`; `w` wraps them with a hanging indent under the text column.
- `W` hides whitespace-only changes (the line shows as context with a `≈` sign).
- The selected line has the `▎` bar and, if the theme has `highlight`, the fill.

### Inline pairs

A removed line and its added twin read as one row when the change is small:

```
 old  new   text
   3    3 ~    let b = 2;20;
```

- The rule: the pair comes from an equal run of `-` and `+` lines (the word-diff pairing), each side changes at most `[review] inline_max_words` runs of words (default 2), and both lines keep at least `[review] inline_min_same` percent of their bytes (default 60). Anything bigger stays split, so a rewrite never turns into a puzzle.
- Both gutters show, the sign is `~` in `warn`, the kept text is plain, each old word is struck through in the removed colours and followed by its replacement in the added colours (the theme's word fills behind them on RGB themes).
- `c` comments on the new side, `C` on the old side; `V` counts the pair as its added line. Threads and drafts on either line hang under the pair.
- `D` switches between inline and split, remembered per MR like folds; inline is the default.

### Anchors in the flow

A thread renders as one collapsed row under its line: `◆ author · first line of the note · n replies`, `resolved` ones in `faded` with `✓`.
`enter` on it opens the right pane. A draft renders the same with `◇` and `you`.
Outdated threads sit in a `outdated` block at the file's tail.

### Visual select

`V` starts a range from the current line; `j/k` extend it; the range fill uses `highlight` or, without one, inverts the gutters.
`c` comments on the range, `y` yanks it as text, `esc` drops it.

## The right pane

One of:

- **Thread**: path:line title, then notes as `author · age` in `muted` and the body as markdown (bold, code, lists, quotes; links show their label with `u` to open). The reply input is the input row. `R` toggles resolved. Drafts in the thread show `◇`.
- **Overview** (`o` on the header, or on open when there is no thread): description as markdown, labels, reviewers with their state, approvals, pipeline link, then the activity list (system notes) in `muted`.
- **Files** (`t`): a tree with directories folded by default beyond depth 2, same badges as file rows, `enter` jumps, `zv` marks viewed.
- **AI answer** (M4): title `ask · file` and the streaming markdown; a `cached` tag when served from cache.

## Publish modal (`P`)

```
┌ Publish review ─────────────────────────┐
│                                          │
│  3 drafts on 2 files                     │
│                                          │
│  ▸ src/pay/charge.rs:13  Yes, keys are…  │
│  ▸ src/pay/charge.rs:57  Consider a re…  │
│  ▸ src/pay/mod.rs:4      nit: unused     │
│                                          │
│  [x] approve                             │
│                                          │
│         enter publish · esc back         │
└──────────────────────────────────────────┘
```

`a` toggles approve; `j/k enter` opens a draft to edit; `d` deletes one.
While publishing, the modal shows the spinner and disables keys; a failure keeps the drafts and toasts the reason.

## Keys

Marked `M1` `M2` `M3` `M4` by milestone. Everything is in `?`.

### Everywhere

| Key | Action | |
|---|---|---|
| `j` `k` `↓` `↑` | move | M1 |
| `g` `G` | first, last | M1 |
| `ctrl-d` `ctrl-u` | half page | M1 |
| `h` `l` | focus left, right pane | M1 |
| `enter` | open the thing under the cursor | M1 |
| `esc` | close overlay, drop selection, go back | M1 |
| `o` | open in browser | M1 |
| `y` | copy the URL (line URL in the review) | M1 |
| `r` | refresh | M1 |
| `/` | filter (queue) or search text (review) | M1 |
| `*` | queue: this checkout's project, or every project | M3 |
| `i` | the MR description, in a modal | M3 |
| `:` | command line | M3 |
| `ctrl-k` | jump to an MR or a file | M3 |
| `z` | reading mode | M3 |
| `?` | help | M1 |
| `q` | quit (`ctrl-c` always) | M1 |

### Review

| Key | Action | |
|---|---|---|
| `Tab` `S-Tab` | next, previous file | M1 |
| `]c` `[c` | next, previous change (hunk) | M1 |
| `]n` `[n` | next, previous thread or draft | M1 |
| `]f` `[f` | next, previous file with unresolved threads | M1 |
| `za` | toggle fold under the cursor | M1 |
| `zc` `zo` | close, open | M1 |
| `zM` `zR` | fold, unfold every file | M1 |
| `zv` | mark viewed (folds) | M3 |
| `t` | file tree | M3 |
| `w` | wrap long lines | M3 |
| `W` | hide whitespace-only changes | M3 |
| `+` | more context around the hunk | M3 |
| `c` | comment on the line (draft) | M2 |
| `V` | select lines | M2 |
| `E` | comment in `$EDITOR` | M2 |
| `s` | suggestion: editor prefilled with the lines | M2 |
| `d` | delete the draft under the cursor | M2 |
| `P` | publish modal | M2 |
| `A` | approve, unapprove | M2 |
| `a` | ask: `e` explain, `r` risks, `s` summary, `t` thread, `c` comment, `a` free | M4 |

### Thread

| Key | Action | |
|---|---|---|
| `r` | reply (draft) | M2 |
| `R` | resolve, unresolve | M2 |
| `u` | open the first link | M1 |
| `e` | edit my note | M3 |

### Command line (`:`)

`:go acme/widgets!42`, `:open`, `:approve`, `:publish`, `:reply <text>`, `:draft <text>`, `:resolve`, `:viewed`, `:set theme=nord`, `:ai off`, `:ask <text>`, `:cache clear`, `:help`, `:quit`.

## Toasts and errors

A toast is one line in the status area, `accent` for success, `danger` for failure, 4 s, replaced by the next.
Failure toasts end with the key that retries (`r`) or the command that fixes it (`revu login`).
A network failure while browsing does not clear the screen: the stale view stays, the status line says `offline · last refresh 3m ago`.

## Performance budgets

| Measure | Budget |
|---|---|
| Startup to first frame (cached queue) | 30 ms |
| Open a cached MR | 50 ms |
| Draw at 120×40 with a 300-file MR open | 4 ms |
| Key to frame | one loop turn, no awaited network |
| Memory with a 5 000-line diff | under 50 MB |

Diff rows are produced lazily from `Review + FoldState` for the visible window, never materialised for the whole MR.

## Accessibility and terminals

- Every colour cue has a glyph or a sign next to it.
- Contrast: every theme's `muted` on `base` clears 4.5:1; check with the `mix()` math, not by eye.
- Kitty keyboard protocol is pushed when available (as slack-tui) so `S-Tab`, `ctrl-k` and `⌘k` arrive.
- Mouse: wheel scrolls the pane under the pointer, click focuses and selects, nothing else. Keyboard remains complete.
- Minimum size 80×24; below that the queue hides and a one-line notice says so.

## DX for the person running it

- `revu` with no subcommand opens the TUI.
- `revu list` and `revu show <ref>` print aligned tables, `--json` for scripts; `revu diff <ref>` prints the coloured diff to a pager (`$PAGER`, default `less -R`).
- `revu comment <ref> <path>:<line> <text>` and `revu approve <ref>` for scripts and other agents.
- `<ref>` accepts `group/project!42`, `!42` (current repo from `git remote`), an MR URL, or nothing (current branch).
- Error lines are `✗ message` plus dimmed causes, same as slack-tui.
- `revu --version` prints the crate version and the commit; `revu update` rebuilds from the repo.
