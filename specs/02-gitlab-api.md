# 02 · GitLab API

Everything below was probed live on gitlab.com (19.5.0-pre) on 2026-09-22 with a personal access token.
`glab` is used for one thing only: `revu login --from-glab` reads the token it already holds (`glab auth status --show-token`).
All other traffic is direct HTTPS from the binary: spawning `glab` per action costs 100 ms+ and it has no draft-note support.

## Auth

- Header `PRIVATE-TOKEN: <pat>` on REST and GraphQL.
- Scope `api`. `read_api` cannot post notes, approve or publish drafts.
- Base URLs: `https://<host>/api/v4/` and `https://<host>/api/graphql`.
- Verify a token with `GET /user` → `{ id, username, name, avatar_url }`. Store `username` and `id` in config, not the answer.

## Rate limits

Observed headers on every answer:

```
Ratelimit-Limit: 2000
Ratelimit-Remaining: 1992
Ratelimit-Reset: <unix seconds>
```

On `429` wait until `Ratelimit-Reset` (or `Retry-After`) then retry once.
The client reads `Ratelimit-Remaining` and, under 100, slows polling to the reset.

## Pagination

REST lists answer with a `Link: <…>; rel="next"` header and `X-Total`, `X-Next-Page`.
`Client::get_all` follows `rel="next"` until absent, with `per_page=100`.

## The queue (GraphQL, two or three calls in parallel)

Run inside a git checkout whose `origin` lives on the configured host, the queue is scoped to that project (`revu --all` or `*` in the TUI widens it to every project).
Scoped, a second query runs in parallel, `project(fullPath: $project) { mergeRequests(state: opened, first: 100, sort: UPDATED_DESC) { ...list } }`, and the four sections below keep only that project's MRs.
One query for both is refused: it scores 359 against GitLab's complexity limit of 250 (the project query alone scores 114, verified 2026-09-22).
The MRs asking me (`reviewRequested`, `assigned`) and mine (`authored`) are two queries: the "needs me" rules read `approvalsLeft` and `commenters { nodes { username } }` on the first and on the project's, and with them one query for all three lists scores 251.
Measured 2026-09-23 with `queryComplexity { score limit }`: asking me 185, authored 87, project 124.
GitLab answers `approvalsLeft: 0` when a project requires no approval at all, so "approved enough" also needs at least one approval.
The fragment also asks for `description`, so the description modal opens from the queue without a request.

Verified: `currentUser.reviewRequestedMergeRequests`, `assignedMergeRequests`, `authoredMergeRequests` exist and accept `state: opened`.

```graphql
query Queue($after: String) {
  currentUser {
    username
    reviewRequested: reviewRequestedMergeRequests(state: opened, first: 50, sort: UPDATED_DESC) { ...list }
    authored:        authoredMergeRequests(state: opened, first: 50, sort: UPDATED_DESC)        { ...list }
    assigned:        assignedMergeRequests(state: opened, first: 50, sort: UPDATED_DESC)        { ...list }
  }
}
fragment list on MergeRequestConnection {
  nodes {
    id iid title draft webUrl updatedAt createdAt
    sourceBranch targetBranch conflicts
    project { id fullPath }
    author { username name avatarUrl }
    approved approvedBy { nodes { username } }
    reviewers { nodes { username mergeRequestInteraction { reviewState } } }
    headPipeline { status detailedStatus { label } }
    diffStatsSummary { additions deletions fileCount }
    resolvableDiscussionsCount resolvedDiscussionsCount
    labels { nodes { title } }
    userNotesCount
  }
}
```

`reviewState` is one of `UNREVIEWED`, `REVIEWED`, `REQUESTED_CHANGES`, `APPROVED`, `REVIEW_STARTED`, `UNAPPROVED`.
Global GraphQL ids look like `gid://gitlab/MergeRequest/123456789`; the REST id is the trailing number.

Sections are derived client side:

| Section | Rule |
|---|---|
| To review | in `reviewRequested` and my `reviewState` is not `APPROVED` or `REVIEWED` |
| Mine | in `authored` |
| Watching | in `assigned` or carries a `queue.watch_labels` label, and not above |
| Open | scoped only: every other open MR of the project, whoever wrote or reviews it |
| Done | in `reviewRequested` and already approved or reviewed by me (folded by default) |

Fallback if GraphQL is unavailable (self-hosted with it disabled): `GET /merge_requests?scope=all&state=opened&reviewer_username=<me>` and `author_username=<me>`, which was also verified to answer.

## One MR (REST)

```
GET /projects/:project_id/merge_requests/:iid
```

Fields used: `id iid title description state draft author source_branch target_branch web_url updated_at sha diff_refs{base_sha head_sha start_sha} head_pipeline{status web_url} changes_count has_conflicts blocking_discussions_resolved user{can_merge} reviewers[] assignees[] labels[]`.

Approvals: `GET …/approvals` → `{ approved, approved_by: [{user}], approvals_left }`.

## Diffs

```
GET /projects/:project_id/merge_requests/:iid/diffs?page=1&per_page=100
```

One element per file:

```json
{
  "diff": "@@ -0,0 +1,5606 @@\n+{\n+  \"id\": …",
  "new_path": "…", "old_path": "…",
  "a_mode": "0", "b_mode": "100644",
  "new_file": true, "renamed_file": false, "deleted_file": false,
  "generated_file": false, "too_large": false, "collapsed": false
}
```

Verified shape. `diff` is the unified body only, hunks start at `@@`.
The endpoint pages; a 60-file MR is one page, a 300-file MR is three.
Do not use `…/changes`: deprecated, unpaginated, and it truncates.

`GET …/versions` lists diff versions; v1 does not need it, `diff_refs` on the MR is enough for positions.

## Discussions (threads)

```
GET /projects/:project_id/merge_requests/:iid/discussions?per_page=100
```

```json
[{
  "id": "6a9c1750…",
  "individual_note": false,
  "notes": [{
    "id": 123, "type": "DiffNote" | "DiscussionNote" | null,
    "body": "…", "author": {…}, "created_at": "…", "updated_at": "…",
    "system": false, "resolvable": true, "resolved": false, "resolved_by": null,
    "position": {
      "base_sha": "…", "head_sha": "…", "start_sha": "…",
      "position_type": "text",
      "old_path": "src/a.rs", "new_path": "src/a.rs",
      "old_line": null, "new_line": 42,
      "line_range": { "start": {…}, "end": {…} } | null
    }
  }]
}]
```

Verified: MR-level notes carry `type: null, resolvable: false, position: null`; skip notes with `system: true` for the thread pane but count them as activity.
A thread is anchored when its first note has a `position` with `position_type == "text"`.
Anchor to the diff by `(new_path, new_line)` for added and context lines, `(old_path, old_line)` for removed lines.
Threads whose anchor is not in the current diff (outdated) render in the file's tail under `outdated`.

Write:

```
POST /projects/:pid/merge_requests/:iid/discussions           body=…  position[…]     new thread (immediately public)
POST /projects/:pid/merge_requests/:iid/discussions/:did/notes body=…                 reply (immediately public)
PUT  /projects/:pid/merge_requests/:iid/discussions/:did       resolved=true|false
```

The TUI does not call the two POSTs directly: every comment goes through draft notes so `P` is the one publishing key.
The `revu comment` subcommand may post directly, since a script means it.

## Draft notes (review mode)

```
GET    /projects/:pid/merge_requests/:iid/draft_notes                     → []  (verified)
POST   /projects/:pid/merge_requests/:iid/draft_notes
         note=…  [in_reply_to_discussion_id=…]  [position[…]]  [resolve_discussion=true]
PUT    /projects/:pid/merge_requests/:iid/draft_notes/:id                 note=… position[…]   resend the position: a PUT with the note alone drops it and the draft turns into an MR-level comment (verified 2026-09-22)
DELETE /projects/:pid/merge_requests/:iid/draft_notes/:id
PUT    /projects/:pid/merge_requests/:iid/draft_notes/:id/publish         one
POST   /projects/:pid/merge_requests/:iid/draft_notes/bulk_publish        all
```

Drafts are per user and survive across sessions and the web UI: the TUI shows GitLab's drafts under `◇` and its own unsent local drafts identically, syncing local → GitLab as soon as the network answers.
Publishing with `P` calls `bulk_publish`, then optionally `POST …/approve` when the publish modal has `approve` ticked.

## Position payload

For a note on a line:

```
position[position_type]=text
position[base_sha]=<diff_refs.base_sha>
position[head_sha]=<diff_refs.head_sha>
position[start_sha]=<diff_refs.start_sha>
position[old_path]=<file.old_path>
position[new_path]=<file.new_path>
position[new_line]=<n>        added or context line
position[old_line]=<n>        removed line; context lines may carry both
```

Multi-line (`V` then `c`):

```
position[line_range][start][line_code]=<sha1(new_path)>_<old>_<new>
position[line_range][start][type]=new|old|null
position[line_range][end][line_code]=…
position[line_range][end][type]=…
```

Live answers carry `old_line`, `new_line` and `type` in `start`/`end` but no `line_code` (it is `Option` in `types.rs`); the writer builds it.
`line_code` is `sha1(file_path) + "_" + old_line + "_" + new_line`, with `0` for the missing side; hash the `new_path` for added/context lines and `old_path` for removed ones.
A range must stay inside one file; crossing a hunk boundary is allowed.

## Suggestions

A note body containing

````
```suggestion:-0+0
new text
```
````

renders as an applicable suggestion in GitLab. `-0+0` means "replace this line"; `-1+2` extends one line up and two down.
The `s` key prefills the editor with the selected lines inside such a block.

## Approve

```
POST /projects/:pid/merge_requests/:iid/approve
POST /projects/:pid/merge_requests/:iid/unapprove
```

`approve` answers 401 when the token owner cannot approve (own MR, rules); show the message, do not retry.

## Mark viewed

GitLab exposes no public endpoint for "viewed" files, so `viewed` is local only: cache `state.json`, keyed by `new_path` and `head_sha`.

## Users and avatars

`GET /users?username=<u>` for the odd lookup; avatars are not rendered (no images in v1).
The author is shown as an initial in a coloured cell, colour from `Theme::user(name)`.

## Open in browser

`web_url` for the MR; `web_url#note_<id>` for a note; `web_url/diffs#<sha1(new_path)>_<old>_<new>` for a line.

## Errors to expect

| Status | Meaning | UI |
|---|---|---|
| 401 | bad token or scope | toast `token rejected, run mr login` |
| 403 | no access to project or action | toast with GitLab's message |
| 404 | MR gone or project private | drop from queue with a toast |
| 409 | draft already published | refresh drafts |
| 429 | rate limited | backoff, status line shows `⏳ 42s` |
