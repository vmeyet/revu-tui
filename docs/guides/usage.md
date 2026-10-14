# See what you use

revu can count which actions you use and how long you spend on each screen.
The counts show what to trim, what to adopt, and where a faster key exists.

## Switch it on

Counting is off until you ask for it.

```toml
[usage]
enabled = true
```

revu keeps the counts on this machine, in `usage.jsonl` under its cache folder.
It never sends them anywhere.
It records action names only: no MR titles, paths, comments, hosts or projects.
Delete the file to start again.

## Read the report

```sh
revu usage              # the last 30 days
revu usage --since 4w   # or 7d, or all
revu usage --json       # for a script
```

| Section | Shows |
|---|---|
| NEVER USED | Every action you did not use, grouped like the help |
| RARELY USED | Actions used twice or less |
| MOST USED | Your top 10 actions |
| TIME | Time on each screen: queue, diff, pane, cover, zen, palette, help |
| HINTS | Habits a faster key replaces |

A key counts as what it did where you pressed it: `q` that closes a pane counts as `close_pane`, not `quit`.
A key that does nothing where you pressed it is not counted.

## Time

A screen counts time while you use it.
It stops a minute after your last key, click or wheel, and starts again on the next one.
It also stops while the terminal is in the background, where the terminal reports it.

Days counted before revu worked this way also counted idle time and fired hints too often.
The report keeps their actions, leaves them out of TIME and HINTS, and says how many there are.

## Hints

| Hint | When | Try |
|---|---|---|
| Long walk | 15 lines or more with `j` or `k` to reach a hunk or a thread | `]c` or `]n` |
| Queue after zen | You left zen and opened another MR from the queue within 30 seconds | `[m` and `]m` |
| Number in search | You typed a bare number in the search | `!42`, or `#42` on GitHub |

The first three times a hint fires, a toast names the key to try.
After that, only the report counts it.

## See also

- [Config reference](../reference/config.md).
- [Keys and AZERTY](keys-and-azerty.md).
