# 07 · Forges

`revu` reviews merge requests on GitLab and pull requests on GitHub.
Everything above `src/forge/` speaks one neutral model; each forge converts its own wire shapes at its edge.
Phase 1 (done 2026-09-22) put GitLab behind the seam with no behaviour change.
Phase 2 (done 2026-09-22) added `src/forge/github/` and the `Forge::GitHub` variant, at parity with GitLab for every method.

## The seam

```
review/  tui/  commands/          neutral model only
        │
        ▼
forge::Forge  (enum, static dispatch: one match per method)
        │
        ├── gitlab::Client   REST + GraphQL, wire types private to it
        └── github::Client   GraphQL + REST, wire types private to it
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
async fn set_draft(&self, key: &MrKey, draft: bool) -> Result<()>
async fn comment(&self, key: &MrKey, body: &str, position: Option<&Position>) -> Result<Discussion>
```

`publish` takes `approve` because GitHub submits a review and its verdict in one call; GitLab publishes, then approves.

## GitLab ↔ GitHub

| Concern | GitLab | GitHub |
|---|---|---|
| Queue | GraphQL `currentUser.{reviewRequested,authored,assigned}MergeRequests`, plus `project.mergeRequests` when scoped | One GraphQL call with four `search(type: ISSUE)` aliases: `review-requested:@me`, `author:@me`, `assignee:@me`, `reviewed-by:@me -author:@me` (feeds Done), each with ` repo:owner/repo` when scoped; plus `repository.pullRequests(states: OPEN)` in parallel for Open. Cost 3 points. |
| Review state | `mergeRequestInteraction.reviewState` | `latestReviews` per reviewer: APPROVED, CHANGES_REQUESTED, COMMENTED, PENDING → `ReviewState` |
| One MR | `GET projects/:path/merge_requests/:iid` + `/approvals` | One GraphQL `pullRequest` query: reviews, requests, decision, checks rollup, `baseRefOid`/`headRefOid` |
| Diffs | `GET …/diffs` (paged), body per file | `GET repos/…/pulls/:n/files` (paged), `patch` per file; no `patch` means binary or too large |
| Threads | `GET …/discussions`, notes carry `position` | One GraphQL query: `reviewThreads` (side, line, range, resolved; outdated ones fall back to `originalLine`), review summaries and PR comments as unanchored threads. Thread ids are node ids (`PRRT_…`). |
| Position out | `base_sha`, `start_sha`, `head_sha`, `old_line`/`new_line`, `line_range` with `line_code = sha1(path)_old_new` | `commit_id = refs.head`, `path`, `side = RIGHT` when `line.new` exists else `LEFT`, `line = line.number()`, `start_line`/`start_side` from `start` |
| Drafts | `…/draft_notes` CRUD, per user, survive sessions | My pending review: `addPullRequestReview` opens it on the first draft; `addPullRequestReviewThread` (line drafts), `addPullRequestReviewThreadReply` (replies), the review body (drafts on the PR itself, appended); read back from the same `reviewThreads` query, where pending comments carry `state: PENDING`. Edits and deletes go by node id, looked up from the draft's database id. |
| Publish | `POST …/draft_notes/bulk_publish`, then `/approve` when asked | `submitPullRequestReview` with `COMMENT` or `APPROVE`, in one call |
| Resolve | `PUT …/discussions/:id resolved=` | GraphQL `resolveReviewThread` / `unresolveReviewThread` |
| Approve | `POST …/approve`, `…/unapprove` | `POST pulls/:n/reviews` with `APPROVE`; there is no unapprove for the reviewer, the error says to request changes or dismiss from the web |
| Review apps | `GET projects/:path/environments?states=available&search=<CI_COMMIT_REF_SLUG>` (the deployments list ignores `ref`), then each environment's `last_deployment`: kept when a success from the branch or `refs/merge-requests/:iid/`, current when from the MR's newest pipeline | `GET repos/…/deployments?ref=<branch>`, the first of each environment, then its newest status: kept when `success` with an `environment_url` |
| Draft or ready | GraphQL `mergeRequestSetDraft(projectPath, iid, draft)`; a no-op when already there | GraphQL `pullRequest { id isDraft }`, then `convertPullRequestToDraft` or `markPullRequestReadyForReview` by node id, skipped when already there |
| Suggestions | ```` ```suggestion:-0+0 ```` fence | ```` ```suggestion ```` fence over the commented lines; the range comes from `start`/`line` |
| Line URL | `web_url/diffs#sha1(path)_old_new` | `web_url/files#diff-sha256(path)R<new>` or `L<old>` |
| Sigil | `group/project!42` | `owner/repo#42` |

## Auth

Tokens stay in the keychain, one account per host (`04-security.md`).
Login borrows the token the forge's own CLI already holds:

| Forge | Seed | Scope |
|---|---|---|
| GitLab | `revu login --from-glab` reads `glab auth status --show-token` | `api` |
| GitHub | `revu login --from-gh` reads `gh auth token` | `repo` |

Without the CLI and without `--token -`, login stops with one line naming the missing tool and the token page to create one.
`GITLAB_TOKEN` is the env override for GitLab hosts, `GITHUB_TOKEN` (else `GH_TOKEN`) for GitHub hosts.
Without `--host`, a command inside a checkout talks to the host of its `origin` remote when a token is there for it (env or keychain), else to the configured host.
Every host remembers its own username (`[hosts."<host>"] username`), so the TUI names me right on each; with only a token variable, the queue answer names me.

## Several hosts in one queue (M5)

Every host the config remembers a login for (`host`, `[hosts.*]`) and holding a token joins the unscoped queue: outside a checkout, `--all`, or `*` in the TUI.
Each host answers its own queue with its own username, rows carry their host (`MrKey.host`, `None` for the host `revu` started with), and `Sections::merge` sorts each section newest first.
Opening a row, its drafts, its pipeline and its cache all go to that row's host; the `!` or `#` and the line links follow the row's forge (`Hosts::kind_of`).
Rows carry a short host tag (`gitlab`, `github`) only when more than one host is in the queue.
A checkout's queue keeps to the checkout's own host; a host that fails to answer is left out rather than failing the queue (`revu list` names it on stderr).

## What changed in phase 1 that a user can see

Nothing on screen or in plain output: `revu list`, `list --all`, `show`, `diff` and the TUI frames are byte-identical on the same data.
`--json` output speaks the neutral model: `number` instead of `iid`, `project` (the path) instead of `project_id`, no numeric `id`.
Cache paths moved from `mr/<project_id>/<iid>/` to `mr/<group+project>/<number>/`; old entries are ignored, so saved folds and viewed files of MRs opened before the upgrade start fresh.

## What differs on GitHub (phase 2)

- **Drafts on the PR itself** live in the pending review's body: several of them read back as one draft, their texts joined.
- **Resolve on publish** does not exist on GitHub: a draft's `resolve` flag is ignored.
- **Moving a draft** is impossible on GitHub: an edit changes the text, the comment stays on its line (verified live).
- **Unapprove** is refused with the way out; approving one's own PR is refused by GitHub, and the message says so.
- **Files GitHub will not diff** (no `patch` with changes counted) read as too large; a file with no patch and no changes counted reads as binary.
- **Deleted accounts** show as `ghost`, as on github.com.

## Verified live (2026-09-22, private sandbox `owner/repo#1` on each forge)

- `revu whoami`, scoped `revu list`, `revu show #1`, `revu diff #1`.
- TUI: open the PR, a draft on an added line (RIGHT 3) and on a removed line (LEFT 6), edit one after a restart (its line stays), publish as one `COMMENTED` review, a reply draft in a thread then published, resolve and unresolve.
- `revu comment #1 --at main.rs:5` lands on RIGHT 5; `revu comment #1 text` on the conversation; `revu approve` on my own PR and `--undo` fail with their messages.
- GitLab unchanged: the same commands against the GitLab sandbox, from a GitLab checkout and from a GitHub checkout with no GitHub token.
