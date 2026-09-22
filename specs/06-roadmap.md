# 06 · Roadmap and agent handoff

Milestones are ordered; each one is shippable and demoable on its own.
An agent picks the first milestone whose acceptance list is not green, works in a worktree, and opens one MR per slice under 500 changed lines.
TDD order inside each milestone is given; write the failing test first.

## How to work in this repo

1. Read `AGENTS.md`, then the spec files this milestone names.
2. `cargo build && cargo test` must be green before and after.
3. Run against the real thing with `mr login --from-glab` once, then `cargo run -- <cmd>`; never paste output that names a private project into the repo.
4. Snapshot changes (`cargo insta review`) are reviewed by eye: a snapshot is a design decision.
5. Every new key goes in the `?` help and in `README.md` in the same MR.

## M0 · Bootstrap (done 2026-09-22)

- `mr login`, `mr logout`, `mr whoami`, `mr tui` (empty shell), `mr completions`.
- Keychain store, config, API client with `GET /user`, theme, event loop.
- Acceptance: `cargo test` green; `mr whoami` prints the username with a token in the keychain or `GITLAB_TOKEN`.

## M1 · Read-only review (done 2026-09-22)

Known gaps carried to M3: the header does not fold, `zo`/`zc` in the queue always target the Done section, the default theme has no added/removed tint (RGB themes do).

Spec: `01`, `02` (queue, MR, diffs, discussions), `03` (layout, diff pane, folds, navigation).

TDD order:

1. `diff::parse` fixtures → parser.
2. `diff::words` pairs → word ranges.
3. `api::graphql` queue query with wiremock → `Queue` type and sections.
4. `api::rest` `mr`, `diffs` (two pages), `discussions` with wiremock.
5. `review::Review::from(mr, files, threads)`: anchors threads to lines, marks outdated.
6. `tui::app` tests: `j/k` in the queue, `enter` opens (returns `Action::Open`), `apply(Incoming::Review)`, `Tab`, `[c ]c`, `za zM zR`, `t`, `z`, `?`.
7. `tui::ui` snapshots: queue empty, queue loaded, MR open with one folded and one open file, thread pane open.
8. Cache: paint from cache then refresh; head sha change test.
9. `mr list` and `mr show <ref>` subcommands (plain and `--json`).

Acceptance:

- Open the TUI, see my three sections with badges matching the web UI.
- Open an MR from the queue in under 50 ms when cached.
- Read a 300-file MR without lag; fold and unfold; jump between changes and comments.
- Threads show under their line with author, age and body; outdated ones in the file tail.
- Every key in `03-ui-ux.md` marked M1 works and is in `?`.

## M2 · Write

Spec: `02` (draft notes, positions, resolve, approve), `03` (comment flow, publish modal, drafts badge).

TDD order:

1. `review::position` table tests → builder, including ranges and `line_code`.
2. `api::rest` draft notes CRUD, bulk publish, resolve, approve with wiremock.
3. `tui::app`: `c` on a line opens the input, `enter` creates a `Draft` and returns `Action::SaveDraft`; `V` range; `r` reply; `R` resolve; `P` opens the publish modal; `A` approve.
4. Draft sync: a local draft gets its GitLab id on the first successful POST; a retry first lists `GET draft_notes` and re-posts only what is absent, so a network blip never duplicates a draft.
5. `E` compose in `$EDITOR` for long comments; `s` suggestion prefill.
6. Snapshots: line with a draft, publish modal, thread with a reply in flight.
7. `mr comment <ref> <path>:<line> <text>` and `mr approve <ref>` subcommands.

Acceptance:

- Write three comments on two files, see `3 drafts` in the status line, quit, reopen, the drafts are still there (from GitLab).
- `P` publishes them as one review, optionally approving; the web UI shows them under one activity entry.
- Resolve and unresolve a thread; reply in a thread; the web UI agrees within one poll.
- Nothing is posted publicly without `P` or an explicit `:reply`.

## M3 · Polish

Spec: `03` in full.

- Themes (copy the nine), `:set theme=`, `highlight`.
- `:` palette with completion and history, `ctrl-k` jump across the queue and the file tree.
- File tree pane `t`, viewed files `zv`, saved fold state.
- Live polling with `●` markers, rate-limit backoff in the status line.
- Reading mode `z`, wrap `w`, whitespace toggle `W`, expand context `+`.
- Empty states and the loading skeleton.
- `mr update`.

Acceptance: the designer test in `03-ui-ux.md` ("the screenshot test") passes on Ghostty and iTerm2 in a dark and a light theme.

## M4 · AI

Spec: `05`.

- Copy `typesafe.rs`, wire the four triage questions, badges in the queue and file tree.
- `anthropic.rs` streaming client, prompt caching, refusal handling.
- Context builder with its budget rules.
- `a` menu, answer pane, `c` to draft from an answer, cache.
- `mr ai login anthropic`, `[ai]` config, `:ai off`.

Acceptance: `a s` on an open MR streams a summary in under two seconds to the first token; a second `a e` on the same file reports cache reads in the debug log.

## M5 · Extras (pick by value)

- Syntax highlighting with `syntect` and a small syntax set (measure startup first; lazy load).
- Pipeline pane: jobs, failed job names, `o` to open the job.
- Suggestion apply (`POST …/notes/:id/suggestions/apply`? check availability) from the thread pane.
- Notifications: a macOS notification when a To-review MR appears while the TUI runs.
- Second host in the queue at once.
- Linux keychain via Secret Service, if anyone asks.

## Definition of done for any slice

- Tests green, `cargo clippy -- -D warnings` clean, `cargo fmt` clean.
- No comment in the diff that explains what the code does.
- README key table updated.
- The MR description says what the user can now do, in two sentences.
