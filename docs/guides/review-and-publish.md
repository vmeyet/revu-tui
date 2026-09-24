# Review and publish

Comment on lines, reply in threads, then publish everything as one review.

## See the whole MR first

`i` opens the MR's cover.
It shows the description, the pipeline, who reviews and the open threads.
Each thread reads its comment first, then its file name and line.
`j` `k` walk the threads, and the bottom line shows the full path of the one you are on.
`enter` goes there in the diff.
`k` above the first thread, or `g`, takes you back to the top.
`p` opens the pipeline, `esc` closes the cover.
The file tree, `t`, lists the files, with `◆n` for the unresolved threads in each.

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
`⌥←` and `⌥→` jump a word, `⌥⌫` deletes the word before the cursor.
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
| `]n` `[n` | Next, previous line with a conversation; a folded file or hunk on the way opens |
| `r` | Reply, as a draft |
| `R` | Resolve or unresolve |
| `q`, `x` or `esc` | Close the pane |
| `e` `d` | Edit, delete your draft |
| `S` | Apply the suggestion under the cursor, after a `y` |
| `+` | React to the note under the cursor |

## React

`+` opens the eight reactions both forges share: 👍 👎 😄 😕 💖 🎉 🚀 👀.
Press `1` to `8`, or move with `h` `l` and press `enter`.
Your reaction shows at once, and a second pick takes it off.
Reactions show under each note, yours in the accent colour.
Set `[tui] ascii = true` when your terminal draws emoji at the wrong width.

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

## Draft or ready

`H` marks your own MR ready for review when it is a draft, and a draft when it is ready.
revu does not ask first: `H` again takes it back.
A toast names the new state, and the header and the queue follow.
revu refuses, and says why, when the MR is not yours or is no longer open.
`revu ready !42` does the same from the shell, and `--undo` makes it a draft again.

## Merge

`M` merges your own MR once it is approved.
revu asks first, and names the method: `merge !42 into main (squash)?`.
`y` merges, any other key cancels.
revu refuses, and says why, when the MR is not yours, is a draft, has conflicts, has a failed pipeline, or needs more approvals.
The method follows the project: squash when it squashes, a merge commit otherwise.
If someone pushes after you looked, the forge refuses the merge: press `r` and look at the new commits.
`revu merge !42` does the same from the shell, and `--yes` skips the question.

## See also

- [Review mode](review-mode.md), to go through every MR that needs you.

- [Concepts: draft and review](../concepts.md#draft).
- [Keys](../reference/keys.md).
