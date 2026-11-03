# AI

Jev is an optional helper that marks what matters in the queue.

It is off until you switch it on.
Switching it on sends MR content to its provider.

## Switch it on

1. Store the key in the keychain.
2. Switch the provider on in the config.

```sh
revu ai login typesafe     # Jev
revu ai status             # whether it is on, and where its key comes from
```

```toml
[ai.typesafe]
enabled = true
```

`TYPESAFE_API_KEY` overrides the keychain.

## What Jev does

| Where | Shows |
|---|---|
| Queue | `!` on urgent MRs, and a mark on MRs whose last comment waits on you |
| Queue sort | `s` gains an urgency order |
| File tree | Risky files are tinted |

`:ai off` stops Jev until you type `:ai on`.

## See also

- [Open an agent session on an MR](agent.md), to ask Claude Code about the MR on its branch.
- [Config reference: `[ai]`](../reference/config.md).
- [Troubleshooting](../troubleshooting.md).
