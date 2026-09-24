# Zen

`zz` shows the diff alone, with nothing around it.

## What changes

| Around the diff | In zen |
|---|---|
| The queue and the pane frames | Hidden |
| The status line | Hidden; a toast shows for two seconds at the bottom |
| The MR header | One faded line on top |
| The diff | A centred column, 100 columns wide |
| Notifications | Held until you leave zen |

## Move between MRs

`←` and `→` open the previous and next MR, without leaving zen.
They follow the queue as it shows: your filter, the sort and the sections.
MRs in folded sections are skipped.
A line on top says which MR you are on, for example `3/12`.

## Leave

`zz`, `esc` or `h` bring the queue back.
The thread pane still opens with `enter` on a marked line.

## Change the width

```toml
[tui]
zen_width = 110
```

## See also

- [Review and publish](review-and-publish.md).
- [Keys and AZERTY](keys-and-azerty.md), to bind `prev_mr` and `next_mr`.
