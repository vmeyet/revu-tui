# Keys and AZERTY

Add your own keys, and make the bracket keys easy on an AZERTY keyboard.

## AZERTY

On a Mac French keyboard, `[` and `]` need `⌥⇧(` and `⌥⇧)`.
This makes `]n` hard to type.

```toml
[keys]
layout = "azerty"
```

Now `(` and `)` work like `[` and `]`.

| Default | With AZERTY |
|---|---|
| `]n` `[n` | `)n` `(n` |
| `]c` `[c` | `)c` `(c` |
| `]f` `[f` | `)f` `(f` |

The bracket keys keep working.

## Your own keys

```toml
[keys.bind]
next_thread = "N"
prev_thread = ["P", "ctrl-y"]
```

A key you add does what the default key does, in the pane you are in.
It adds to the default key, it does not replace it.
`?` shows your keys next to the default ones.

A key can be one character, two in a row like `")x"`, or a name like `ctrl-e` or `tab`.
revu stops at start if a key already has a job, and names the action that owns it.

## See also

- [Keys reference](../reference/keys.md), with every action you can bind.
- [Troubleshooting: the Option key](../troubleshooting.md).
