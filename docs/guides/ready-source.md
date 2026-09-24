# Ready source

A ready source tells revu which MRs their authors say are ready for review.
revu shows them in READY, right under MINE.

## Set one

Give revu a command that prints MR links.
Any output works: plain text, JSON, a chat export.

```toml
[queue.ready]
command = "slack messages '#review' --since 14d --json"
```

revu runs the command without a shell, on every queue refresh.
It keeps the last answer, so the queue paints at once.

## What lands in READY

| An MR from the output | Goes to |
|---|---|
| Open and needs you | READY |
| Merged or closed | Nowhere |
| Approved enough, stale or not ready | OTHER, like any MR |

An MR in READY does not show again in another section.
Inside a checkout, READY keeps to that project; `*` shows every project.

## Examples

| Source | Command |
|---|---|
| A chat channel | `slack messages '#review' --since 14d --json` |
| A GitLab label | `glab mr list --label ready-for-review --output json` |
| A GitHub label | `gh pr list --label ready-for-review --json url` |
| Your own script | `~/bin/ready-mrs` |

## When it fails

The command has 10 seconds and 1 MB of output.
A failure shows its error once, and the queue keeps the last good answer.

## See also

- [Share an MR](share.md), to post one where this command reads.

- [Concepts: what "needs me" means](../concepts.md#what-needs-me-means).
- [Config reference](../reference/config.md).
