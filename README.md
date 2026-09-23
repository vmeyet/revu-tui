# revu

Review merge requests in your terminal: GitLab MRs and GitHub PRs, one keyboard, no browser tab.
One Rust binary called `revu`: a TUI to read diffs, comment and publish reviews, plus scriptable commands.

> [!IMPORTANT]
> **macOS only** for now (keychain, `open`, `pbcopy`).
>
> **Vibe coded** for personal use; use it at your own risk.

## Install

```sh
cargo install --git https://github.com/vmeyet/revu-tui
```

That is all: `revu` is now on your path.
You need macOS and Rust 1.88+ (`curl -sSf https://sh.rustup.rs | sh`).

Update with `revu update` (a no-op when you already run the latest commit, `-f` to rebuild anyway).
`revu --version` prints the version and the commit it was built from.

## Log in

```sh
revu login --from-glab            # reuse the token glab already holds (GitLab, scope `api`)
revu login github.com --from-gh   # reuse the token gh already holds (GitHub, scope `repo`)
revu login                        # or paste a token; it goes to the keychain
revu whoami
```

The host picks the forge: `github.com` is GitHub, anything else GitLab.
Inside a checkout, `revu` talks to the host of its `origin` remote.

## Review

```sh
revu
```

Three panes, like a chat client: the queue of MRs, the diff, the thread.
The diff is coloured by syntax for TypeScript, JavaScript, Python and JSON.
Inside a checkout the queue shows that repo only: what waits on you, yours, what you watch, and every other open MR.
`*` widens it to every project.

| Key | Action |
|---|---|
| `j` `k`, `g` `G` | Move |
| `enter` / `l` | Open the MR, the thread, or toggle a fold |
| `tab`, `]c`, `]n` | Next file, hunk, thread |
| `za`, `zM` `zR` | Fold one, fold all, unfold all |
| `zo` `zc` (queue), `zh` | Open or fold the section under the cursor, fold the MR header |
| `D` | One-word changes inline (`2;` struck, `20;` after it), or every line split |
| `i` | The MR description |
| `c` | Comment on the line (`V` first for a range, `s` for a suggestion, `C` for the old side of an inline change) |
| `r` / `R` | Reply / resolve, in a thread |
| `P` | Publish every draft as one review: `enter` sends, `e` edits, `a` also approves |
| `o` / `y` | Open in the browser / copy the link |
| `?` | Every key |

Comments stay drafts until `P`: GitLab draft notes, or your pending review on GitHub.
They survive a restart, and nothing is public before you publish.

## Commands

Every command takes `--json`, for scripts and agents.

```sh
revu list                              # the queue, as a table
revu show acme/widgets!42              # header, files, open threads (owner/repo#42 on GitHub)
revu diff !42                          # the coloured diff in $PAGER; !42 takes the repo from origin
revu comment !42 --at src/a.rs:13 "looks racy"
revu approve !42
revu publish !42                       # every draft you hold, as one review
```

## Settings

`~/Library/Application Support/revu/config.toml`, every key optional:

```toml
[queue]
watch_labels = ["infra"]      # MRs with these labels land in Watching

[review]
fold = ["*.lock", "*.snap"]   # files that open folded
inline_max_words = 2          # a change reads inline when each side changes at most this many words…
inline_min_same = 60          # …and both lines keep at least this percent of their text

[tui]
theme = "tokyonight"          # dracula, catppuccin, catppuccin-latte, rosepine, rosepine-dawn, nord, tokyonight, monokai

[hosts."git.acme.dev"]
forge = "github"              # a GitHub Enterprise host
```

## Security

Tokens live in the macOS login keychain (service `revu`), never on disk in clear.
Each token goes to its own host only, and redirects are refused.
`GITLAB_TOKEN`, `GITHUB_TOKEN` or `GH_TOKEN` override the keychain for scripts.

## Development

```sh
cargo test                   # unit, snapshots, and HTTP against mock servers
cargo test -- --ignored      # also the real keychain round trip
```

The design lives in [`specs/`](specs), the roadmap in [`specs/06-roadmap.md`](specs/06-roadmap.md).

## License

[MIT](LICENSE)
