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
| Is checked out somewhere, the main checkout included | That folder |
| Is in your checkout, checked out nowhere | A new worktree, `.claude/worktrees/feat+checkout` for `feat/checkout` |
| Is only on origin | Fetched, then the same new worktree |
| Is on a fork | No session: the toast says the branch is not on origin |

revu never makes a new branch and never moves one you have.

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

## In tmux

The session opens in a tmux window named after it, and revu keeps running in its own window.
`a g` again on the same MR goes to that window instead of opening a second one.
Leave the session with `ctrl-z`: its window closes and the session keeps running.

## Outside tmux

The session takes the terminal, as your editor does for `^v`.
`ctrl-z` brings revu back; the session keeps running.

## From a phone

Run revu inside tmux on your computer and reach it from the phone, as in [Review from a phone](phone.md).
`a g` opens the session in the next tmux window, and the tmux key for the last window brings revu back.
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
Both run without a shell, in a tmux window or on the terminal.
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
- A branch you already have is used as it is, even behind origin: ask the session to pull.
- The tmux window is found by its name: rename the session and `a g` opens a second window.
- revu does not send the line or the thread under the cursor to the session, and shows no session state in the queue.
- revu remembers one session per MR, in its cache, for as long as it keeps the MR there.
- revu never removes a worktree it added: `git worktree remove` does.

## See also

- [Config reference: `[agent]`](../reference/config.md).
- [Keys](../reference/keys.md).
- [Ask Claude](ai.md), for a short answer inside revu.
