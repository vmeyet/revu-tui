# Zen

`zz` shows the diff alone, with nothing around it.

## What changes

| Around the diff | In zen |
|---|---|
| The queue and the pane frames | Hidden |
| The status line | Hidden; a toast shows for two seconds at the bottom |
| The MR header | One faded line on top; `◆3` counts the unresolved threads |
| File and hunk headers | Pinned at the top of the column while you scroll |
| The diff | A centred column, 70 % of the screen and 120 columns at least; the whole screen when `D` shows it side by side |
| The thread pane | The same column, with one faded title line and no frame |
| Notifications | Held until you leave zen |

## Move between MRs

`[m` and `]m` open the previous and next MR, without leaving zen.
They do the same outside zen.
They follow the queue as it shows: your filter, the sort and the sections.
MRs in folded sections are skipped.
A line on top says which MR you are on, for example `3/12`.

## Leave

`zz`, `esc`, `h` or `←` bring the queue back.
The thread pane still opens with `enter` or `→` on a marked line, or with `T` on every thread; `q`, `x` or `esc` close it.
In the list of every thread, `enter` shows the diff at that thread's line; `l`, `→` or `T` bring the list back.

## Change the width

A fixed width stops the column from growing with the screen.
Side by side ignores it and takes the whole screen, so a window of about 120 columns shows both sides.

```toml
[tui]
zen_width = 110
```

## See also

- [Review and publish](review-and-publish.md).
- [Keys and AZERTY](keys-and-azerty.md), to bind `prev_mr` and `next_mr`.
