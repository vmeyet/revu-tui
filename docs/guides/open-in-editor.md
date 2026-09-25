# Open a file in your editor

`^v` opens the file under the cursor, as it is after the change, in the program you choose.

## Use it

1. Put the cursor on a line of the diff.
2. Press `^v`.
3. Quit your program to come back to revu.

The program opens at the cursor's line.
`:view old` opens the file as it was before the change.
`:view src/a.rs:42` opens any file of the MR at a line.

## Which file opens

| Where you run revu | revu opens |
|---|---|
| A checkout on the MR's latest commit | The real file; your edits are real |
| Anywhere else | A read-only copy, deleted when you quit |

## Choose the program

```toml
[open]
default = "hx"

[open.files]
"*.md" = "glow -p"
```

The longest matching glob wins.
Without `[open]`, revu uses `$VISUAL`, then `$EDITOR`, then `less`.
revu knows how to open a line in hx, vim, nvim, nano, micro, emacs, kak, less, bat and glow.
For another program, place `{file}` and `{line}` in the command yourself.

## See also

- [Config reference: `[open]`](../reference/config.md).
