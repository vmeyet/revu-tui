# gitlabmr

GitLab merge request review in your terminal, as yourself.
One Rust binary called `mr`: a TUI for reading diffs, commenting, and publishing reviews, plus scriptable commands.

> Status: bootstrap (M0). The design is in `specs/`, the roadmap in `specs/06-roadmap.md`.

## Install

```sh
cargo install --path .
```

Needs macOS (keychain) and Rust 1.88+.
Update with `mr update`: it pulls this checkout (fast-forward only), and rebuilds when the commit moved; `-f` rebuilds anyway.
`mr --version` prints the version and the commit it was built from.

## Log in

```sh
mr login                 # prompts for a personal access token with the `api` scope, stores it in the keychain
mr login --from-glab     # reuses the token glab already holds
mr whoami
mr logout
```

`GITLAB_TOKEN` and `GITLAB_HOST` override the keychain and the config, for scripts.

## Commands

```sh
mr list                        # the MRs waiting on you, yours, the ones you watch, and the rest of the repo's (`--cached` skips the network)
mr show acme/widgets!42        # header, files, unresolved threads
mr diff !42                    # the coloured diff through $PAGER (`!42` takes the project from the origin remote)
mr show                        # the open MR of the current branch
mr comment !42 --at src/a.rs:13 looks racy   # a public comment, on a line with --at
mr approve !42                 # or --undo
mr publish !42                 # every draft you hold on the MR, as one review
```

Inside a GitLab checkout, `mr` and `mr list` show that project only, plus an `OPEN` section with its other open MRs; `--all` shows every project.
Every command takes `--json`. An MR is `group/project!42`, `!42`, an MR URL, or nothing for the current branch.

## TUI

```sh
mr
```

| Key | Action |
|---|---|
| `j` `k` / arrows, `g` `G`, `ctrl-d` `ctrl-u` | Move |
| `h` `l` | Pane to the left; open the selected MR, pane to the right |
| `enter` | Open the MR, open the thread, or toggle the fold under the cursor |
| `esc` | Back: close the thread, then the queue |
| `/` | Filter the queue by title, author or iid |
| `*` | Queue: this repo only, or every project |
| `i` | The MR description, in a modal |
| click `!42` | Open the MR, in terminals that follow links (Ghostty, iTerm2, Kitty, WezTerm) |
| `r` | Refresh |
| `o` / `y` | Open in the browser / copy the URL (the line, inside a diff) |
| `tab` `S-tab` | Next, previous file |
| `]c` `[c` | Next, previous hunk |
| `]n` `[n` | Next, previous thread |
| `]f` `[f` | Next, previous file with an unresolved thread |
| `za` `zc` `zo` | Toggle, close, open the fold under the cursor |
| `zM` `zR` | Fold, unfold every file |
| `zo` (queue) | Show the done section |
| `c` | Comment on the line, as a draft |
| `V` | Select lines: `c` comments on them, `y` copies them, `esc` drops them |
| `E` / `s` | Write the comment in `$EDITOR` / as a suggestion prefilled with the lines |
| `enter` / `d` (draft) | Edit / delete the draft under the cursor |
| `P` | Publish every draft in one review (`a` in the modal also approves) |
| `A` | Approve, unapprove |
| `r` / `R` (thread) | Reply as a draft / resolve, unresolve |
| `u` (thread) | Open the first link of the thread |
| `?` | Every key |
| `q` | Quit |

Comments are GitLab draft notes until `P`: they survive a restart, show up in the web UI as pending, and nothing is public before you publish.

Read-only for now (M1): the queue, the diff with folds, the threads. Comments and approvals come with M2.
The queue refreshes every minute and the open MR every 30 s; an MR that moved since you last opened it shows `●`.

The full key grammar planned for the review is in `specs/03-ui-ux.md`.

## Config

`~/.config/gitlabmr/config.toml`:

```toml
host = "gitlab.com"

[queue]
watch_labels = ["infra"]   # MRs with these labels land in Watching

[review]
fold = ["*.lock", "*.snap"]   # files that open folded

[tui]
theme = "tokyonight"   # default, dracula, catppuccin, catppuccin-latte, rosepine, rosepine-dawn, nord, tokyonight, monokai
```
