# Architecture

`revu` is one binary: a TUI (`revu`) and scriptable commands (`revu list`, `revu show`, ...).
`src/lib.rs` holds everything and exports only `cli`, `commands`, `ctx` and `log` to `src/main.rs`, which parses the command line and calls a command.
This page says how the code is laid out today.
When a boundary or a flow changes, change this page in the same MR.

## Module map

| Module | What it holds |
|---|---|
| `main.rs`, `cli.rs` | The clap command line and the dispatch to `commands/` |
| `commands/` | One module per subcommand, each with a `run`; plain output through `render.rs`, JSON through `ctx::emit` |
| `ctx.rs` | `Ctx`: forge, credentials, config, cache and the checkout's project, opened once per command |
| `forge/mod.rs` | `Forge`, one enum over the backends; every method takes and returns the neutral model |
| `forge/model.rs` | The neutral model: `MrKey`, `ProjectPath`, `Mr`, `DiffFile`, `Discussion`, `Note`, `Draft`, `Sha`, ... |
| `forge/gitlab/`, `forge/github/` | One backend each: the client in `mod.rs`, calls and wire shapes in `rest.rs` and `graphql.rs`, shared shapes and conversion in `wire.rs` |
| `forge/gitlab/award.rs`, `upload.rs`, `forge/github/attachment.rs` | GitLab reactions (award emoji), pictures GitLab notes hold as uploads, pictures in GitHub comments |
| `forge/http.rs`, `http.rs` | The shared transport: the token goes to one host (and GitHub's web host for attachments), no redirects, TLS on rustls and ring |
| `forge/queue.rs`, `rules.rs`, `checks.rs`, `budget.rs`, `image.rs` | The queue and its sections, the "needs me" rules, CI runs, the rate limit, pictures in notes |
| `review/` | `Review`: the value the TUI edits; files, hunks, rows, folds, drafts, where threads hang |
| `diff/` | Unified diff parsing, word highlighting inside lines, fold state; `arbitrary.rs` builds diffs for property tests |
| `syntax/` | Syntax colours from tree-sitter grammars, loaded on first use |
| `outline/` | The changed functions and classes of an MR and the calls between them (`O`) |
| `act.rs` | Every action in one table: name, default keys, help, palette verb, panes, usage section |
| `keymap.rs` | The user's `[keys]` on top of the defaults |
| `tui/mod.rs` | The event loop: terminal events, answers and ticks in, frames out |
| `tui/app/` | `App`, the pure state machine: keys in, `Action`s out, `Incoming` answers applied |
| `tui/actions.rs` | The action runner: one task per `Action`, answers sent back as `Incoming` |
| `tui/backend.rs` | `Backend`: the parts behind the runner, one per concern, and `Homes`, each host's forge and cache |
| `tui/backend/saves.rs`, `queue.rs`, `reviews.rs` | What the reader keeps (`state.json`, pins, queue view); the queue of every host; opening an MR and loading ahead |
| `tui/backend/drafts.rs`, `tui/lanes.rs` | Draft writes and publishing; one lane per MR so they run one at a time |
| `tui/backend/ai.rs`, `files.rs`, `prune.rs` | Claude's answers and Jev's marks; files at a commit and pictures; the daily cache prune |
| `tui/ui.rs`, `tui/*_view.rs` | Drawing: layout in `ui.rs`, one view module per pane or modal |
| `tui/theme.rs`, `tui/screen.rs`, `tui/ground.rs` | The colour palette; the terminal in raw mode on the alternate screen; the terminal's background, asked once at startup |
| `tui/field.rs`, `complete.rs`, `palette.rs`, `help.rs` | The input box, tab completion, the `ctrl-k` palette, the `?` overlay |
| `tui/prose.rs`, `table.rs`, `images.rs` | Markdown files block by block, Markdown tables, pictures in the thread pane |
| `tui/compose.rs`, `clipboard.rs`, `drag.rs`, `motion.rs` | `$EDITOR` for a comment, the clipboard through OSC 52, text selection by drag, how often the screen moves on its own |
| `cache.rs` | JSON files under the cache folder; `cache::keys` spells the path of every entry |
| `config.rs`, `config/` | `config.toml`: one file per section group (`queue`, `tui`, `programs`), load, and `save` that rewrites only the changed keys; keys this revu does not read are warned, secrets refused (`ignored`) |
| `auth/` | Credentials and the keychain through `/usr/bin/security` |
| `ai/` | Claude (`anthropic.rs`, questions with `a`; `context.rs`, what a question sends) and Jev (`typesafe.rs`, `triage.rs`, marks on the queue) |
| `agent/` | Coding-agent sessions per MR (`a g`): find or start one, show it in tmux or a terminal split |
| `open.rs`, `program.rs`, `share.rs`, `ready.rs` | Programs the user configures: a viewer, a share target, a ready source |
| `changelog.rs`, `update.rs`, `version.rs` | `CHANGELOG.md` built into the binary, `revu update`, the version label |
| `docs.rs` | Writes `docs/reference/` from the code (`revu docs`) |
| `usage.rs`, `reactions.rs`, `log.rs` | Local usage counts, the reactions used most, the log file |
| `mrref.rs`, `query.rs`, `fuzzy.rs`, `render.rs` | Parsing `group/project!42`, the filter language, fzf-style matching, terminal output |

## Data flow

Reading an MR:

1. A backend in `forge/<backend>/` fetches the MR, its diffs, its discussions and the reader's drafts.
2. The backend turns the forge's JSON into the neutral model; nothing outside it sees the JSON.
3. `Reviews::build` makes a `Review` and adds what the cache remembers for this MR: folds, viewed files, side by side.
4. The runner sends `Incoming::Review`; `App::apply` puts it in the state, and the last cursor spot follows as `Incoming::Resume`.
5. `ui::draw` renders the state on each frame; heavy derived values are rebuilt only when their inputs change (`tui/app/kept.rs`).

Handling a key:

1. `tui/mod.rs` reads a terminal event and calls `App::handle_key`.
2. `keymap.rs` maps the user's keys to revu's own, then `act.rs` turns a key into an `Act` for the focused pane.
3. The pane's handler in `tui/app/` changes the state and returns a list of `Action`s.
4. The loop hands each `Action` to `actions::spawn`, which runs it in its own task through `Backend`; only `Action::Compose` runs in the loop, which hands the terminal to the editor.
   `Action::Copy` also writes the terminal's clipboard from the loop before its task runs `pbcopy`.
5. The answer comes back as an `Incoming`, applied like any other.

Commands: `main.rs` runs the ones that need no forge (`login`, `update`, `docs`, ...) at once.
The others get a `Ctx`, call `Forge` and print; `revu` and `revu tui` hand the `Ctx` to `tui::run`.

## Boundaries

- Only `src/forge/<backend>/` sees a GitLab or GitHub field; everything else uses `forge/model.rs`.
- Side effects live at the edges: `main.rs`, `commands/`, `tui/mod.rs` (the loop and the terminal), `tui/actions.rs` and `tui/backend/` (network, cache, processes).
- `tui/app/` is pure: it reads time from `app.now` and `app.today`, set by the loop, and never does I/O; its tests drive it with keys and `Incoming` values.
- The one exception is `App::new`, which reads the clock once to set `now` and `today` before the loop takes over.
- A program that takes the terminal (`$EDITOR`, the `^v` viewer, an agent session) runs from the loop through `Screen::lend`, with the key stream paused.
- Secrets are read only in `auth/`, `ai/` and the `login` and `ai` commands: `revu login` takes a token from `glab`, `gh` or stdin, and `revu ai` can copy slack-tui's key from the keychain.
- Secrets are sent only by `forge::http` (to the forge's API host, and its web host for GitHub attachments), the AI clients, and `revu ai`, which checks a new key against the Anthropic API before it stores it.

## Concurrency

- One tokio multi-thread runtime (`#[tokio::main]`).
- The loop is one task with a `tokio::select!` over terminal events, an unbounded `Incoming` channel and a tick; it never awaits the network.
- Each `Action` runs in its own `tokio::spawn` task.
- Blocking work goes to `spawn_blocking` through `blocking::run` (or `backend::run_blocking`, which hands the work its own copy of the backend part it needs): cache reads and writes, outline parsing, agent sessions, usage saves.
- Child programs that answer quickly use `tokio::process`; programs that own the terminal block the loop on purpose.

State shared between tasks, and why:

| What | Why |
|---|---|
| `Reviews::opening: Arc<AtomicUsize>` | Loading ahead waits while the reader opens an MR |
| `Saves::saved: Arc<Mutex<HashMap<MrKey, u64>>>` | The one lock every `state.json` write takes, the reader's saves and the opened stamp alike; saves finish out of order, and one older than the last is dropped |
| `Saves::changed: Arc<Mutex<HashMap<String, u64>>>` | The same for pins and the queue view |
| `Drafts::lanes: Lanes` | Draft writes and publishing of one MR run one at a time: on GitHub each reads the pending review, then writes it |
| `forge::budget::Budget` (atomics) | Every clone of a client shares the rate limit, so the loop shows what the tasks saw |
| GitLab `Client::me: Arc<OnceCell<String>>` | The reader's username is asked once and shared by every clone of the client |
| GitHub `Client::downloads: Arc<Downloads>` | Read only: the picture client is shared so the forge client stays cheap to clone |
| `TypeSafe::slots: Arc<Semaphore>` | At most 8 Jev requests run at once, across every task that asks |
| `log` subscriber: `Mutex<File>` | Every task writes the one log file |
| `OnceLock` in `syntax`, `outline`, `review`; `LazyLock` in `act`, `version` | Grammars, hunk colours, the default keys and the version label are built once, on first use |

## Caching

What the cache holds for users, and where, is in [`docs/concepts.md`](docs/concepts.md).
The internals:

- One folder per host under the platform cache folder (`dirs::cache_dir()/revu`, or `$REVU_CACHE_DIR`), one JSON file per key.
- Folders are 0700, files 0600, written to a temp name then renamed.
- A cache failure never stops the reader: `backend::read_cache` turns a stopped task into a logged miss, `backend::write_cache` logs a failed write, and an unreadable file is logged then removed.
- `Cache::shared()` is the root, for what belongs to no host: the changelog position, reactions, usage, the last share, the prune stamp, the log.
- Opening an MR paints the cached review at once, then fetches it again; when the head did not move, the diffs are not read again.
- Diffs are keyed by head sha; a file at a sha is cached forever.
- Each MR keeps `state.json`: folds, viewed files, the cursor spot, the side-by-side setting.
- Claude's answers are keyed by MR and the whole request: model, context and question.
- MRs waiting on the reader load ahead, two at a time, so `enter` paints at once.
- Once a day the cache drops MRs the forge says are merged or closed, and any MR untouched for 30 days.

## Where to add things

- **A key**: an `Act` variant and its row in `act.rs` `ROWS`; handle it in the pane's handler in `tui/app/` (`handle_queue_key`, `handle_review_key`, `handle_side_key`); run `cargo run -- docs`.
- **An action with a side effect**: an `Action` variant in `tui/app/mod.rs`, its arm in `tui/actions.rs`, the work in the `Backend` part it concerns, and an `Incoming` variant applied in `tui/app/incoming.rs`.
- **A forge field**: the field in `forge/model.rs`, filled by both backends in `forge/gitlab/` and `forge/github/`.
- **A command**: a variant in `cli.rs`, a module in `commands/` with `run`, an arm in `main.rs`; run `cargo run -- docs`.
- **A config key**: the field in `config.rs` or `config/` and a row in `docs::SETTINGS`; a test fails without it.
- **A syntax language**: see `AGENTS.md`.

## Glossary

One word per concept, in code, docs and UI.
The words users see are also in [`docs/concepts.md`](docs/concepts.md).

| Word | Means |
|---|---|
| queue | The list of MRs on the left, grouped in sections |
| section | One group of the queue: `PINNED`, `TO REVIEW`, `MINE`, ...; `forge::Sections` lists them |
| MR | One merge request (a pull request on GitHub), `group/project!42` or `owner/repo#42`; in code an `MrKey` |
| forge | The host's code platform: GitLab or GitHub |
| file | One changed file in the MR diff |
| hunk | One `@@` block inside a file |
| symbol | A function, method or class the outline lists |
| prose | A Markdown file drawn rendered, block by block |
| line | One diff row with an old and/or new number |
| thread | One discussion, resolvable or not |
| note | One comment inside a thread |
| draft | A note not yet published, written locally or held by the forge |
| review | The set of drafts, published together with `P` |
| viewed | A file the reviewer marked as read; it folds |
| pin | An MR kept on top of the queue, in `PINNED`, until it is merged or closed |
