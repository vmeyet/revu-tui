# AI

Two optional helpers: Jev marks what matters in the queue, and Claude answers questions about the diff.

Both are off until you switch them on.
Switching one on sends MR content to its provider.

## Switch them on

1. Store the key in the keychain.
2. Switch the provider on in the config.

```sh
revu ai login typesafe     # Jev; add --from-slack-tui to reuse slack-tui's key
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

## Ask Claude

Press `a`, then a letter.

| Keys | Asks about | You get |
|---|---|---|
| `a e` | The change under the cursor | What it does and why, in 2 or 3 sentences |
| `a r` | The file | At most 5 risks, worst first |
| `a s` | The MR | At most 5 bullets and a reading order |
| `a t` | The thread | What is agreed, what is open, who acts next |
| `a c` | The selected lines | A review comment, ready to edit |
| `a a` | Anything | Your own question |

The answer streams into the right pane.

| Key on an answer | Does |
|---|---|
| `c` | Turn it into a draft on those lines |
| `enter` | Ask a follow-up |
| `R` | Ask again, past the cache |
| `y` | Copy it |

`:ai off` stops both helpers until you type `:ai on`.

## See also

- [Config reference: `[ai]`](../reference/config.md).
- [Troubleshooting](../troubleshooting.md).
