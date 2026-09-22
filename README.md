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

## TUI

```sh
mr
```

| Key | Action |
|---|---|
| `h` `l` | Focus the pane to the left, right |
| `r` | Refresh |
| `?` | Help |
| `q` | Quit |

The full key grammar planned for the review is in `specs/03-ui-ux.md`.

## Config

`~/.config/gitlabmr/config.toml`:

```toml
host = "gitlab.com"

[tui]
theme = "tokyonight"   # default, dracula, catppuccin, catppuccin-latte, rosepine, rosepine-dawn, nord, tokyonight, monokai
```
