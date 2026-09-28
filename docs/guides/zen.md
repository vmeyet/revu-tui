# Zen

`zz` shows the diff alone, with nothing around it.

`revu 42` starts there: MR 42 of the checkout's project opens in zen while the queue loads behind it.
It takes every form `revu show` takes, `acme/widgets!42` or an MR URL too; quote `'!42'`, which bash and zsh read as history.

## What changes

| Around the diff | In zen |
|---|---|
| The queue and the pane frames | Hidden |
| The status line | Hidden; a toast shows for two seconds at the bottom |
| The MR header | One faded line on top; `◆3` counts the unresolved threads |
| File and hunk headers | Pinned at the top of the column while you scroll |
| The diff | A centred column, 70 % of the screen and 120 columns at least; the whole screen when `D` shows it side by side |
| The thread pane | Under the diff in the same column, behind a faded rule with its title, no frame |
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

## The thread pane

It opens under the diff, which keeps your line in view.
It takes the rows its threads need, up to a quarter of the column, and grows up to half while you write a comment.
`h` and `l`, or `←` and `→`, move between the diff and the pane.
The mouse wheel scrolls the part under the pointer; a drag copies from one part only.
In the list of every thread, `enter` moves the diff above to that thread's line.

On a terminal under about 25 rows the pane takes the whole column instead.
There, `enter` in the list of every thread shows the diff at that line, and `l`, `→` or `T` bring the list back.

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
