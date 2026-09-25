# 06 · Roadmap and agent handoff

Milestones are ordered; each one is shippable and demoable on its own.
An agent picks the first milestone whose acceptance list is not green, works in a worktree, and opens one MR per slice under 500 changed lines.
TDD order inside each milestone is given; write the failing test first.

## How to work in this repo

1. Read `AGENTS.md`, then the spec files this milestone names.
2. `cargo build && cargo test` must be green before and after.
3. Run against the real thing with `revu login --from-glab` once, then `cargo run -- <cmd>`; never paste output that names a private project into the repo.
4. Snapshot changes (`cargo insta review`) are reviewed by eye: a snapshot is a design decision.
5. Every new key goes in the `?` help and in `README.md` in the same MR.

## M0 · Bootstrap (done 2026-09-22)

- `revu login`, `revu logout`, `revu whoami`, `revu tui` (empty shell), `revu completions`.
- Keychain store, config, API client with `GET /user`, theme, event loop.
- Acceptance: `cargo test` green; `revu whoami` prints the username with a token in the keychain or `GITLAB_TOKEN`.

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
9. `revu list` and `revu show <ref>` subcommands (plain and `--json`).

Acceptance:

- Open the TUI, see my three sections with badges matching the web UI.
- Open an MR from the queue in under 50 ms when cached.
- Read a 300-file MR without lag; fold and unfold; jump between changes and comments.
- Threads show under their line with author, age and body; outdated ones in the file tail.
- Every key in `03-ui-ux.md` marked M1 works and is in `?`.

## M2 · Write (done 2026-09-22)

Verified live: a draft created from the TUI lands on GitLab with its position, the modal counts it, `d` deletes it there. Publish, resolve, reply and approve are covered by wiremock and App tests only; the first real publish is the M3 acceptance run.

Spec: `02` (draft notes, positions, resolve, approve), `03` (comment flow, publish modal, drafts badge).

TDD order:

1. `review::position` table tests → builder, including ranges and `line_code`.
2. `api::rest` draft notes CRUD, bulk publish, resolve, approve with wiremock.
3. `tui::app`: `c` on a line opens the input, `enter` creates a `Draft` and returns `Action::SaveDraft`; `V` range; `r` reply; `R` resolve; `P` opens the publish modal; `A` approve.
4. Draft sync: a local draft gets its GitLab id on the first successful POST; a retry first lists `GET draft_notes` and re-posts only what is absent, so a network blip never duplicates a draft.
5. `E` compose in `$EDITOR` for long comments; `s` suggestion prefill.
6. Snapshots: line with a draft, publish modal, thread with a reply in flight.
7. `revu comment <ref> <path>:<line> <text>` and `revu approve <ref>` subcommands.

Acceptance:

- Write three comments on two files, see `3 drafts` in the status line, quit, reopen, the drafts are still there (from GitLab).
- `P` publishes them as one review, optionally approving; the web UI shows them under one activity entry.
- Resolve and unresolve a thread; reply in a thread; the web UI agrees within one poll.
- Nothing is posted publicly without `P`, an explicit `:reply`, or `⌘enter`/`ctrl-s` in the compose box.

## Shipped beyond M2 (2026-09-22 → 09-23)

Done outside the original milestones, recorded so nobody plans them again:

- **Forges:** GitLab and GitHub behind one seam (`07-forges.md`); `--from-gh`, `--from-glab`.
- **Queue:** scoped to the checkout's repo with an `OPEN` section, `*` or `--all` for every project; clickable `!iid` (OSC 8); description modal `i`.
- **Diff:** inline one-word changes (`D` toggled split, since replaced by side by side), tint from the terminal's own background (OSC 11, sign-only fallback), tree-sitter syntax highlighting for TS/TSX/JS/Python/JSON (replaces the `syntect` plan in M5).
- **Themes:** the nine palettes, each with its diff and syntax colours.
- **Tooling:** `revu update` (cargo install from the public repo), pedantic lints, CI (fmt, clippy, test), rename `gitlabmr`/`mr` → `revu` with a one-time move of config, cache and keychain (removed 2026-09-24).

## M3 · Polish

Spec: `03` in full.

Carried gaps, first:

- `?` help fits a 24-row terminal (scrolls).
- Publish modal: `enter` on the footer publishes; the footer reads `enter publish · e edit`.
- A publish the forge refuses names the draft whose line vanished and offers to turn it into an MR-level note.
- The header folds to one row.
- `zo` / `zc` in the queue act on the section under the cursor.

Then:

- `:` palette with completion and history, `ctrl-k` jump across the queue and the open MR's files.
- File tree pane `t`, viewed files `zv`, saved fold state.
- Zen `zz` (was reading mode `z`), wrap `w`, whitespace toggle `W`, expand context `+`.
- Live `●` markers, rate-limit backoff in the status line, empty states and the loading skeleton, `:set theme=`.

Acceptance: the designer test in `03-ui-ux.md` ("the screenshot test") passes on Ghostty and iTerm2 in a dark and a light theme.

## M3b · Reading and discussing (done 2026-09-23)

Two specs, built after M3 and before M4:

- `08-open-file.md`: open the file under the cursor, after the change, in the terminal editor or viewer of choice, per file type.
- `09-thread-pane.md`: threads and their comments live in a right pane, like slack-tui's thread, and replies are written there, not inside the diff.

Decisions taken while building them:

- `v` pauses revu's key reader while the program owns the terminal (its reader thread used to swallow keys typed into `$EDITOR` too).
- `enter` on a file row keeps folding it; `l` opens its outdated threads.
- `]n` also stops on the header row when the MR itself has an unresolved thread or my draft; `]N` when it has any.
- `E` in the compose box would type an E, so the box hands its text to `$EDITOR` with `ctrl-o`.
- After saving or leaving the box, the keys go back where it was opened from: the diff for `c`, the pane for `r`.
- `e` in the publish modal closes the modal and edits in the pane.

## M4 · AI (done 2026-09-23)

Spec: `05`.

Decision (2026-09-23): AI is configuration and its keys are secrets.

- `[ai]` in the config enables each provider on its own (`typesafe`, `anthropic`); both are off by default.
- Keys live in the macOS keychain under service `revu`, accounts `typesafe` and `anthropic`, set with `revu ai login <provider>` (hidden prompt or `--token -`).
- `TYPESAFE_API_KEY` and `ANTHROPIC_API_KEY` override the keychain; a key is never written to the config.

Work:

- Copy `typesafe.rs`, wire the four triage questions, badges in the queue and file tree.
- `anthropic.rs` streaming client, prompt caching, refusal handling.
- Context builder with its budget rules.
- `a` menu, answer pane, `c` to draft from an answer, cache.
- `revu ai login <provider>`, `[ai]` config, `:ai off`, `:ai on`.

Acceptance: `a s` on an open MR streams a summary in under two seconds to the first token; a second `a e` on the same file reports cache reads in the debug log.

## M5 · Extras (done 2026-09-23)

- Pipeline pane `p`: jobs by stage, failures first, `o` opens a job, refreshed every 15 s while it runs; GitLab's newest MR pipeline, GitHub's check runs grouped by workflow.
- Apply a suggestion with `S` in the thread pane, after a `y`: GitLab applies it by id; GitHub has no API, so revu commits it through the contents API on the PR's own branch when I can push there, else `o` opens it on the web.
- Notifications: one macOS notification per queue answer for MRs new to To review, each once; `[notify] enabled = false` turns them off.
- Several hosts in one queue: every logged-in host joins the unscoped queue, rows tagged with their host (`07-forges.md`).
- More syntax languages stay open: one grammar crate and one registry entry each (see `AGENTS.md`).
- Linux keychain via Secret Service: **not planned**. macOS only is a stated non-goal (`00-vision.md`); the keychain trick through `/usr/bin/security`, `open` and `pbcopy` are macOS tools, and nobody asked for Linux.

## Definition of done for any slice

- Tests green, `cargo clippy -- -D warnings` clean, `cargo fmt` clean.
- No comment in the diff that explains what the code does.
- README key table updated.
- The MR description says what the user can now do, in two sentences.
