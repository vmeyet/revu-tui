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
| What moves | A spinner steps four times a second, and the queue marks stay lit: a screen at rest sends nothing |

`enter` or `l` on a queue row opens the diff as the page.
`h` goes back one page: from the right pane to the diff, from the diff to the queue.
Every key works as it does on a wide screen.

## Tap instead of typing

The bottom row is a tap bar: each label presses a key for you.

| Page | Labels | Keys they press |
|---|---|---|
| Queue | open, filter, refresh, copy, more | `enter` `/` `r` `y` `:` |
| Diff | file, thread, viewed, comment, back, more | `tab` `]n` `zv` `c` `h` `:` |
| A thread, in the diff or in the pane | reply, resolve, react, copy, back, more | `r` `R` `+` `y` `h` `:` |
| Another right pane | back, more | `h` `:` |

`more` opens the commands, where `approve`, `publish` and `copy` are.
`copy` puts the link of the MR or the thread on the phone's clipboard.
On a thread in the diff, `back` goes to the queue, as it does one row above.
A tap on a row picks it, and a second tap opens it.
A swipe scrolls the page under your finger.
The bar hides while revu asks a question or you type, and comes back after.

```toml
[tui]
tap_bar = false
```

`?` lists the keys of the bar for the page you are on, and `?` again lists every key.
`,` opens the search without `ctrl`, on a phone and on a desk.

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
The tap bar stays.
A question, the search and the queue filter still show on their row.
A toast reads on the bottom row for two seconds.

## Set up tmux

Run revu inside tmux on the Mac, and attach from the phone.

```sh
# ~/.tmux.conf on the Mac
set -g mouse on              # taps and swipes reach revu
set -g set-clipboard on      # a copy in revu reaches the phone
set -g window-size latest    # the window takes the size of the screen used last
```

`mouse on` lets a tap pick a row and a swipe scroll the page.
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

## Take a link to the phone

`o` opens the browser on the Mac, where revu runs.
Copy the link instead, and paste it on the phone.

| Key | Copies |
|---|---|
| `y` | The link of the MR, the line, the thread or the job under the cursor |
| `U` on a thread | Its first link, or its first picture when it has no link |
| `:copy` | The link `:open` would show |

Every copy goes to the Mac's clipboard and to the terminal you look at.
tmux hands it on with `set -g set-clipboard on`, or with `set -g allow-passthrough on`.
The phone's terminal app must take copies sent by a program, named OSC 52 in its settings.

## Pictures

A session started from iTerm2 on the Mac can leave blank rows where a picture should be.
Set `[tui] images = false`: each picture reads as `[image: alt]`, and `U` copies its link.

## See also

- [Config reference](../reference/config.md).
- [Zen](zen.md), for the diff alone on a wide screen.
