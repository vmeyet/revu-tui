# Working in this repo

`revu` is a Rust TUI and CLI for reviewing GitLab merge requests.
The design lives in `specs/`; read `specs/06-roadmap.md` first, it says what to build next and which spec files to open.

## Ground rules

- The specs are the contract. When code and spec disagree, fix one of them in the same MR and say which.
- Copy from `~/Code/slack` (public as `vmeyet/slack-tui`) whenever `specs/00-vision.md` says a piece transfers. Keep its tests.
- One MR per roadmap slice, under 500 changed lines excluding fixtures and snapshots.
- Branches: `<scope>/<kebab-title>` with scope in `feat fix chore refactor docs test ci`; worktrees under `.claude/worktrees/<scope>+<kebab-title>`.
- Commits follow conventional commits; body in two short imperative sentences at most.

## Code

- `docs/` is for users, `specs/` for contributors; `revu docs` writes `docs/reference/` from the code, and a test fails when it is stale.
- Rust 2024, `rustfmt.toml` as committed (140 columns, `use_small_heuristics = "Max"`).
- Lints are pedantic (`[lints]` in Cargo.toml): `cargo clippy --all-targets -- -D warnings` must be clean, and `unwrap`/`expect` only live in tests.
- No comments that say what the code does; a comment must say why, and only when no name can.
- Immutability across function boundaries: never hand a mutable value to a sibling or child; side effects at the edges (`main.rs`, `commands/`, `tui/mod.rs` action runner).
- Flat bodies of named steps at one level of abstraction; signatures designed from the call site.
- One word per concept, the words in `specs/00-vision.md` § Vocabulary.
- Anything that can run twice (draft sync, cache writes, logout) is safe to run twice.
- Only `src/forge/<backend>/` sees a GitLab or GitHub field; everything else speaks the neutral model in `src/forge/model.rs` (`specs/07-forges.md`).

## Adding a syntax language

1. Add the grammar crate to `Cargo.toml` (`tree-sitter-<lang>`, a version that builds with the `tree-sitter` already there).
2. Add one entry to `LANGUAGES` in `src/syntax/mod.rs`: name, extensions, and a function building its `HighlightConfiguration` from the crate's highlight query.
3. If its query uses capture names `CAPTURES` does not list, map them to a `Token` there.
4. Add a line to the tests in `src/syntax/mod.rs` showing a keyword, a string and a comment landing on the right bytes.

Grammars load on first use, so a new language costs nothing until a file of it is opened.

## Tests

```sh
cargo test                      # unit, wiremock, snapshots, CLI
cargo test -- --ignored         # the real keychain round trip, touches the login keychain
cargo insta review              # after a deliberate rendering change
cargo run -- login --from-glab && cargo run -- whoami   # against gitlab.com with the token glab holds
```

Every module ships its tests in the same file; snapshot tests render on `TestBackend`.
Fixtures are invented: `acme/widgets!42`, user `nina`, token `glpat-xxxx`.

## Security

`specs/04-security.md` is not optional.
No token in config, cache, logs, fixtures or `Debug` output; the token reaches one host; keychain through `/usr/bin/security -i`.
The repo may go public: no real host other than `gitlab.com`, no real project path, id or MR title, no work email in commits.
