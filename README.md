# gitlabmr

GitLab merge request review in your terminal, as yourself.
One Rust binary called `mr`: a TUI for reading diffs, commenting, and publishing reviews, plus scriptable commands.

> Status: bootstrap (M0). The design is in `specs/`, the roadmap in `specs/06-roadmap.md`.

## Install

```sh
cargo install --path .
```

Needs macOS (keychain) and Rust 1.88+.

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
mr list                        # the MRs waiting on you, yours, the ones you watch (`--cached` skips the network)
mr show acme/widgets!42        # header, files, unresolved threads
mr diff !42                    # the coloured diff through $PAGER (`!42` takes the project from the origin remote)
mr show                        # the open MR of the current branch
```

Every command takes `--json`. An MR is `group/project!42`, `!42`, an MR URL, or nothing for the current branch.

## TUI

```sh
mr
```

| Key | Action |
|---|---|
| `j` `k` / arrows, `g` `G`, `ctrl-d` `ctrl-u` | Move |
| `h` `l` | Focus the pane to the left, right |
| `enter` | Open the MR, open the thread, or toggle the fold under the cursor |
| `esc` | Back: close the thread, then the queue |
| `/` | Filter the queue by title, author or iid |
| `r` | Refresh |
| `o` / `y` | Open in the browser / copy the URL (the line, inside a diff) |
| `tab` `S-tab` | Next, previous file |
| `]c` `[c` | Next, previous hunk |
| `]n` `[n` | Next, previous thread |
| `]f` `[f` | Next, previous file with an unresolved thread |
| `za` `zc` `zo` | Toggle, close, open the fold under the cursor |
| `zM` `zR` | Fold, unfold every file |
| `zo` (queue) | Show the done section |
| `u` (thread) | Open the first link of the thread |
| `?` | Every key |
| `q` | Quit |

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
