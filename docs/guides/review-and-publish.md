# Review and publish

Comment on lines, reply in threads, then publish everything as one review.

## Comment

| Key | Does |
|---|---|
| `c` | Comment on the line under the cursor |
| `V`, then `j` `k`, then `c` | Comment on a range of lines |
| `C` | Comment on the old side of a line shown inline |
| `s` | Suggest a change: the box starts with the lines to edit |
| `E` | Write the comment in `$EDITOR` |

The comment box sits at the bottom of the right pane.
`enter` saves the draft.
`⌥enter` adds a new line.
`ctrl-o` moves the text to `$EDITOR`.
`esc` leaves the box and keeps the text for later.

## Read and answer threads

A line with a conversation has a mark left of its numbers.

| Mark | Means |
|---|---|
| `◆` | An open thread |
| `◇` | Your draft |
| `✓` | A resolved thread |

| Key | Does |
|---|---|
| `enter` or `l` on a marked line | Open its threads in the right pane |
| `]n` `[n` | Next, previous line with a conversation |
| `r` | Reply, as a draft |
| `R` | Resolve or unresolve |
| `e` `d` | Edit, delete your draft |
| `S` | Apply the suggestion under the cursor, after a `y` |

## Publish

1. Press `P`.
2. Move through your drafts with `j` `k`.
3. Press `a` to approve in the same step.
4. Press `enter` to publish.

| Key in the publish list | Does |
|---|---|
| `e` | Edit the draft |
| `d` | Delete the draft |
| `m` | Move a draft whose line is gone to the MR itself |

`A` approves or unapproves without publishing.

## See also

- [Concepts: draft and review](../concepts.md#draft).
- [Keys](../reference/keys.md).
