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

## Hints

| Hint | When | Try |
|---|---|---|
| Long walk | 15 lines or more with `j` or `k` in a diff with hunks or threads | `]n` or `]c` |
| Queue after zen | You went back to the queue to change MR after using zen | `[m` and `]m` |
| Number in search | You typed a bare number in the search | `!42`, or `#42` on GitHub |

## See also

- [Config reference](../reference/config.md).
- [Keys and AZERTY](keys-and-azerty.md).
