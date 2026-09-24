# revu

Review GitLab merge requests and GitHub pull requests in your terminal, with the keyboard only.

> [!IMPORTANT]
> revu runs on macOS only for now.
> It is built for personal use: use it at your own risk.

## Install

```sh
cargo install --git https://github.com/vmeyet/revu-tui
revu login --from-glab            # GitLab: reuse the token glab holds
revu login github.com --from-gh   # GitHub: reuse the token gh holds
```

You need Rust 1.90 or newer.
`revu update` installs the latest version later.

## A 30-second tour

```sh
cd ~/code/widgets   # any checkout of a GitLab or GitHub project
revu
```

| You see | You do |
|---|---|
| The queue: MRs that wait on you, yours, and the rest of the repo | `j` `k` to move, `enter` to open one |
| The diff of that MR, with folds and syntax colours | `]c` next change, `za` fold, `c` comment on a line |
| Your comments, kept as drafts | `P` publishes them all as one review, `M` merges your approved MR |
| Anything else | `⌘K` or `ctrl-k` to search, `?` for every key |

## Learn more

| Page | For |
|---|---|
| [Start here](docs/start.md) | Your first review, step by step |
| [Concepts](docs/concepts.md) | The words revu uses, and how it works |
| [Guides](docs/guides/) | One task per page: review mode, search, ready source, share, zen, AI, your editor, keys, hosts, update |
| [Keys](docs/reference/keys.md), [config](docs/reference/config.md), [commands](docs/reference/commands.md) | Every key, setting and command |
| [Troubleshooting](docs/troubleshooting.md) | When something does not work |

Contributors: read [`AGENTS.md`](AGENTS.md) and the design in [`specs/`](specs/).

## License

[MIT](LICENSE)
