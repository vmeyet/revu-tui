# 01 · Architecture

Crate `revu`, binary `revu`, edition 2024, Rust 1.90+.
`cargo install --git <repo>` is the only install path, as for slack-tui.

## Crate layout

```
src/
  main.rs            parse Cli, dispatch, print errors as `✗ message` + dimmed causes
  lib.rs             the library behind the binary
  cli.rs             clap: login, logout, whoami, list, show, diff, comment, approve, merge, ready, publish, share, ai, tui, completions, update, usage, docs
  ctx.rs             Ctx { forge: Forge, config, cache, json, project } opened once per command
  config.rs          ~/.config/revu/config.toml
  cache.rs           ~/.cache/revu/<host>/…  json files, atomic writes
  keymap.rs          `[keys]`: the AZERTY preset and `[keys.bind]` aliases
  mrref.rs           `group/project!42`, `owner/repo#42`, `42` on the command line
  query.rs           the filter language the palette, the queue filter and saved views share
  fuzzy.rs           fzf-style subsequence matching
  open.rs, program.rs   `^v` and user commands, run without a shell
  ready.rs, share.rs    the ready source and `Y`, both through a user command
  render.rs          plain-terminal output for the scriptable commands (copied from slack-tui)
  docs.rs            `revu docs`: docs/reference/ written from the code
  usage.rs           `[usage]`: what revu is used for, counted on this machine only
  version.rs         `revu --version` = crate version + commit from build.rs
  update.rs          `revu update`, copied from slack-tui
  syntax/mod.rs      tree-sitter grammars, loaded on first use
  auth/
    mod.rs           SERVICE, Credentials { host, token }, resolve(env, store, config, host)
    store.rs         SecretStore trait, SecurityCli, MemoryStore   (copied from slack-tui)
  forge/             the seam, see 07-forges.md
    mod.rs           Kind (which forge a host runs), Forge (enum, one arm per backend), re-exports
    model.rs         the neutral model: MrKey, Mr, MrState, PipelineStatus, Refs, Sha, DiffFile, FileKind, Discussion, Note, Position, LineRef, Draft, NewDraft
    queue.rs         Queue, QueueMr, Sections and the pure split into sections
    rules.rs         which queue MRs need me now, and why the others do not
    checks.rs        the CI run of the head commit, jobs by stage
    budget.rs        rate-limit budget and backoff state
    image.rs         fetching pictures notes point at, capped
    http.rs          Transport both clients share: origin guard, pagination, rate-limit backoff, HttpError
    gitlab/
      mod.rs         Client: PRIVATE-TOKEN header, the GitLab Flavor, line_url
      wire.rs        GitLab JSON (Mr, DiffRefs, Discussion, Note, DraftNote, Position, line_code) and its conversions
      graphql.rs     the queue query; typed answer turned into Queue
      rest.rs        mr, diffs, discussions, drafts, draft CRUD, publish, resolve, approve, comment, merge
      award.rs       reactions (award emoji)
      upload.rs      pictures uploaded to a project
    github/
      mod.rs         Client: bearer token, the GitHub Flavor, line_url
      wire.rs        GitHub REST and GraphQL shapes and their conversions
      graphql.rs     the queue, one PR, its threads, the pending review
      rest.rs        who I am, changed files, comments, approvals, the contents API
      attachment.rs  pictures in comments
  diff/
    mod.rs           parse(unified: &str) -> Vec<Hunk>; Line { kind, old, new, text }
    words.rs         intra-line word diff between a paired -/+ block (crate `similar`)
    fold.rs          FoldState per file and hunk, viewed files, "expand all" helpers
    arbitrary.rs     generated diffs for property tests
  review/
    mod.rs           Review { mr, files, threads, drafts, viewed, fold } — the pure model the TUI edits
    position.rs      builds a neutral Position from a selected line (or range) and the MR's refs
    thread.rs        Thread model: root note, replies, resolvable/resolved, anchored line
    place.rs         anchor marks per line, and what the right pane lists
    draft.rs, suggestion.rs, image.rs, tree.rs   drafts, suggestion blocks, pictures, the file tree
  commands/          one file per subcommand
  tui/
    mod.rs           run(): terminal setup, event loop
    actions.rs       the Action runner: each action in its own task, answering through Incoming
    backend.rs       Backend: the forge, the cache and the AI behind the actions
    app/             the pure state machine: keys in, actions out, incoming answers applied
      mod.rs         Focus, Input, Action, Incoming enums
      state.rs       App struct, Default
      keys.rs        handle_key -> Vec<Action>
      incoming.rs    apply(Incoming)
      test_support.rs   what the app tests share: the fixture app, keys to press, the drawn screen
      tests.rs       whole-screen snapshots and rendering checks; every other test sits in the module it exercises
      …              one module per feature: queue, order, stack, review, pane, write, search, zen, brief, pipeline, ask, …
    ui.rs            draw(): layout and panes
    *_view.rs        one renderer per pane or modal: queue, diff, thread, tree, pipeline, answer, brief, publish, share, palette
    theme.rs         copied from slack-tui, plus diff colours
    screen.rs, ground.rs   the terminal as revu holds it, and its background colour
    field.rs, palette.rs, complete.rs, compose.rs   copied from slack-tui
    help.rs, table.rs, images.rs, drag.rs   the `?` overlay, markdown tables, pictures, mouse selection
  ai/
    mod.rs           which provider is on, and where its key comes from
    context.rs       what Claude reads for a question, in cached blocks
    triage.rs        Jev's questions about queue MRs and files
    typesafe.rs      copied from slack-tui (typed judgments)
    anthropic.rs     Messages API, streaming, prompt caching
tests/
  cli.rs             assert_cmd
  docs.rs            docs/reference/ is fresh, links resolve, the prose keeps the house style
```

## Runtime

Tokio multi-thread runtime.
The TUI loop is the one from slack-tui `src/tui/mod.rs`:

```
loop {
  draw(app); print links only when the frame changed
  wake = select! { terminal event, incoming channel, tick }   // tick: 100 ms after a changed frame, 1 s after a still one
  match wake {
    Event(key)   => actions = app.handle_key(key)
    Incoming(i)  => app.apply(i)
    Tick         => maybe poll                                // every wake sets app.now
  }
  for action in actions { spawn(run(action, tx.clone())) }   // except Compose, run inline
}
```

Rules, copied from slack-tui and kept:

- `App` never touches the network or the clock. `handle_key` returns `Vec<Action>`; `apply(Incoming)` mutates.
- Every `Action` runs in its own task and reports through `Incoming`, including `Incoming::Failed(action, message)`.
- `Action::Compose` (opens `$EDITOR`) runs on the loop thread because it owns the terminal.
- `app.now` is set by the loop; rendering reads it for spinners and ages.

## Data model

```rust
pub struct Mr {                  // forge/model.rs; see 07-forges.md for the rest of the neutral model
    pub project: String,         // "group/project" or "owner/repo"
    pub number: u64,             // GitLab iid, GitHub PR number
    pub title: String,
    pub description: String,     // markdown
    pub state: MrState,          // Open | Merged | Closed
    pub draft: bool,
    pub author: User,
    pub source_branch: String,
    pub target_branch: String,
    pub web_url: String,
    pub updated_at: DateTime<Utc>,
    pub refs: Refs,              // base, start, head (Sha): what notes on lines are made against
    pub pipeline: Option<Pipeline>,   // status: PipelineStatus, web_url
    pub changes_count: Option<String>,
    pub conflicts: bool,
    pub reviewers: Vec<User>,
    pub labels: Vec<String>,
    pub approvals: Approvals,    // approved, approvals_left, user_has_approved, user_can_approve, approved_by
}

pub struct File {
    pub old_path: String,
    pub new_path: String,
    pub kind: FileKind,          // Added | Deleted | Renamed | Modified | Mode, the DiffFile's change
    pub binary: bool,
    pub too_large: bool,         // GitLab `too_large` or > 2000 lines: folded by default
    pub hunks: Vec<Hunk>,
}

pub struct Hunk {
    pub header: String,          // "@@ -1,5 +1,7 @@ fn main()"
    pub old_start: u32,
    pub new_start: u32,
    pub lines: Vec<Line>,
}

pub struct Line {
    pub kind: LineKind,          // Context | Added | Removed
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub text: String,
    pub words: Vec<Range<usize>>,  // changed byte ranges, from words.rs, empty for context
}

pub struct Thread {
    pub id: String,
    pub resolvable: bool,
    pub resolved: bool,
    pub anchor: Option<Anchor>,  // file path + old/new line, None for MR-level threads
    pub notes: Vec<Note>,
}

pub struct Draft {
    pub id: Option<u64>,         // Some once GitLab holds it as a draft note
    pub local_id: Option<u64>,   // Some on a draft written this session: the save's answer finds it by this
    pub anchor: Option<Anchor>,
    pub reply_to: Option<String>,  // thread id
    pub body: String,
}

pub struct Review {
    pub mr: Mr,
    pub files: Vec<File>,
    pub threads: Vec<Thread>,
    pub drafts: Vec<Draft>,
    pub viewed: BTreeSet<String>,  // new_path
    pub fold: FoldState,
}
```

`Review` is the value the TUI edits and the value the tests build.
It is immutable across function boundaries: helpers take `&Review` and return a new piece, the `App` swaps it in.

## Diff parsing

GitLab returns each file as one unified-diff string without the `---`/`+++` header.
`diff::parse` reads `@@ -a,b +c,d @@ rest` headers and `+`, `-`, ` ` prefixes, and keeps `\ No newline at end of file` as a flag on the previous line.
Nothing else is in the grammar.
Unit tests cover: empty diff, added file, deleted file, hunk without counts (`@@ -1 +1 @@`), CRLF, trailing no-newline marker, tabs.

Intra-line highlighting: within a hunk, a run of `-` lines followed by a run of `+` lines of equal length pairs up line by line, and `similar::TextDiff::from_words` marks the changed byte ranges on both.
Unequal runs get no word ranges.
This is the rule GitHub and delta use; it is cheap and looks right.

## Folds

```rust
pub struct FoldState {
    pub files: BTreeMap<String, Fold>,        // key new_path; missing = Open
    pub hunks: BTreeMap<String, BTreeMap<usize, Fold>>,   // path, then hunk index: JSON has no tuple keys
}
pub enum Fold { Open, Closed }
```

Defaults on open: everything open except `too_large`, `binary`, `viewed` and files under a configured `fold` glob list (`*.lock`, `*.snap`, generated paths).
The fold state is saved in the cache per MR and head sha, so reopening an MR restores it.

## Cache

`dirs::cache_dir()/revu/<host>/` (`~/Library/Caches/revu` on macOS, `~/.cache/revu` elsewhere; `REVU_CACHE_DIR` overrides):

```
queue.json                          last queue answer + fetched_at, every project
queue.<group+project>.json          the same, scoped to one project
mr/<group+project>/<number>/mr.json
mr/<group+project>/<number>/diffs.<head_sha>.json
mr/<group+project>/<number>/discussions.json
mr/<group+project>/<number>/drafts.json
mr/<group+project>/<number>/state.json    viewed files, folds, last opened
mr/<group+project>/<number>/ai/answer.<request_hash>.json   Claude's answer, its title, time, head, request
pruned.json                         (shared root) when the cache was last pruned
```

Files are written to a temp name then renamed, mode 0600, directory 0700.
Once a day, starting the TUI prunes every host's cache in the background: the forge is asked once per project which kept MRs are merged or closed (GitLab `merge_requests?iids[]=…&state=all`, GitHub one GraphQL query with a `pullRequest(number:)` alias per PR), and their `mr/…/<number>/` folders go; so does any MR folder untouched for 30 days, which also covers a project the forge cannot answer for. Pruning twice is harmless.
Opening an MR paints from cache, then three requests refresh it in parallel; a differing `head_sha` invalidates diffs and clears word-diff and fold state for changed files only.

## Polling

- Queue: every 60 s while the TUI runs, one GraphQL call.
- Open MR: discussions every 30 s, MR every 60 s (pipeline, approvals, head sha).
  While the head sha is the one shown, that poll reads the MR alone; once it moved, diffs (from the cache when loading ahead kept them), discussions and drafts follow and the review is built again.
- A change in the open MR shows in the status line (`● 2 new notes`, `● new commits`) until the next key; it never moves the cursor. Queue rows with activity since they were last opened carry a `●` that pulses once a second.
- Both forge clients keep the rate-limit count their answers report (`forge::budget`); under 100 requests left, every poll interval is five times longer, and a request waiting out a 429 shows `⏳ 42s` in the status line.
- Backoff to 5 min after a `429` or a network error, reset after one success.
- A write the reader makes succeeds (approve, merge, publish, resolve, post, draft or ready): its effect shows at once from what revu knows (my approval counts in `approved_by` and `approvals_left`), then the MR and the queue are read again right away, so the forge's own numbers replace the guess within a request.

No websocket: GitLab has none for this. Polling at these rates stays far under the 2000 requests/min limit observed.

## Config

`~/.config/revu/config.toml`:

```toml
host = "gitlab.com"          # default host; `revu --host` and GITLAB_HOST override

[queue]
watch_labels = ["infra"]     # MRs with these labels appear in Watching

[review]
fold = ["*.lock", "*.snap", "**/generated/**"]
context = 3                  # context lines shown around a hunk; `+` expands

[tui]
theme = "tokyonight"
highlight = "none"
ascii = false                # true swaps glyphs (◆ ▸ ●) for ASCII

[ai.anthropic]
enabled = false              # questions about the MR; off until true
model = "claude-opus-5"

[ai.typesafe]
enabled = false              # Jev triage of the queue and files; off until true
```

Unknown keys fail loudly with the file path and key, as in slack-tui.
Saving (login, logout, `:set theme=`) rewrites only the keys that changed, comments and layout kept, through a temp file and a rename.

## Errors

`anyhow` at the edges, typed errors where a caller branches (`api::Error::{Unauthorized, NotFound, RateLimited(retry_after), Http(status, message), Network}`).
Error text never contains the token; `api::Client` strips the `PRIVATE-TOKEN` header from reqwest error chains before they leave the module.

## Testing

- `diff::parse` and `words`: unit tests with fixtures under `src/diff/fixtures/`.
- `api`: `wiremock` for every endpoint, including pagination and 429.
- `review::position`: table tests, one row per case (added line, removed line, context line, range across a hunk boundary rejected).
- `tui::app`: state machine tests, keys in, `Vec<Action>` and state out, no terminal.
- `tui::ui`: `insta` snapshots on `ratatui::backend::TestBackend` for the three main screens and the empty states.
- `tests/cli.rs`: `assert_cmd` for `--help`, `whoami` without a token, `login --token -`.
- The keychain test is `#[ignore]`, as in slack-tui.

## Dependencies

Pinned to what slack-tui compiles with today, so versions are known good together:

| Crate | Why |
|---|---|
| ratatui 0.30, crossterm 0.29 (event-stream) | TUI |
| tokio 1 (rt-multi-thread, macros, process, time, fs, sync, io-util) | runtime |
| reqwest 0.13 (rustls on ring, json), no default features | HTTP, GraphQL |
| serde, serde_json (preserve_order), toml | data |
| toml_edit | saving the config keeps the user's comments |
| clap 4 (derive, env), clap_complete | CLI |
| anyhow | errors |
| chrono | ages |
| dirs | paths |
| similar | word diff |
| unicode-width, textwrap | layout |
| owo-colors | plain terminal output |
| glob | fold patterns |
| futures-util | select on the event stream |
| tracing, tracing-subscriber (fmt) | the log file, `REVU_LOG` |

Dev: wiremock, insta, assert_cmd, predicates.

Not used: `keyring` (prompts on every rebuild, see `04-security.md`), `syntect` (v2 candidate, see roadmap M5), `git2` (no local checkout needed).
