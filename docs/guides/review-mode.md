# Review mode

Review mode walks the MRs that need you like an inbox.

## Move through them

| Key | Does |
|---|---|
| `]r` | Open the next MR that needs you |
| `[r` | Open the previous one |
| `enter` after `P` | Open the next one; `esc` stays |

The order is READY first, then TO REVIEW, as the queue sorts them.
MRs you already approved are skipped.
`]r` works from the queue and from the diff, so you never go back to the queue.

## See how far you got

`zv` marks a file viewed.
The review header shows `viewed 7/12` with a small bar.
Files that open folded (lock files, huge or binary files) wait apart as `· 3 folded` until you mark them viewed.
The queue shows the same count on the row of each MR you started.

## Pick up where you stopped

revu remembers the file and line under your cursor in each MR.
Opening the MR again puts you back there, even after the author pushed changes to other files.

## See also

- [Review and publish](review-and-publish.md).
- [Concepts: what "needs me" means](../concepts.md#what-needs-me-means).
