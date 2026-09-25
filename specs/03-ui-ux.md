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

Since M3b the diff holds no thread rows: conversations are marked in an anchor column and read and written in the right pane; `09-thread-pane.md` has the layout, keys and width rules.
The mockup below predates it.

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

- Left pane: 34 columns, the queue. Hidden in zen (`zz`), see below.
- Middle: the review, `Min(60)`.
- Right: 32 columns or 40 % of the width, only when a thread, the MR overview, the file tree or an AI answer is open.
- Row above the status line: the input row, only while typing.
- Status line: host, user, pending drafts, then right aligned the poll state and `? help`. Toasts replace the left part for 4 s.

Focus moves with `h` `l` between the three panes, like slack-tui's channels, messages and thread; `Tab` is reserved for the next file inside the review.

## Zen

`zz` hides everything but the diff, and `zz`, `esc`, `h` or `←` bring it back.
No frames, no status line: one faded line on top says which MR, whose, its size, its pipeline and `◆n` unresolved threads when there are any.
The diff sits in a centred column, 70 % of the screen and never under 100 columns, nor wider than the screen; `[tui] zen_width` fixes it instead (60 at least).
Nothing pulses; a toast shows for two seconds on the bottom row, and notifications wait until zen ends.
A question that needs an answer (applying a suggestion, the `'` views) brings the status line back while it waits.
`[m` `]m` open the previous or next MR in the order the queue shows them, without leaving zen, and outside zen as well: filter, sort, sections and stacks all count, folded sections do not.
The arrows mean what they mean outside zen: `←` `→` move focus.
A line on top says where you are for a moment: `‹  !1797 feat add a page  ·  3/12  ›`.
The thread pane still opens with `enter` or `l` on a marked line, as a page of its own, and `esc`, `x` or `q` close it.
It takes the diff's column, with no frame: one faded title line where the zen header sits, then its content from the diff's first column.
The file tree, the pipeline and an AI answer open the same way.

## The queue

Inside a GitLab checkout the queue shows that project only, named in the pane title (`Queue · acme/widgets`); outside one, or after `*`, it shows every project (`Queue · all`).
Each scope has its own cache file, so a switch paints the right list at once and never the other one.
Scoped, an `OPEN` section lists the project's other open MRs, between `WATCHING` and `DONE`.
Other people's draft MRs leave `WATCHING` and `OPEN` for a `DRAFTS` section, folded by default, between `OPEN` and `DONE`; my own drafts stay in `MINE` with `D`. A review request on a draft stays in `TO REVIEW` only with the rules off; with them on it joins `DRAFTS` too.

The "needs me" rules (`[queue.rules]`, on by default) then judge every MR of `TO REVIEW`, `WATCHING` and `OPEN` that is not mine, first reason wins:

| Rule | When | Where it goes | Reason shown |
|---|---|---|---|
| not ready | draft; failing pipeline; a `not_ready` word in the title or the description's first paragraph | `DRAFTS` for a draft, else `OTHER` | `draft`, `pipeline failed`, `"wip" in title` |
| stale | no activity for more than `stale_days` (14) | `OTHER` | `stale 21d` |
| approved enough | at least one approval and none left to give | `OTHER` | `2 approvals, needs none` |
| reviewed by others | `reviewed_comments` (3) or more comments, others commented, I did not | stays, sorted last | `reviewed by 3` |

A review request to me by name pins an MR against stale and approved enough, not against not ready: nobody asks for a review by accident.
A ready source (`[queue.ready] command`) adds `READY` right after `MINE`: every MR link its output names (GitLab `/-/merge_requests/N`, GitHub `/pull/N`), when the MR is someone else's, open, not reviewed by me, and not moved out by a rule, leaves `TO REVIEW`, `WATCHING` or `OPEN` for it. Inside a checkout only that project's links count; outside one, named MRs no list holds are fetched one by one (30 at most). The command runs as words (no shell), 10 s at most, 1 MB of output; its last answer is cached per scope (`ready.<scope>.json`) so the queue paints at once, and a failure keeps it and only warns.
`OTHER` sits last, folded. The selected MR's reason shows in the status line; `revu list` prints it in a last, dim column and `--json` carries it as `reason`.
The rules read only what the queue queries already return, plus `approvalsLeft` and `commenters` on GitLab (the MRs asking me and the project's open ones; each query stays under GitLab's complexity limit of 250) and `participants` on GitHub.
The host tag (`gitlab`, `github`) shows only when rows from several hosts share the queue, never inside a checkout.

A section header is a faded rule, `── OPEN · 28 ────`, filling the pane, with one blank line above it (not above the first); a folded one reads `── ▸ DONE · 3 ──`.
It reads as a break between groups, never as one more row.
A row takes two lines (`[tui] queue = "comfortable"`, the default).
Line one: the `▎` bar when selected, the conventional-commit kind as a coloured chip (`feat` success, `fix` danger, `docs` link, `refactor`/`tech` mention, `test` code, `perf` warn, the rest muted; the scope is dropped), the rest of the title, then Jev's mark, the approval mark and the badge flush right.
Line two, all `faded` so the eye goes from title to title: the author's short name (`romain.courtois` is `romain`: the handle up to its first `.`, `_` or `-`), `!iid`, the host when mixed and `+adds −dels` when they fit, and the age flush right.
`[tui] queue = "compact"` keeps one line: `!iid` in `muted`, the title, then Jev's mark, the approval mark and one badge column at the right edge.
The badge is the most pressing of:

| Badge | Means |
|---|---|
| `✗` danger | pipeline failed or conflicts |
| `⠋` muted | pipeline running (spinner) |
| `●` accent | activity since I last opened it |
| `◆` warn | waits on me (Jev `waits_on_me`, M4) |
| `D` muted | draft MR |

The approval mark is two cells of its own, so a red or busy row still shows it: `✓` in `success` on my MR once the forge would merge it (approved, by at least one person), on anyone else's once I approved it, else blank; a stack shows it when all its MRs do.

Rows sort by `updated_at` desc inside a section, and Jev ranks `To review` by urgency when it is on.
`s` cycles the order inside every section: updated (default), oldest first (created), by author, smallest first (additions + deletions), most urgent first (only when Jev is on). `S` groups `OPEN` and `DRAFTS` by author with a faded sub-header per author. The pane title says both (`Queue · acme/widgets · by author, grouped by author`), and both are remembered per scope in the cache (`queue_view.<scope>.json`).
One author's MRs that build on each other (each targets the branch of the one below it, in one project) are a stack, folded into one row: `▸ feat read shared PDFs` over `romain · 3 MRs · stack` and the newest age.
Its title is the words the MRs' titles share when they share two or more, else the base MR's title; its badge is the most pressing of its MRs'.
`enter`, `zo` or `za` on it unfolds it (`▾`): its MRs follow, base first, joined by a faded `│`; `zc` on the stack or on one of its MRs folds it back and puts the cursor on its row.
Stacks start folded; the unfolded ones are remembered per scope with the order and grouping. Only real chains stack: several MRs by one author that do not target each other stay apart.
On a terminal at least 160 columns wide the queue is 44 columns instead of 34, which shows about twice the title.
Every section folds: `zo`, `zc`, `za` (or `enter` on a folded header) act on the section under the cursor. `Done` and `Drafts` start folded. The cursor skips the headers of open sections and author headers, stops on folded ones, their only row, and opening a section puts it on its first MR.
Each `!iid` is a terminal hyperlink (OSC 8) to the MR: the loop prints it again over the drawn cells after every frame, only where the cells still spell it, and never over a modal.
`i` opens the MR's cover, on demand only: opening an MR still lands on its diff.
It shows the author, branches, age and labels, the description as light markdown, then CHECKS (pipeline status, failed job names once the pipeline pane fetched them), REVIEW (approvals n of m, reviewers and their state, mine) and THREADS: each open thread on two lines, `author · first words` on the full width, then a faded `file.rs:57 · 2 replies`; the selected thread's full `path:line` shows in the bottom border, its start cut when it does not fit. The file tree (`t`) lists the files, so the cover does not.
`j k` walk the threads, `enter` goes there in the diff (a thread also opens in the pane), `p` opens the pipeline pane, `^d ^u` scroll, `o` opens the MR, `esc` `i` `q` close.
`k` on the first thread lets go of it and scrolls on to the top; `g` goes to the top and stays there with nothing selected; `j` picks the first thread again.
From the queue the cover knows the row only: threads wait for the MR, and `enter` opens it.
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

### Header (2 rows, `zh` folds it to 1)

Row 1: author, `source → target`, age, `+adds −dels`, file count, pipeline glyph and word.
Row 2: approvals `n of m`, thread counts, drafts count. Anything at zero is omitted.
Then, once a file is viewed, `viewed 3/8 · 3 folded ━━━───`: the bar turns `success` when every counted file is viewed.
Files that open folded (a `[review] fold` glob, too large, binary) count only once marked viewed; the others show apart as `· n folded`, `muted`.
A file folded by hand still counts; the tree title and the queue row (`· 3/8`) count the same way.
When folded: `nina · +412 −38 · ✓ · 2 drafts`.

### Files

A file row is `▾`/`▸`, the path with the directory in `muted` and the basename in `fg` bold, right aligned `+adds −dels`, then anchors: `◆n` published threads, `◇n` drafts, and a state word: `viewed`, `folded`, `binary`, `too large`, `renamed from x`, `deleted`.
Renames show `old → new`; a pure mode change shows `mode 644 → 755`.

### Hunks

A hunk row is `▾ @@ -a,b +c,d @@ context`, `muted`, with the function context in `fg`; folded it appends `(n lines)`.
`+` on a hunk (its header or any line in it) shows 10 more unchanged lines above and below it, read once from the whole file at the head commit (`Forge::file`: GitLab `repository/files/:path/raw`, GitHub `contents/:path` as raw). Lines between two hunks are never shown twice; old-side numbers follow the offset at each end of the hunk. Comments go on diff lines, not on these extra lines.

Sticky headers: once a file's own row scrolls above the view, a copy stays pinned at the top of the diff, on the surface colour, until the next file's row reaches the top.
When the cursor's hunk header has also scrolled off, it is pinned below the file.
The cursor never hides under the pins, and `za` or `zc` on a line then folds the pinned file and lands on its row (`src/tui/app/pins.rs`).
Pins are off below 20 diff rows; in zen (`zz`) they sit inside the centred column.

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
- Syntax colours sit on top (TypeScript, TSX, JavaScript, Python, JSON, SQL and Markdown today, `src/syntax/`): tree-sitter highlights each hunk's old side and new side as one text, so a string or comment spanning lines keeps its colour, and each line takes its side's spans. Syntax sets the text colour, the diff keeps the line fill and the word fill; on an unknown ground the sign alone says `+` or `-`. Each palette carries its upstream code colours (keyword, string, comment, number, type, function, constant, punctuation). Files too large to show, binary files and unknown extensions stay plain.
- Markdown is the block grammar with its inline grammar injected into each block and table cell, so headings, code, emphasis and links colour. A markdown file's table line (trimmed, it starts with `|`) draws each unescaped pipe as `│`, and a delimiter row its dashes as `─` and inner pipes as `┼`, in `faded` (the same lines split a note's table columns). Only the drawing changes: each glyph takes the one cell of the character it covers, and comments, suggestions, yank, search and word diff all see the raw text.
- Every palette carries these six colours (`added`, `removed`, the two fills, the two word fills), so a theme decides the diff look, not the renderer. A key typed in the first ~50 ms after launch may be read with the answer and lost.
- Context lines are `fg` with no surface.
- Tabs render as `→   `, trailing whitespace as `·` in `warn`, both only on changed lines.
- Long lines are cut with `…`; `w` wraps them with a hanging indent under the text column, the line's fill carried on every row.
- Wrapped, a markdown table line wider than the text column wraps each cell inside its column instead, `│` repeated at the column lines on every extra row and a delimiter row shrunk to the same `─┼─` columns. The columns come from that line's own pipe positions: each is as wide as the text between two pipes less its two padding spaces, and the widest gives up one column at a time (never under 3) until the line fits, so the rows of an aligned table stay aligned without seeing each other. A cell breaks at its last space in reach, else between letters, each character keeping its colours; its alignment is not kept. A line whose columns cannot fit at 3 wraps as any other line.
- `W` hides whitespace-only changes: a removed line and its added twin that differ only in spaces, tabs or line endings read as one context row with a `≈` sign, even in split mode.
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

Built (M3b, `09-thread-pane.md`): no row is inserted for a conversation.
A two-cell anchor column left of the numbers shows `◆` (unresolved, `warn`), `◇` (my draft, `accent`, `danger` while unsaved) or `✓` (resolved, `faded`), then a count when the line holds several.
The row under the header lists the threads on the MR; file rows add `· n outdated`.
While the pane is on a range comment, the range's other lines show `│` in `accent`.

### Visual select

`V` starts a range from the current line; `j/k` extend it; the range fill uses `highlight` or, without one, inverts the gutters.
`c` comments on the range, `y` yanks it as text, `esc` drops it.

## The right pane

One of:

- **Conversations** (M3b, `09-thread-pane.md`): every thread and draft of one line (or of the MR, or a file's outdated threads), unresolved first, resolved folded; notes as `author · age` then the body as light markdown, a suggestion drawn as a small `-`/`+` diff, a table as aligned columns split by `│` under a `─┼─` rule, its widest columns cut with `…` when the pane is narrower and left as raw text when even that does not fit. The compose box sits at its bottom. It follows the cursor onto marked lines. Width: three columns from 150, the queue steps aside from 120, a page of its own below.
- **Overview** (`o` on the header, or on open when there is no thread): description as markdown, labels, reviewers with their state, approvals, pipeline link, then the activity list (system notes) in `muted`.
- **Pipeline** (`p`): the CI run of the head commit (GitLab's newest MR pipeline, GitHub's check runs grouped by workflow): a count per state, then each stage in the order it ran with its jobs, counted failures first, glyph, name and duration; the cursor starts on the first failure; `o` opens the job, `y` copies its link, `r` asks again, and a run still going is asked again every 15 s while the pane shows it. A failure the forge lets pass shows `!` in the warning colour. The header's pipeline word links to the run.
- **Files** (`t`): a tree with folders before files, folders deeper than two levels folded, `+adds −dels`, `◆n` unresolved threads and `✓` viewed on each file; `enter` on a folder folds it, on a file jumps the diff there (the tree stays open), `t` or `esc` closes it. The title counts viewed files.

`zv` (in the diff or the tree) marks the file viewed and folds it. Viewed files are saved per MR with a fingerprint of their change: a file the author pushes to again comes back unviewed.
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
│  enter publish · e edit · d delete · esc │
└──────────────────────────────────────────┘
```

`a` toggles approve; `j/k` then `e` edits a draft; `d` deletes one.
`enter` publishes from anywhere in the modal; `e` edits the selected draft.
Before sending, every draft is checked against the current diff: a draft whose line left it is marked `✗`, the cursor lands on it, and `m` turns it into a note on the MR (the forge would refuse it otherwise).
While publishing, the modal shows the spinner and disables keys; a failure keeps the drafts and toasts the reason.

## Keys

Marked `M1` `M2` `M3` `M4` by milestone. Everything is in `?` `?`.

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
| `i` | the MR cover: description, checks, reviews, threads, files | M3 |
| `:` | command line | M3 |
| `ctrl-k` `⌘k` | jump to an MR or a file | M3 |
| `zz` | zen: the diff alone, quiet (`z` is the fold prefix) | M3 |
| `[m` `]m` | the previous, next MR in the queue's order, in zen or not | M3 |
| `?` | help: the focused pane's keys; again, every key | M1 |
| `q` | close the right pane when one is open and not from the queue, else quit (`ctrl-c` always quits) | M1 |

### Review

| Key | Action | |
|---|---|---|
| `Tab` `S-Tab` | next, previous file | M1 |
| `]c` `[c` | next, previous change (hunk) | M1 |
| `]n` `[n` | next, previous thread or draft, in every file: a folded file or hunk on the way opens and stays open | M1 |
| `]f` `[f` | next, previous file with unresolved threads | M1 |
| `za` | toggle fold under the cursor | M1 |
| `zc` `zo` | close, open | M1 |
| `zM` `zR` | fold, unfold every file | M1 |
| `zv` | mark viewed (folds) | M3 |
| `t` | file tree | M3 |
| `p` | pipeline: jobs by stage, failures first, `o` opens a job | M5 |
| `w` | wrap long lines | M3 |
| `W` | hide whitespace-only changes | M3 |
| `+` | more context around the hunk | M3 |
| `c` | comment on the line (draft) | M2 |
| `V` | select lines | M2 |
| `v` | the file after the change in the reader's program, at the cursor's line (`08-open-file.md`) | M3b |
| `E` | comment in `$EDITOR` | M2 |
| `s` | suggestion: editor prefilled with the lines | M2 |
| `d` | delete the draft under the cursor | M2 |
| `P` | publish modal | M2 |
| `A` | approve, unapprove | M2 |
| `a` | ask: `e` explain, `r` risks, `s` summary, `t` thread, `c` comment, `a` free | M4 |

### Thread pane (M3b)

| Key | Action |
|---|---|
| `j` `k`, `J` `K` | note by note, thread by thread |
| `enter` | unfold, fold a resolved thread |
| `r` | reply, in the compose box |
| `R` | resolve, unresolve (also on a marked line in the diff) |
| `S` | commit the note's suggestion on the MR branch, after a `y` (M5): GitLab applies it by its id; on GitHub, which has no API for it, revu commits the change itself through the contents API, only on the PR's own branch and with push access, else `o` opens it on the web |
| `e` `d` `E` | edit, delete my draft; edit it in `$EDITOR` |
| `u` `o` `y` `v` | first link; the thread in the browser; copy its link; the file in your program |
| `x` `esc` `q` | close |

In the compose box: `enter` saves the draft, `⌘enter` (or `ctrl-s`, where the terminal keeps `⌘enter`) posts a new thread or a reply at once with no draft, `⌥enter` adds a line, `⌥←`/`⌥→` jump a word (also `esc b`/`esc f`), `⌥⌫` deletes one, `ctrl-o` moves the text to `$EDITOR`, `esc` leaves it with the text kept.

### Your own keys (`[keys]`)

`layout = "azerty"` makes `(` and `)` stand for `[` and `]`, which a Mac French keyboard types with `⌥⇧`; the brackets keep working.
`[keys.bind]` maps an action name to one key, two keys, a named key (`ctrl-e`, `tab`) or a list of them.
A user key never replaces a default: it stands for the default key of its action, so it does in each pane what that key does there.
Keys are translated after text boxes, modals and the help had their turn, so typing `(` in a comment is still `(`.
A two-key binding holds its first key; a second key that matches nothing hands both over as they are, so `z` then `a` stays `za`.
The config fails to load, naming the file, on an unknown action, a key that does not parse, a key revu already reads (named with its action), and two actions on one key or on keys where one starts the other.
`?` shows the keys in effect: with the preset, `]n [n` reads `)n (n`, and a bound key comes first (`N )n (n`).

### Search: `ctrl-k`, `⌘k`, `:`

One popup finds MRs, files and commands, picked by the first character as VS Code does.
Nothing typed searches the queue's MRs; `/` first searches the open MR's files; `>` first runs commands, and `⌫` on an empty line steps back to MRs.
`ctrl-k` and `⌘k` open it on MRs; `:` and `⌘⇧k` (where the terminal tells it apart) open it on commands.
The prompt shows the mode (`›`, `/`, `>`) and the title says what it searches.

MR terms (`src/query.rs`, shared with the queue filter and saved views): free words, `"quoted words"`, `@author` (`@me`; several are any of them), `!42` or `#42` (several are any of them), `~label` (`~"two words"`; several are all of them).
The terms keep MRs, then the free words rank them fuzzily; `@loic ~infra slack` finds Loïc's infra MRs about slack.

### Queue filter and saved views

`/` in the queue filters in place with the same terms, plus `draft:yes|no`, `size:small|large` (up to 100 changed lines, from 500), `is:failing` (pipeline failed or conflicts) and `is:mine`; free words must appear in the title, author, project or branch.
A term still being typed (`draft:`, a lone `@`) is left out until it makes sense.
The active filter shows in the queue title; `esc` clears it.
`[queue.views]` saves filters by name; `'` then a view's first letter, or `1`–`9` in name order, applies one, and the title names the view.
While `'` waits, the status line lists the views. Views are checked when the config loads: a term that does not parse, or two views starting with the same letter, fail loudly with the file and the view named.

### Command line (`>` in the search)

Built (M3): `:go !42` (or `#42`, `42`, `acme/widgets!42`), `:open`, `:approve`, `:publish`, `:all`, `:set theme=nord` (saved to the config), `:view`, `:view old`, `:view <path>[:<line>]` (M3b, `08-open-file.md`), `:help`, `:quit`.
Tab cycles the completions for the token under the cursor (verbs, the queue's MRs, themes), `→` accepts the grey ghost, `↑` `↓` walk the history.
Planned with their features: `:reply <text>`, `:draft <text>`, `:resolve`, `:viewed`, `:ai off`, `:ai on`, `:ask <text>`, `:cache clear`.


## Notifications (M5)

While the TUI runs, an MR that lands in To review raises one macOS notification (`osascript`, text passed as arguments, never inside the script).
The first queue answer of a scope only records what To review holds; later fresh answers announce the newcomers, each MR once, all of one answer in a single notification.
Whether the terminal has focus cannot be told reliably, so there is no quiet mode beyond `[notify] enabled = false`.

## Help overlay (`?`)

Every key, grouped by task in the order a review goes: MOVE, QUEUE, VIEW, COMMENT & PUBLISH, THREAD PANE, ASK CLAUDE, SEARCH & APP.
The first `?` shows only the groups of the focused pane; `?` again shows every group; a third `?`, like any key that does not scroll, closes it.
The queue gets MOVE, QUEUE, SEARCH & APP; the diff MOVE, VIEW, COMMENT & PUBLISH, ASK CLAUDE, SEARCH & APP; the right pane COMMENT & PUBLISH, THREAD PANE, ASK CLAUDE, SEARCH & APP.
While filtered, the bottom border ends on `? every key`.
Group titles are faded uppercase; keys are right-aligned in the accent, bold; what they do is plain text, a few words each.
Two columns when the overlay is at least 100 columns wide, split between groups so both columns end level; one column below.
The frame pads two columns on the sides and one row above and below; a blank row separates groups and six columns separate the two columns.
It takes at most 90 % of the screen each way, centred, and scrolls with `j k`, `^d ^u`, `g G` when taller; the bottom border says where you are.
Keys show as they are in effect: user bindings first, `(`/`)` for `[`/`]` with the AZERTY preset.
The groups live in `src/tui/help.rs`; a test fails when a bindable action has no line there.

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
- Kitty keyboard protocol is pushed when available (as slack-tui) so `S-Tab`, `ctrl-k`, `⌘k` and `⌘enter` arrive; `DISAMBIGUATE_ESCAPE_CODES` alone carries `⌘`.
- Mouse: wheel scrolls the pane under the pointer, click focuses and selects, nothing else. Keyboard remains complete.
- Minimum size 80×24; below that the queue hides and a one-line notice says so.

## DX for the person running it

- `revu` with no subcommand opens the TUI.
- `revu list` and `revu show <ref>` print aligned tables, `--json` for scripts; `revu diff <ref>` prints the coloured diff to a pager (`$PAGER`, default `less -R`).
- `revu comment <ref> <path>:<line> <text>` and `revu approve <ref>` for scripts and other agents.
- `<ref>` accepts `group/project!42`, `!42` (current repo from `git remote`), an MR URL, or nothing (current branch).
- Error lines are `✗ message` plus dimmed causes, same as slack-tui.
- `revu --version` prints the crate version and the commit; `revu update` rebuilds from the repo.
