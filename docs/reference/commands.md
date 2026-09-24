<!-- Written by `revu docs` from the code. Edit the code, then run `revu docs`. -->
# Commands

`revu` with no command opens the review screen.
Every command also takes the flags below.

| Flag | Means |
|---|---|
| `--host` | The forge host, GitLab or GitHub. Defaults to the checkout's, then the config, then gitlab.com. |
| `--json` | Print machine-readable JSON instead of the pretty output. |
| `--all` | Queue every project, not only the one of the checkout you are in. |

## `revu login`

Store a token in the keychain: GitLab scope `api`, GitHub scope `repo`.

| Argument | Means |
|---|---|
| `HOST` | The forge host: `gitlab.com`, `github.com`, or your own. By default the checkout's, then the configured one. |

| Flag | Means |
|---|---|
| `--from-glab` | Read the token `glab` already holds instead of prompting (GitLab) |
| `--from-gh` | Read the token `gh` already holds instead of prompting (GitHub) |
| `--token` | Read the token from stdin (`-`) instead of prompting. |

## `revu logout`

Forget a host: keychain entry, config and cache.

| Argument | Means |
|---|---|
| `HOST` | Forge host, the configured one by default. |

## `revu whoami`

Show who you are logged in as.

## `revu list`

The merge requests waiting on you, yours, and the ones you watch.

| Flag | Means |
|---|---|
| `--cached` | Print the last fetched queue without touching the network. |

## `revu show`

One merge request: header, files, unresolved threads.

| Argument | Means |
|---|---|
| `MR` | `group/project!42`, `!42` (project from the origin remote), an MR URL, or nothing for the current branch. |

## `revu diff`

The coloured diff of a merge request, through your pager.

| Argument | Means |
|---|---|
| `MR` | `group/project!42`, `!42` (project from the origin remote), an MR URL, or nothing for the current branch. |

## `revu comment`

Post a public comment on a merge request, on a line with `--at path:line`.

| Argument | Means |
|---|---|
| `MR` | `group/project!42`, `!42`, or an MR URL. |
| `TEXT` | The comment, markdown. |

| Flag | Means |
|---|---|
| `--at` | Anchor the comment on a line of the new file: `src/a.rs:13`. |

## `revu approve`

Approve a merge request (`--undo` takes it back)

| Argument | Means |
|---|---|
| `MR` | `group/project!42`, `!42`, an MR URL, or nothing for the current branch. |

| Flag | Means |
|---|---|
| `--undo` | Remove your approval instead. |

## `revu publish`

Publish every draft comment you hold on a merge request as one review.

| Argument | Means |
|---|---|
| `MR` | `group/project!42`, `!42` (project from the origin remote), an MR URL, or nothing for the current branch. |

## `revu share`

Post a merge request through a `[share]` command: a chat channel, a webhook, a script.

| Argument | Means |
|---|---|
| `MR` | `group/project!42`, `!42`, an MR URL, or nothing for the current branch. |

| Flag | Means |
|---|---|
| `--target` | Which `[share.targets.<name>]` to post to; needed when there are several. |
| `--note` | A line of context under the link. |
| `--yes` | Send without asking. |
| `--dry-run` | Print the message and send nothing. |

## `revu ai`

The AI providers: store a key, forget it, see which one is on.

## `revu ai login`

Store a provider's key in the keychain; the config still has to switch it on.

| Argument | Means |
|---|---|
| `PROVIDER` | Which provider. |

| Flag | Means |
|---|---|
| `--token` | Read the key from stdin (`-`) instead of prompting. |
| `--from-slack-tui` | Reuse the TypeSafe key slack-tui keeps in the keychain. |

## `revu ai logout`

Forget a provider's key.

| Argument | Means |
|---|---|
| `PROVIDER` | Which provider. |

## `revu ai status`

Which provider is on and where its key comes from (never the key)

## `revu ai ask`

Ask Claude about a merge request; the answer streams to stdout.

| Argument | Means |
|---|---|
| `MR` | `group/project!42`, `!42`, an MR URL, or `-` for the current branch. |
| `QUESTION` | The question; a summary of the MR (or an explanation of the file) without one. |

| Flag | Means |
|---|---|
| `--file` | Ask about one file instead of the whole MR. |
| `--lines` | With `--file`: the new-side lines asked about, `13` or `13-20`. |

## `revu tui`

Interactive review client.

## `revu completions`

Generate shell completions.

| Argument | Means |
|---|---|
| `SHELL` | The shell to write the script for. |

## `revu update`

Rebuild and install the latest `revu` with cargo.

| Flag | Means |
|---|---|
| `-f, --force` | Install even when the running binary is already the latest commit. |
