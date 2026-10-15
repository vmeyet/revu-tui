# Working in this repo

`revu` is a Rust TUI and CLI for reviewing GitLab merge requests and GitHub pull requests.
Read [`ARCHITECTURE.md`](ARCHITECTURE.md) first: the module map, the data flow, the boundaries and where to add things.
The code is the contract; `ARCHITECTURE.md` describes it.
When an MR changes a boundary or a flow, it updates `ARCHITECTURE.md` too.
`archive/specs/` holds the old design notes: history, not kept up to date.

## Ground rules

- One MR per change a reviewer can read in one sitting, under 500 changed lines excluding fixtures and snapshots.
- Branches: `<scope>/<kebab-title>` with scope in `feat fix chore refactor docs test ci`; worktrees under `.claude/worktrees/<scope>+<kebab-title>`.
- Commits follow conventional commits; body in two short imperative sentences at most.
- `docs/` is for users; `revu docs` writes `docs/reference/` from the code, and a test fails when it is stale.
- Markdown files put one sentence per line.

## Code

- Rust 2024, `rustfmt.toml` as committed (140 columns, `use_small_heuristics = "Max"`).
- Lints live in `[lints]` in `Cargo.toml`: `cargo clippy --all-targets -- -D warnings` must be clean, and `unwrap`/`expect` only live in tests.
- Doc comments (`///`, `//!`) say what an item is for or why it is so; they describe the item they sit on and stay true when it changes.
- No comment restates the code; a `//` comment says why, and only when no name can.
- Immutability across function boundaries: pass helpers values or `&` borrows and take back a new value; only the owner of a state changes it, through `&mut self`.
- Side effects at the edges only, as `ARCHITECTURE.md` § Boundaries lists them; `tui/app/` stays pure.
- Flat bodies of named steps at one level of abstraction.
- Design a signature from the call site: a caller never needs to read the body to use it right.
- One word per concept: the glossary in `ARCHITECTURE.md`.
- Anything that can run twice (draft sync, cache writes, logout) is safe to run twice.
- Only `src/forge/<backend>/` sees a GitLab or GitHub field; everything else uses `src/forge/model.rs`.

## Rust design rules

Some code predates these rules: fix what you touch, and do not copy it.

- Domain values get newtypes, not bare `String` or `u64` (ids, project paths, URLs, shas, usernames): the compiler then stops a mix-up.
- Parse into those types at the forge boundary and keep the invariant inside the type: code above it never checks again.
- Visibility is the smallest that compiles: private by default, `pub(super)` or `pub(crate)` only where used; the library serves only `main.rs` and `tests/`, so `pub` is for what they call.
- Never drop a `Result` silently: `let _ =` or `.ok()` on a fallible call needs a reason a reader can see, and best-effort work logs the failure with `tracing::warn!`.
- `anyhow` at the edges; a typed error (`thiserror` or an enum) where a caller branches on the kind: a string is not a contract.
- No `bool` parameter on a function with more than one caller: use a two-variant enum or two functions, so the call site reads.
- No tuple or tuple alias for data that travels between modules: a struct with named fields says what each part is.
- A name says what the symbol does without reading its body: no one-word function names like `of`, `off`, `same`, `ahead`, unless the type makes them obvious (`from_x` constructors are fine).
- No shared mutable state across tasks when message passing works: the loop and its channel are the model.
- State shared between tasks (`Arc<Mutex<_>>`, atomics) says in a one-line doc comment on its field why it is shared.
- Prefer owned values passed by move, or `&` borrows, over clones; a `.clone()` of a large value on a hot path needs a reason.
- Never block the runtime: `std::process`, `std::fs` and heavy CPU work go through `spawn_blocking` or their tokio form.
- Never hold a `std::sync::Mutex` guard across an `.await`: take what you need and drop the guard.
- Tests are deterministic: no sleep to make tasks overlap (use a barrier, `Notify` or a channel), no real network, no real clock.
- One behaviour per test, named as a sentence that says it.
- `unsafe` only inside a safe function whose runtime checks make its `SAFETY:` comment true, not just a claim.
- One crate per job: prefer std or a dependency already in `Cargo.toml`; a new one needs a line in the MR saying why.
- A file over about 800 lines, or a type with more than one reason to change, gets split; a god struct is split by concern.

## Releases

- `CHANGELOG.md` is for users and ships in the binary (`revu news`, `:news`, the end of `revu update`).
- An MR with a `feat` or `fix` a user can notice adds its entry under `## Unreleased` in the same MR.
- Entries sit under `### New`, `### Fixed` or `### Removed`, in that order.
- An entry is three lines, one sentence each: what it is in bold, where it applies, how to use it; a test parses the page and fails on anything else.
- Bumping the version in `Cargo.toml` renames `## Unreleased` to `## <version> - <date>` and starts an empty `## Unreleased` above it; a test fails when a bump leaves entries behind.

## Adding a syntax language

1. Add the grammar crate to `Cargo.toml` (`tree-sitter-<lang>`, a version that builds with the `tree-sitter` already there).
2. Add one entry to `LANGUAGES` in `src/syntax/mod.rs`: name, extensions, and a function building its `HighlightConfiguration` from the crate's highlight query.
3. If its query uses capture names `CAPTURES` does not list, map them to a `Token` there.
4. Add a line to the tests in `src/syntax/mod.rs` showing a keyword, a string and a comment landing on the right bytes.

Grammars load on first use, so a new language costs nothing until a file of it is opened.

## Tests

```sh
cargo test                      # unit, wiremock, snapshots, CLI, docs staleness
cargo test -- --ignored         # the real keychain round trip, touches the login keychain
cargo insta review              # after a deliberate rendering change
cargo run -- docs               # rewrite docs/reference/ after changing keys, commands or config
cargo run -- login --from-glab && cargo run -- whoami   # against gitlab.com with the token glab holds
```

Every module ships its tests in the same file; snapshot tests render on `TestBackend`.
Fixtures are invented: `acme/widgets!42`, user `nina`, token `glpat-xxxx`.

## Security

- The token lives in the keychain (service `revu`, account `<host>`), written and read through `/usr/bin/security -i`, never through `argv`.
- `GITLAB_TOKEN`, `GITHUB_TOKEN` (or `GH_TOKEN`) and `GITLAB_HOST` override the keychain, for CI and scripts.
- The token reaches one host: `forge::http::Transport` refuses any other scheme, host or port, and follows no redirect.
- No token in config, cache, logs, fixtures or `Debug` output; `Credentials` prints `<redacted>`.
- AI keys follow the same rules: keychain or environment only, never the config.
- The cache holds MR content, so it is private to the user (`ARCHITECTURE.md` § Caching); `revu logout` wipes it.
- No telemetry and no crash reporting.
- The repo may go public: no real host other than `gitlab.com` and `github.com`, no real project path, id or MR title, no work email in commits.
