# 07 · Forges

`mr` reviews merge requests on GitLab and, next, pull requests on GitHub.
Everything above `src/forge/` speaks one neutral model; each forge converts its own wire shapes at its edge.
Phase 1 (done 2026-09-22) put GitLab behind the seam with no behaviour change.
Phase 2 adds `src/forge/github/` and a `Forge::GitHub` variant.

## The seam

```
review/  tui/  commands/          neutral model only
        │
        ▼
forge::Forge  (enum, static dispatch: one match per method)
        │
        ├── gitlab::Client   REST + GraphQL, wire types private to it
        └── github::Client   phase 2
```

`Forge` is an enum, not a trait object: async methods stay plain `async fn`, and adding GitHub is a new variant plus one arm per method.

`Kind` names the forge of a host.
`Kind::for_host(host, config)`: `github.com` is GitHub, any other host GitLab, unless the config says otherwise:

```toml
[hosts."git.acme.dev"]
forge = "github"      # a GitHub Enterprise host, which the name alone cannot tell
```

`Kind` also carries what is pure and forge-shaped: the sigil between project and number (`!` or `#`) and the web URL of one diff line.

## Neutral model (`src/forge/model.rs`, `src/forge/queue.rs`)

| Type | Holds |
|---|---|
| `MrKey { project, number }` | The one address of an MR: `group/sub/project` + iid on GitLab, `owner/repo` + number on GitHub. Keys the cache and every forge call. |
| `Mr` | Header data: title, description, author, branches, `refs`, pipeline, conflicts, labels, `approvals`. |
| `Refs { base, start, head }` | The diff a review reads. GitLab tells `start` apart from `base`; GitHub repeats `base`. |
| `DiffFile` | One file: unified body from the first `@@`, both paths, added/deleted/renamed flags, modes when known, `too_large`. |
| `Discussion { id, notes }`, `Note` | A thread and its notes; `Note.position` says where the first one hangs. |
| `Position { refs, old_path, new_path, line, start }` | Where a note hangs: one line, or `start..=line` in one file. |
| `LineRef { old, new }` | One diff line by its numbers: removed = old only, added = new only, context = both. `side()` is New when `new` exists. |
| `Draft`, `NewDraft` | A held unpublished note (with its forge id), and the payload to create or replace one. |
| `Queue`, `QueueMr`, `Sections` | The queue answers and their pure split into To review, Mine, Watching, Open, Done. |

The numeric GitLab project id is gone from keys and cache paths.
GitLab accepts the URL-encoded path wherever it takes the id, so no lookup is needed; `123!42` still works through `Forge::project_path(123)`.

## `Forge` methods

```rust
fn connect(kind: Kind, credentials: &Credentials) -> Result<Forge>
fn kind(&self) -> Kind
fn host(&self) -> &str
async fn me(&self) -> Result<User>
async fn queue(&self, project: Option<&str>) -> Result<Queue>
async fn project_path(&self, id: u64) -> Result<String>
async fn mr_for_branch(&self, project: &str, branch: &str) -> Result<Option<u64>>
async fn mr(&self, key: &MrKey) -> Result<Mr>
async fn diffs(&self, key: &MrKey) -> Result<Vec<DiffFile>>
async fn discussions(&self, key: &MrKey) -> Result<Vec<Discussion>>
async fn drafts(&self, key: &MrKey) -> Result<Vec<Draft>>
async fn create_draft(&self, key: &MrKey, draft: &NewDraft) -> Result<Draft>
async fn update_draft(&self, key: &MrKey, id: u64, draft: &NewDraft) -> Result<Draft>
async fn delete_draft(&self, key: &MrKey, id: u64) -> Result<()>
async fn publish(&self, key: &MrKey, approve: bool) -> Result<()>
async fn resolve(&self, key: &MrKey, discussion: &str, resolved: bool) -> Result<()>
async fn approve(&self, key: &MrKey, approve: bool) -> Result<()>
async fn comment(&self, key: &MrKey, body: &str, position: Option<&Position>) -> Result<Discussion>
```

`publish` takes `approve` because GitHub submits a review and its verdict in one call; GitLab publishes, then approves.

## GitLab ↔ GitHub

| Concern | GitLab (done) | GitHub (phase 2) |
|---|---|---|
| Queue | GraphQL `currentUser.{reviewRequested,authored,assigned}MergeRequests`, plus `project.mergeRequests` when scoped | GraphQL `search(type: ISSUE, query: "is:pr is:open review-requested:@me")`, `author:@me`, `assignee:@me`, plus `repo:<owner/repo> is:pr is:open` when scoped |
| Review state | `mergeRequestInteraction.reviewState` | `latestReviews` per reviewer: APPROVED, CHANGES_REQUESTED, COMMENTED, PENDING → `ReviewState` |
| One MR | `GET projects/:path/merge_requests/:iid` + `/approvals` | `GET repos/:owner/:repo/pulls/:n` + `/reviews` |
| Diffs | `GET …/diffs` (paged), body per file | `GET repos/…/pulls/:n/files` (paged), `patch` per file; no `patch` means binary or too large |
| Threads | `GET …/discussions`, notes carry `position` | GraphQL `pullRequest.reviewThreads { isResolved comments { path line originalLine diffSide startLine } }` plus issue comments as unanchored threads |
| Position out | `base_sha`, `start_sha`, `head_sha`, `old_line`/`new_line`, `line_range` with `line_code = sha1(path)_old_new` | `commit_id = refs.head`, `path`, `side = RIGHT` when `line.new` exists else `LEFT`, `line = line.number()`, `start_line`/`start_side` from `start` |
| Drafts | `…/draft_notes` CRUD, per user, survive sessions | The pending review: `POST pulls/:n/reviews` without `event` holds comments; list with `GET reviews/:id/comments`; one pending review per user |
| Publish | `POST …/draft_notes/bulk_publish`, then `/approve` when asked | `POST pulls/:n/reviews/:id/events` with `event: COMMENT` or `APPROVE` |
| Resolve | `PUT …/discussions/:id resolved=` | GraphQL `resolveReviewThread` / `unresolveReviewThread` |
| Approve | `POST …/approve`, `…/unapprove` | Submit a review with `APPROVE`; taking it back is `PUT reviews/:id/dismissals` (needs rights) or a `REQUEST_CHANGES`/`COMMENT` review: the UI says which |
| Suggestions | ```` ```suggestion:-0+0 ```` fence | ```` ```suggestion ```` fence over the commented lines; the range comes from `start`/`line` |
| Line URL | `web_url/diffs#sha1(path)_old_new` | `web_url/files#diff-sha256(path)R<new>` or `L<old>` (phase 1 links to `web_url/files`) |
| Sigil | `group/project!42` | `owner/repo#42` |

## Auth

Tokens stay in the keychain, one account per host (`04-security.md`).
Login borrows the token the forge's own CLI already holds:

| Forge | Seed | Scope |
|---|---|---|
| GitLab | `mr login --from-glab` reads `glab auth status --show-token` | `api` |
| GitHub | `mr login --from-gh` reads `gh auth token` | `repo` |

Without the CLI and without `--token -`, login stops with one line naming the missing tool and the token page to create one.
`GITLAB_TOKEN` stays the env override for GitLab; phase 2 reads `GITHUB_TOKEN` for GitHub hosts.

## What changed in phase 1 that a user can see

Nothing on screen or in plain output: `mr list`, `list --all`, `show`, `diff` and the TUI frames are byte-identical on the same data.
`--json` output speaks the neutral model: `number` instead of `iid`, `project` (the path) instead of `project_id`, no numeric `id`.
Cache paths moved from `mr/<project_id>/<iid>/` to `mr/<group+project>/<number>/`; old entries are ignored, so saved folds and viewed files of MRs opened before the upgrade start fresh.
