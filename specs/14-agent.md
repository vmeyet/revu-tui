# 14 · Agent session

Goal: from an MR in revu, one key lands in the coding-agent session that belongs to that MR.
The reader runs Claude Code on a subscription, through the `claude` command, often as background sessions inside tmux and sometimes from a phone.
On someone else's MR the session answers questions about the diff with the reader's own skills; on their own it takes a change or a rebase.

This is step 1: find or start the session, and show it.
It sits beside "ask Claude" (`05-ai.md`) and changes nothing there.

## The setting

`[agent]`, off by default.

| Key | Means |
|---|---|
| `enabled` | `a g` does something. Off, the key says how to turn it on and nothing runs. |
| `start` | Another agent than Claude Code: the command that starts its session. |
| `open` | With `start`: the command that opens that session again. |
| `checkouts` | Project path → its folder on this machine, a full path or one from `~/`. |
| `show` | In tmux: `auto` (default), `split` or `window`. |

Checked when the config loads, with the key named: both commands split into words and name only known placeholders, `open` needs `start`, a checkout is a full path.

## The action

`session`, default keys `a` then `g`, and `:session`.

- On the MR under the cursor: the queue's row while the queue has the keys, else the open MR. So it works in the queue, the diff, zen and the compact layout.
- `a` is the "ask" family, and an agent is who you ask: `g` is free under it, and a key of its own would cost one of the last free letters.
- `a` now opens its prefix in the queue too, where only `g` answers.
- While Claude's answer or the outline has the keys, `a` is theirs: `:session` is the way in.
- Bindable under `[keys.bind] session`, listed in `?` under search & app, which every pane shows.
- No label on the tap bar: `more` lists it.

## Finding the session

In this order; the toast says which case it took.

1. **Remembered**: the session revu kept for this MR, if `claude agents --json --all` still lists it.
2. **The opener**: a background session whose `state.json` lists this MR's link among the links it produced. Of several, the running one, then the newest.
3. **A pick**: sessions started under the project's checkout exist. The palette lists them under a "new session" row, running ones first, then newest first.
4. **New**: otherwise.

`state.json` (`~/.claude/jobs/<id>/state.json`, field `children`, each with an `href`) is Claude Code's own file and documented nowhere.
Reading it is best-effort: a missing folder, file or field, or text that does not parse, means "not found" and never an error.
Links compare without their scheme, a trailing slash, GitLab's `/-/`, a query or letter case.
Observed on 2026-10-13 with Claude Code 2.1.296: it holds GitHub pull request links and GitLab merge request links alike.

Only background sessions count: `claude attach` opens no other kind.
A session with a `status` in the list is running or waiting and opens as it is; one without is started again first.

## Starting one

- **The checkout**: the one revu was started in when its remote is the MR's project, else `[agent.checkouts]`, else a toast naming that setting. No disk search. Started inside a worktree, revu takes the repository's main checkout, so worktrees never nest.
- **The folder**: a worktree on the MR's source branch, never a new branch.
  - `git worktree list --porcelain` shows the branch checked out, the main checkout included: that folder.
  - The checkout holds the branch: `git worktree add <folder> <branch>`, the branch as it is.
  - Else `git fetch origin refs/heads/<branch>:refs/remotes/origin/<branch>`, then `git worktree add -b <branch> <folder> origin/<branch>`.
  - The folder is `<checkout>/.claude/worktrees/<branch>` with `+` for `/`.
  - The fetch fails, as for a fork's branch: a toast says so and nothing starts.
  - A branch name starting with `-` or holding `..` is refused before git runs.
- **The session**: `claude --bg --name <name> [--permission-mode plan] <prompt>`, run in that folder.
  - The name is `revu: <ref> <title>`, cut at 60 characters.
  - Plan mode on someone else's MR, the reader's default on their own.
  - The prompt sets the scene (reference, title, author, link, both branches, and that `revu show`, `revu diff` and `revu comment` exist) and asks to wait: one short turn.
  - The title and the author are the forge's text and reach the model: one more reason for plan mode on someone else's MR.
  - Its id is read back from `claude agents --json`: the session whose id the command printed, else the newest of that name.

## Showing it

- A stopped session first: `claude --bg --resume <session-id>`, in the folder it was started in.
- **In tmux** (`TMUX` set), beside revu when it fits, so the diff stays in view:
  - Already on show: `tmux list-panes -s -F '#{pane_id}<tab>#{window_id}<tab>#{@revu_session}'` lists the panes of revu's tmux session. A pane marked with this session gets `tmux select-window -t <window>` then `tmux select-pane -t <pane>`, and nothing opens.
  - A pane: `tmux split-window -h -P -F '#{pane_id}' claude attach <id>`. revu keeps the left half and the session takes the keys.
  - A window: `tmux new-window -n <name> -P -F '#{pane_id}' claude attach <id>`.
  - The mark: `tmux set-option -p -t <pane> @revu_session <session id>` on the pane just made. A pane option survives a rename of the window or of the session; an agent with no id of its own is marked with the session's name.
  - `fits_beside`: tmux leaves revu half of its pane's width (`tmux display-message -p '#{pane_width}'`), and that half must be `compact_width` or more: 140 columns by default. So a split never pushes revu into the compact layout, and `layout = "compact"` never splits. A width tmux does not give means a window.
  - `[agent] show`: `split` and `window` skip the question; `auto` asks it.
  - The words go to tmux one by one, so no shell reads them.
  - The pane closes when the program exits; tmux resizes revu and the loop draws again on the resize event, as for any window change.
- **Outside tmux**: the loop lends the terminal to `claude attach <id>`, as it does for `^v` (`08-open-file.md`), and takes it back when the program exits.

## What is remembered

MR → session (`name`, `dir`, the agent's `id`) in the cache, `mr/<project>/<number>/session.json`: next to the MR's other state, never in the config.
It goes when the cache drops the MR.
It is written every time a session is shown, so the last one found is the one remembered.

## Another agent

`[agent] start` replaces Claude Code with two commands, filled with `{ref}` `{iid}` `{title}` `{url}` `{author}` `{project}` `{branch}` `{target}` `{dir}` `{name}`.

- Nothing remembered, or no `open`: the worktree as above, then `start` in it.
- Remembered and `open` set: `open`, in the remembered folder.
- No list, no opener, no pick: revu cannot ask such an agent what it runs.
- The command is split into words first and filled after, so a title with a quote stays one word.
- Its first word is a program, never a placeholder: text from the forge is an argument, not a command.

## The seam

`src/agent/`:

| File | Holds |
|---|---|
| `mod.rs` | `Agent::open`, the neutral types, and `Run`: the one trait every `claude`, `tmux` and `git` call goes through |
| `claude.rs` | Everything Claude Code: its list, `state.json`, the find order, start, resume, attach |
| `custom.rs` | The two commands of another agent |
| `worktree.rs` | The folder on the MR's branch |
| `show.rs` | The tmux pane or window, or the hand-off |

A second built-in agent is a second file beside `claude.rs` and one more arm in `Agent::open`.
The app (`src/tui/app/session.rs`) only asks for `Action::Session` and applies what comes back.

## Tests

`Run` is replaced by a script that answers from a list and records every call, so no test starts a session, a tmux window or a git command, or reads the real `~/.claude`.

- The find order: each of the four cases, with a fixture list and fixture `state.json` files, broken and missing ones included.
- The commands, word for word: attach, resume, new; in tmux and outside.
- The pane against the window: by width on both sides of the limit, by each value of `show`, a width tmux does not give; a session already in a pane and one already in a window.
- The worktree: reused, added from the local branch, fetched then added, the fork, a name that reads as a flag.
- Plan mode against the default; the name cut; the id read back.
- The mapping kept across a restart; a remembered session Claude Code forgot.
- The other agent: both commands filled, the folder, no search.
- The config: the default, both commands, `open` alone, a relative checkout.
- The app: the disabled toast, the key in the queue, the diff, zen and compact, the command, a user binding, the picker.

## Later steps, not built

Sending the line or thread under the cursor to the session, answers streamed into the answer pane, draft comments written by the agent, session state on queue rows, and the session replacing the API-key "ask Claude".

## Considered

- **`claude -w` for the worktree**: it makes a new branch, and the session must sit on the MR's own.
- **A key of its own, `B` or `I`**: reads in every pane, but spends a free letter on one action.
- **Finding the pane by its window's name**: a rename opened a second one; a pane option costs one more command at creation.
- **A split on top of the right pane's width only (`narrow_width`)**: half of 240 columns is rare; staying out of compact is the rule that matters.
- **`tmux switch-client` across every tmux session**: it needs an attached client and finds windows the reader moved away on purpose; the current session is enough.
- **Passing `--session-id`**: revu would know the id without reading it back, at the cost of making UUIDs.
- **Sessions listed with `--cwd`**: one call to `--all` already answers the remembered case and the pick.
