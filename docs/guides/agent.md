# Open an agent session on an MR

`a g` opens the coding-agent session that belongs to the MR under the cursor.
The agent is Claude Code, the real one: your login, your skills, your permissions.
On someone else's MR you ask it about the diff; on your own you ask it for a change.

It is off until you switch it on.
It is not [Ask Claude](ai.md), which answers inside revu with an API key: this is a session of its own.

## Switch it on

```toml
[agent]
enabled = true
```

You need the `claude` command, logged in, and a checkout of the project that Claude Code trusts: run `claude` in it once.

## Use it

1. Put the cursor on an MR in the queue, or open one.
2. Press `a`, then `g`, or type `:session`.
3. Talk to the session.

It works in the queue, in the diff, in zen and on a phone.
While Claude's answer or the outline has the keys, `a` is theirs: type `:session` there.

## Which session opens

revu looks in this order, and the toast says which case it took.

| Case | When | The toast says |
|---|---|---|
| Remembered | revu opened a session for this MR before, and Claude Code still knows it | `the session revu remembered for !42` |
| The one that opened the MR | A background session created this MR | `the session that opened !42` |
| Your pick | Claude Code has sessions started in this checkout | `the session picked for !42` |
| New | None of the above | `a new session for !42` |

The pick is a list: a new session first, then the running sessions, then the others, newest first.
Type to narrow it, `enter` opens the row, `esc` leaves.

A running session opens as it is.
A stopped one starts again first, with what it knew.

## A new session

It starts in the background, in a folder on the MR's own branch.

| The branch | The folder |
|---|---|
| Is checked out somewhere, the main checkout included | That folder, as it is |
| Is in your checkout, checked out nowhere | A new worktree on it, as it is: `.claude/worktrees/feat+checkout` for `feat/checkout` |
| Is only on origin | The same new worktree, started from origin's branch |
| Is on a fork | No session: the toast says the branch is not on origin |

revu fetches origin's branch first, a rebased and force-pushed one included.
It never makes a new branch and never moves one you have.
When a branch you have is behind origin, or has gone its own way, the toast says so.
When the fetch fails for another reason, the toast gives git's own words.

| The MR | The session starts |
|---|---|
| Someone else's | In plan mode: it reads and proposes, and changes nothing until you say so |
| Yours | In your default mode |

Its name is `revu: !42` and the MR title, which is how it reads in `claude agents`.
Its first message names the MR, its link, its author and its branches.
It also says that `revu show`, `revu diff` and `revu comment` read the MR and comment on it.
It ends by asking the session to wait for you.

## Where the checkout is

revu uses the checkout it was started in when the MR is of that project.
For any other project, say where it is:

```toml
[agent.checkouts]
"acme/widgets" = "~/code/widgets"
```

Without it, the toast names this setting and nothing starts.

## Where the session opens

Beside revu when something can split the screen, so the diff stays in view.
revu takes the first of these that applies:

| revu runs in | The session opens |
|---|---|
| tmux | In a tmux pane beside revu, or a tmux window |
| Ghostty, iTerm2 or WezTerm | In that terminal's own split, on the right |
| Another terminal, with `[agent] split` set | Where your command puts it |
| Anything else | In revu's place: the session takes the terminal |

Inside tmux, tmux always wins, whatever terminal is around it.
The width rule is the same everywhere: a split needs 140 columns, so that revu keeps 70.
Under that, or with `show = "window"`, nothing splits.

## In tmux

The session opens beside revu, in a pane on the right, so the diff stays in view.
revu keeps the left half, and the keys go to the session.

| revu's pane is | The session opens |
|---|---|
| Wide enough for both: 140 columns or more | In a pane beside revu |
| Narrower, or in the compact layout | In a tmux window named after it |

"Wide enough" means half of revu's width is still `[tui] compact_width` (70) or more.
After the split revu usually draws its narrow layout: the diff alone, then the queue when you go back to it.

| To | Press |
|---|---|
| Go back to revu | The tmux prefix, then `;` |
| Go to the session again | `a g` on the same MR, or the tmux prefix, then `;` |
| Close the session's pane | `ctrl-z` in the session: it keeps running, and revu takes the full width back |
| Come back from a window | The tmux prefix, then `l` |

`a g` on an MR whose session is already on show goes to its pane or its window, and opens no second one.

To always get one or the other:

```toml
[agent]
show = "split"    # or "window"; "auto" is the default
```

## In Ghostty

Ghostty 1.3.0 or later splits to the right of revu and gives the keys to the session.
The first time, macOS asks whether your terminal may control Ghostty: allow it.
Say no, and the session takes the terminal instead, with the reason in the toast.

| To | Press |
|---|---|
| Go back to revu | `cmd-[` |
| Go to the session again | `cmd-]`, or `a g` on the same MR |
| Close the session's split | `ctrl-z` in the session: it keeps running, the split closes, and revu takes the full width back |

revu splits the terminal that is in front when you press `a g`, which is revu's own.
Ghostty needs `macos-applescript` left on, as it is by default.
The split starts your own shell, which becomes the session, so it closes when the session ends.
For the moment your shell takes to start, the split shows it and one short line it types: ` printf …; exec /full/path/claude attach <id>`.
That line wipes the screen and the scrollback right before the session starts, so nothing is left when you scroll up.
The leading space keeps that line out of zsh's history only with `setopt HIST_IGNORE_SPACE`; bash needs `HISTCONTROL=ignorespace`.
The session gets the `PATH` your shell sets, not revu's.

## In iTerm2

iTerm2 splits the session revu runs in, to the right, with the same profile.
The first time, macOS asks whether iTerm2 may be controlled: allow it.

| To | Press |
|---|---|
| Go back to revu | `cmd-[` |
| Go to the session again | `cmd-]`, or `a g` on the same MR |
| Close the session's split | `ctrl-z` in the session |

## In WezTerm

WezTerm splits revu's pane to the right with its own `wezterm cli`.
No permission is asked.

| To | Press |
|---|---|
| Go back to revu | `ctrl-shift-←` |
| Go to the session again | `ctrl-shift-→`, or `a g` on the same MR |
| Close the session's split | `ctrl-z` in the session |

In these three terminals, `a g` on an MR whose session is already in a split goes to that split.
revu keeps the split's id with the session, and opens a new one when the terminal no longer has it.

## In another terminal

Give revu the command that opens a pane beside it.

```toml
[agent]
split = "kitty @ launch --location=vsplit --cwd {dir} {command}"
```

| Placeholder | Becomes |
|---|---|
| `{command}` | The session's program, word by word: write it as a word of its own |
| `{dir}` | The folder the session works in |

The command runs without a shell.
revu cannot see the pane it opened: `a g` again runs the command again.
When the command fails, the session takes the terminal and the toast says why.

Two recipes, not tested:

| Tool | `split` |
|---|---|
| kitty, with `allow_remote_control` on | `kitty @ launch --location=vsplit --cwd {dir} {command}` |
| Zellij | `zellij run --direction right --cwd {dir} -- {command}` |

## Anywhere else

The session takes the terminal, as your editor does for `^v`.
`ctrl-z` brings revu back; the session keeps running.
Run revu in tmux or in one of the terminals above to see the diff and the session together.

## From a phone

Run revu inside tmux on your computer and reach it from the phone, as in [Review from a phone](phone.md).
A phone is too narrow for two panes, so `a g` opens the session in the next tmux window.
The tmux prefix, then `l`, brings revu back.
With the tap bar, `more` then `session` does the same.

## Another agent

Give revu the commands and it runs them in place of Claude Code.

```toml
[agent]
enabled = true
start = "aider --message 'Review {ref}: {title}'"
open = "aider --restore"
```

`start` runs in the folder on the MR's branch, the first time.
`open` runs there the next times; without it, `start` runs again.
Both run without a shell, in a tmux pane or window, or on the terminal.
revu cannot ask another agent which sessions it has, so there is no list and no search.
The session is the remembered one, or a new one.

| Placeholder | Becomes |
|---|---|
| `{ref}` | `!42` on GitLab, `#42` on GitHub |
| `{iid}` | `42` |
| `{title}` | The MR title |
| `{url}` | The MR link |
| `{author}` | The author's handle |
| `{project}` | `acme/widgets` |
| `{branch}` | The MR's branch |
| `{target}` | The branch it merges into |
| `{dir}` | The folder the session works in |
| `{name}` | `revu: !42` and the title |

A typo like `{titel}` stops revu at start, with the file and the key named.

## Limits

- "The session that opened the MR" comes from a file Claude Code keeps for itself: if that file changes, revu goes on to the list.
- A session revu did not start may work in another folder than the MR's branch: revu opens it as it is.
- A branch you already have is used as it is, even behind origin: the toast says so, and you ask the session to pull.
- A pane runs the agent by its full path, with the `PATH` revu was started with, and the terminal's own environment for the rest.
- An agent that is not on revu's `PATH` opens no pane: the toast names the program.
- revu finds a session's pane by a mark it sets on it, so only a pane revu opened is found again.
- In Ghostty, switch to another tab right after `a g` and the split may open there: revu cannot ask Ghostty which terminal it runs in.
- revu does not send the line or the thread under the cursor to the session, and shows no session state in the queue.
- revu remembers one session per MR, in its cache, for as long as it keeps the MR there.
- revu never removes a worktree it added: `git worktree remove` does.

## See also

- [Config reference: `[agent]`](../reference/config.md).
- [Keys](../reference/keys.md).
- [Ask Claude](ai.md), for a short answer inside revu.
