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
You need macOS and Rust 1.90+ (`curl -sSf https://sh.rustup.rs | sh`).

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
Outside one (or with `--all`), the queue and `revu list` merge every host you logged in to, each row tagged with its host.

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
| `enter` / `l` | Open the MR, or the threads of a marked line (`◆` `◇` `✓` left of the numbers); `enter` on a file or hunk folds it |
| `tab`, `]c`, `]n` | Next file, hunk, line with a conversation |
| `za`, `zM` `zR` | Fold one, fold all, unfold all |
| `zo` `zc` (queue), `zh` | Open or fold the section under the cursor, fold the MR header |
| `D` | One-word changes inline (`2;` struck, `20;` after it), or every line split |
| `i` | The MR description |
| `t` | File tree in the right pane; `enter` jumps to a file |
| `p` | The pipeline in the right pane: jobs by stage, failures first; `o` opens a job, `r` refreshes |
| `zv` | Mark the file viewed: it folds, and unfolds again if the author changes it |
| `zz`, `w`, `W` | Reading mode, wrap long lines, hide whitespace-only changes |
| `+` | Ten more unchanged lines around the hunk |
| `v` | The file as it is after the change, in your own program at this line (`:view old` for before) |
| `c` | New thread on the line, written in a box at the bottom of the pane (`V` first for a range, `s` for a suggestion, `C` for the old side of an inline change); `enter` saves the draft, `⌥enter` adds a line, `ctrl-o` moves the text to `$EDITOR`, `esc` keeps it for later |
| `r` / `R` | Reply / resolve, in the right pane (`R` also on a marked line) |
| `S` | Commit the suggestion of the note under the cursor on the MR branch, after a `y` |
| `J` `K`, `e`, `d`, `x` | In the pane: next, previous thread; edit, delete my draft; close |
| `P` | Publish every draft as one review: `enter` sends, `e` edits, `a` also approves |
| `a` then `e` `r` `s` `t` `c` `a` | Ask Claude: explain the hunk, the file's risks, a summary of the MR, this thread, a comment about the lines, or anything (`:ask …` too) |
| `c` `enter` `R` `y` (answer) | Turn the answer into a draft, ask a follow-up, ask again past the cache, copy it |
| `o` / `y` | Open in the browser / copy the link |
| `:` | Command line: `:go !42`, `:approve`, `:publish`, `:all`, `:view old`, `:set theme=nord`; tab completes |
| `ctrl-k` | Jump to a file of the open MR, or to another MR |
| `?` | Every key |

Comments stay drafts until `P`: GitLab draft notes, or your pending review on GitHub.
They survive a restart, and nothing is public before you publish.

**Your own keys.**
On a Mac French AZERTY keyboard, `[` and `]` take `⌥⇧(` and `⌥⇧)`, so `]n` is three keys and a chord.
`layout = "azerty"` under `[keys]` makes `(` and `)` do what `[` and `]` do: `)n` next thread, `(c` previous hunk, `)f` next file with an open thread.
`[keys.bind]` adds a key of your own to any action; it does what the default key does in the pane you are in, and `?` shows it.

```toml
[keys]
layout = "azerty"             # ( and ) work like [ and ]; the brackets keep working

[keys.bind]
next_thread = "N"             # one key, two keys (")x"), a named one ("ctrl-e", "tab"), or a list
prev_thread = ["P", "ctrl-y"]
```

A key revu already reads is refused at startup with the action that owns it, and so are two actions on one key.
The actions: `next_thread` `prev_thread` `next_hunk` `prev_hunk` `next_file_unresolved` `prev_file_unresolved` `next_file` `prev_file` `fold_toggle` `fold_open` `fold_close` `fold_all` `unfold_all` `fold_header` `viewed` `reading` `split` `tree` `pipeline` `wrap` `whitespace` `more_context` `view_file` `description` `comment` `comment_old` `select` `suggest` `editor` `resolve` `publish` `approve` `open_browser` `copy_link` `scope` `filter` `palette` `jump` `help` `quit`.

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

`~/.config/revu/config.toml` (or `$XDG_CONFIG_HOME/revu/`), every key optional.
A config left in `~/Library/Application Support/revu/` by an earlier version moves there on the next run.

```toml
[queue]
watch_labels = ["infra"]      # MRs with these labels land in Watching

[review]
fold = ["*.lock", "*.snap"]   # files that open folded
inline_max_words = 2          # a change reads inline when each side changes at most this many words…
inline_min_same = 60          # …and both lines keep at least this percent of their text

[tui]
theme = "tokyonight"          # dracula, catppuccin, catppuccin-latte, rosepine, rosepine-dawn, nord, tokyonight, monokai
images = true                 # pictures in comments, drawn in the thread pane on Kitty, Ghostty, WezTerm and iTerm2

[notify]
enabled = true                # a macOS notification when an MR lands in To review while revu runs

[keys]
layout = "azerty"             # ( and ) work like [ and ]; see "Your own keys" above

[hosts."git.acme.dev"]
forge = "github"              # a GitHub Enterprise host

[open]
default = "hx"                # what `v` runs; else $VISUAL, $EDITOR, then less

[open.files]
"*.md" = "glow -p"            # by glob, the longest match wins; {file} and {line} place the arguments yourself

[ai.anthropic]
enabled = false               # true sends the MR you ask about to Claude
model = "claude-opus-5"

[ai.typesafe]
enabled = false               # true sends queue and file summaries to TypeSafe's Jev for triage
```

`v` knows how to jump to the line in hx, vim, nvim, nano, micro, emacs, kak, less, bat and glow.
Inside a checkout on the MR's head commit it opens your real file; elsewhere a read-only copy that is deleted when you quit.

## Security

Tokens live in the macOS login keychain (service `revu`), never on disk in clear.
Each token goes to its own host only, and redirects are refused.
Pictures in comments come from the forge only: GitLab uploads through its API, GitHub attachments through its web host, whose signed redirect is fetched without the token.
`GITLAB_TOKEN`, `GITHUB_TOKEN` or `GH_TOKEN` override the keychain for scripts.

AI is off until the config switches a provider on, and its key is a keychain secret too:

```sh
revu ai login anthropic           # hidden prompt, checked against Anthropic before it is stored (`--token -` reads stdin)
revu ai login typesafe            # offers to reuse the key slack-tui already stores
revu ai status                    # which provider is on and where its key comes from, never the key
revu ai logout anthropic
revu ai ask !42                                    # Claude summarises the MR, streamed to stdout (in the TUI: a s)
revu ai ask !42 --file src/a.rs --lines 13-20 is this safe?
```

`ANTHROPIC_API_KEY` and `TYPESAFE_API_KEY` override the keychain.

With `[ai.typesafe]` on, Jev marks the queue: `◆` the last note asks you something, `!` someone is blocked on this review, `~` a sprawling MR; To review sorts by urgency.
In the file tree, a file Jev finds risky reads red (security or auth) or amber (data or schema), a cosmetic one dimmed.
Answers are cached per MR state, so an unchanged MR is never asked twice; if Jev fails, one notice and the plain views.

With `[ai.anthropic]` on, `a` asks Claude about what is under the cursor and the answer streams into the right pane.
The same question on the same diff comes back from the cache at once (`R` asks again); the footer names the model and the tokens the prompt cache saved.
`:ai off` stops every AI call until revu starts again.

## Development

```sh
cargo test                   # unit, snapshots, and HTTP against mock servers
cargo test -- --ignored      # also the real keychain round trip
```

The design lives in [`specs/`](specs), the roadmap in [`specs/06-roadmap.md`](specs/06-roadmap.md).

## License

[MIT](LICENSE)
