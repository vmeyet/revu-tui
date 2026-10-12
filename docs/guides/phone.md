# Review from a phone

revu has a compact layout for a small screen.
It fits a phone held upright, about 50 columns wide, reaching revu on your Mac through tmux.

## What changes

| Around you | In the compact layout |
|---|---|
| The panes | One page at a time, full screen: the queue, then the diff, then the right pane |
| A queue row | Two lines: the title, then who, which MR, its size, its age and its marks |
| Long lines in the diff | They wrap; `w` cuts them again |
| Line numbers | Only the sign shows; the title of the diff tells the cursor's line, as `L12` |
| A long file path | Cut in its middle, so the folder and the file name both read |
| The threads of a line | In the diff, under their line, where the comment box opens too |

`enter` or `l` on a queue row opens the diff as the page.
`h` goes back one page: from the right pane to the diff, from the diff to the queue.
Every key works as it does on a wide screen.

## When it turns on

revu picks the layout again each time it draws.
Turn the phone, or go back to your desk, and the layout follows with no restart.

| Setting | Default | Does |
|---|---|---|
| `layout` | `auto` | `auto` follows the width, `compact` forces the compact layout, `full` never shows it |
| `compact_width` | 70 | Below this many columns the window is compact |

```toml
[tui]
layout = "auto"
compact_width = 70
```

`compact_width` cannot be above `narrow_width`.

## A short screen

The keyboard of a phone leaves 12 to 20 rows.
Under 20 rows the status row and the MR header hide, and the page takes their rows.
A question, the search and the queue filter still show on their row.
A toast reads on the bottom row for two seconds.

## Set up tmux

Run revu inside tmux on the Mac, and attach from the phone.

```sh
# ~/.tmux.conf on the Mac
set -g mouse on              # taps and swipes reach revu
set -g set-clipboard on      # a copy sent to the terminal reaches the phone
set -g window-size latest    # the window takes the size of the screen used last
```

`mouse on` lets a tap pick a row and a swipe scroll the page.
`set-clipboard on` hands what tmux copies to the phone's terminal.
revu itself copies on the Mac, where it runs: `y` and a drag fill the Mac's clipboard.
With `window-size latest` the phone resizes the window only while you use it.
Back at the desk, the first key gives the window its size again.

A grouped session gives the phone its own view of the same windows.

```sh
# from the phone, over mosh or ssh
mosh mac -- tmux new-session -A -t main -s phone
```

It shares the windows of `main`, so revu is there.
The phone resizes only the window it looks at, and the others keep the size of the desk.

The font size is a setting of the phone's terminal app, not of revu.

## See also

- [Config reference](../reference/config.md).
- [Zen](zen.md), for the diff alone on a wide screen.
