# AI

Two optional helpers: Jev marks what matters in the queue, and Claude answers questions about the diff.

Both are off until you switch them on.
Switching one on sends MR content to its provider.

## Switch them on

1. Store the key in the keychain.
2. Switch the provider on in the config.

```sh
revu ai login typesafe     # Jev
revu ai login anthropic    # Claude; revu checks the key before it saves it
revu ai status             # which provider is on, and where its key comes from
```

```toml
[ai.typesafe]
enabled = true

[ai.anthropic]
enabled = true
model = "claude-opus-5"
```

`TYPESAFE_API_KEY` and `ANTHROPIC_API_KEY` override the keychain.

## What Jev does

| Where | Shows |
|---|---|
| Queue | `!` on urgent MRs, and a mark on MRs whose last comment waits on you |
| Queue sort | `s` gains an urgency order |
| File tree | Risky files are tinted |

`:ai off` stops Jev until you type `:ai on`.

## See also

- [Open an agent session on an MR](agent.md), for Claude Code itself on the MR's branch.
- [Config reference: `[ai]`](../reference/config.md).
- [Troubleshooting](../troubleshooting.md).
