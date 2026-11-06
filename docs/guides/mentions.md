# Mentions

revu shows where someone mentioned you, or wrote to you, on an MR, so you can go there.
It reads them from your GitLab to-do list: only the `mentioned` and `directly addressed` items, nothing else.

## In the queue

| You see | Means |
|---|---|
| `@` at the end of a row | Someone mentioned you on this MR and you have not answered |
| MENTIONS | Mentioned MRs no other section shows: approved by others, not yours, merged or closed |
| `merged`, `closed` under a MENTIONS row | The MR is finished; it leaves two weeks after |

MENTIONS hides when it is empty, and the `@` column only shows while a mention waits.
Inside a checkout, mentions keep to that project; `*` shows every project.

## Go there

`enter` on a mentioned MR opens it with the cursor on the thread that mentions you.
`]n` still walks every open thread, as before.

## Mark it done

| You do | Mentions marked done |
|---|---|
| `x` on the row | Every mention of the MR |
| Reply to the thread with `⌘enter` | The mentions in that thread |
| Resolve the thread with `R` | The mentions in that thread |

GitLab marks them done too when a comment of yours is published on the MR.
Merging or closing the MR does not mark them done, so revu drops a finished MR's mentions after two weeks.
`x` is bindable as `mention_done` in `[keys.bind]`.

## Limits

- GitHub PRs show no mentions yet.
- `revu local` has no forge, so no mentions.
- A host whose token cannot read the to-do list shows none, and says why in the log.

## See also

- [Concepts: section](../concepts.md#section).
- [Review and publish](review-and-publish.md), for replies and resolving.
