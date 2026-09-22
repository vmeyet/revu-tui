# 01 · Architecture

Crate `revu`, binary `revu`, edition 2024, Rust 1.88+.
`cargo install --git <repo>` is the only install path, as for slack-tui.

## Crate layout

```
src/
  main.rs            parse Cli, dispatch, print errors as `✗ message` + dimmed causes
  lib.rs             pub mod list
  cli.rs             clap: login, logout, whoami, list, show, diff, comment, approve, tui, completions, update
  ctx.rs             Ctx { forge: Forge, config, cache, json, project } opened once per command
  config.rs          ~/.config/revu/config.toml
  cache.rs           ~/.cache/revu/<host>/…  json files, atomic writes
  version.rs         `revu --version` = crate version + git hash from build.rs
  update.rs          `revu update`, copied from slack-tui
  auth/
    mod.rs           SERVICE, Credentials { host, token }, resolve(env, store, config, host)
    store.rs         SecretStore trait, SecurityCli, MemoryStore   (copied from slack-tui)
    login.rs         prompt / --from-glab / --token -, verify with GET /user, store
  forge/             the seam, see 07-forges.md
    mod.rs           Kind (which forge a host runs), Forge (enum, one arm per backend), re-exports
    model.rs         the neutral model: MrKey, Mr, Refs, DiffFile, Discussion, Note, Position, LineRef, Draft, NewDraft
    queue.rs         Queue, QueueMr, Sections and the pure split into sections
    gitlab/
      mod.rs         Client: PRIVATE-TOKEN header, host guard, pagination, rate-limit backoff, line_url
      wire.rs        GitLab JSON (Mr, DiffRefs, Discussion, Note, DraftNote, Position, line_code) and its conversions
      graphql.rs     the queue query; typed answer turned into Queue
      rest.rs        mr, diffs, discussions, drafts, draft CRUD, publish, resolve, approve, comment
    github/          phase 2
  diff/
    mod.rs           parse(unified: &str) -> Vec<Hunk>; Line { kind, old, new, text }
    words.rs         intra-line word diff between a paired -/+ block (crate `similar`)
    fold.rs          FoldState per file and hunk, viewed files, "expand all" helpers
  review/
    mod.rs           Review { mr, files, threads, drafts, viewed, fold } — the pure model the TUI edits
    position.rs      builds a neutral Position from a selected line (or range) and the MR's refs
    thread.rs        Thread model: root note, replies, resolvable/resolved, anchored line
  render/            plain-terminal rendering for the scriptable commands (copied from slack-tui)
  commands/          one file per subcommand
  tui/
    mod.rs           run(): terminal setup, event loop, Action runner
    app/
      mod.rs         Focus, Input, Action, Incoming enums
      state.rs       App struct, Default
      keys.rs        handle_key -> Vec<Action>
      incoming.rs    apply(Incoming)
      queue.rs       queue rows, sections, badges
      review.rs      diff navigation, folds, selection, drafts in the App
      commands.rs    `:` verbs
      feedback.rs    Toast
      tests.rs       state machine tests
    ui.rs            draw(): layout and panes
    diff_view.rs     the diff pane renderer (rows from Review + FoldState)
    thread_view.rs   the right pane renderer
    theme.rs         copied from slack-tui, plus diff colours
    field.rs, palette.rs, complete.rs, jump.rs, compose.rs, motion.rs   copied from slack-tui
  ai/
    mod.rs           Assistant trait, Context builder, cache
    typesafe.rs      copied from slack-tui (typed judgments)
    anthropic.rs     Messages API, streaming, prompt caching
tests/
  cli.rs             assert_cmd
  snapshots/         insta
```

## Runtime

Tokio multi-thread runtime.
The TUI loop is the one from slack-tui `src/tui/mod.rs`:

```
loop {
  draw(app)
  wake = select! { terminal event, incoming channel, tick }
  match wake {
    Event(key)   => actions = app.handle_key(key)
    Incoming(i)  => app.apply(i)
    Tick         => app.now = Instant::now(); maybe poll
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
    pub state: String,
    pub draft: bool,
    pub author: User,
    pub source_branch: String,
    pub target_branch: String,
    pub web_url: String,
    pub updated_at: DateTime<Utc>,
    pub refs: Refs,              // base, start, head: what notes on lines are made against
    pub pipeline: Option<Pipeline>,   // status, web_url
    pub changes_count: Option<String>,
    pub conflicts: bool,
    pub reviewers: Vec<User>,
    pub labels: Vec<String>,
    pub approvals: Approvals,    // approved, approvals_left, user_has_approved, user_can_approve, approved_by
}

pub struct File {
    pub old_path: String,
    pub new_path: String,
    pub kind: FileKind,          // Added | Deleted | Renamed | Modified | Mode
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
ai/<group+project>/<number>/<head_sha>/<question_hash>.json
```

Files are written to a temp name then renamed, mode 0600, directory 0700.
Opening an MR paints from cache, then three requests refresh it in parallel; a differing `head_sha` invalidates diffs and clears word-diff and fold state for changed files only.

## Polling

- Queue: every 60 s while the TUI runs, one GraphQL call.
- Open MR: discussions every 30 s, MR every 60 s (pipeline, approvals, head sha).
- A change in the open MR shows `●` in the status line and in the file or thread it touched; it never moves the cursor.
- Backoff to 5 min after a `429` or a network error, reset after one success.

No websocket: GitLab has none for this. Polling at these rates stays far under the 2000 requests/min limit observed.

## Config

`~/.config/revu/config.toml`:

```toml
host = "gitlab.com"          # default host; `revu --host` and GITLAB_HOST override

[queue]
groups = ["acme"]            # limit the queue to these groups (optional)
projects = []                # or these projects (optional)
watch_labels = ["infra"]     # MRs with these labels appear in Watching

[review]
fold = ["*.lock", "*.snap", "**/generated/**"]
context = 3                  # context lines shown around a hunk; `+` expands

[tui]
theme = "tokyonight"
highlight = "none"
ascii = false                # true swaps glyphs (◆ ▸ ●) for ASCII

[ai]
enabled = false              # both providers off until this is true
provider = "anthropic"       # or "typesafe"; typesafe is triage only
model = "claude-opus-5"
```

Unknown keys fail loudly with the file path and key, as in slack-tui.

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
| reqwest 0.13 (rustls, json), no default features | HTTP, GraphQL |
| serde, serde_json (preserve_order), toml | data |
| clap 4 (derive, env), clap_complete | CLI |
| anyhow | errors |
| chrono | ages |
| dirs | paths |
| similar | word diff |
| unicode-width, textwrap | layout |
| owo-colors | plain terminal output |
| glob | fold patterns |
| futures-util | select on the event stream |

Dev: wiremock, insta, assert_cmd, predicates.

Not used: `keyring` (prompts on every rebuild, see `04-security.md`), `syntect` (v2 candidate, see roadmap M5), `git2` (no local checkout needed).
