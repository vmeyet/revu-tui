# 00 · Vision

`revu` is merge request review in the terminal, as yourself: GitLab merge requests and GitHub pull requests (`07-forges.md`).
One Rust binary, ~10 ms startup, keyboard first, gorgeous.
The binary stays `revu` on every forge; the crate keeps its name, `revu`.

It is the sibling of `slack` (`~/Code/slack`, public as `vmeyet/slack-tui`).
Same shape: a TUI plus scriptable subcommands, same event loop, same keychain trick, same theme engine.
Where this spec says "as in slack-tui", the implementer copies the pattern (and often the file) from there.

## The loop it serves

1. See every MR that waits on me, and mine, in one queue.
2. Open one, read the diff fast: fold what I know, expand what I doubt.
3. Comment on a line, reply in a thread, resolve, approve.
4. Comments accumulate as drafts and publish in one go, like GitLab's "review" mode.
5. Ask an AI about a hunk, a file, a thread or the whole MR without leaving the screen.

The queue is the sidebar, an MR is a channel, a discussion is a thread.
Anyone who knows slack-tui knows this app after one `?`.

## Feasibility (assessed 2026-09-22)

Very feasible.
The hard parts of slack-tui are already solved and transfer unchanged:

| Piece | slack-tui file | Reuse |
|---|---|---|
| Keychain via `/usr/bin/security -i` (no ACL prompt on rebuild) | `src/auth/store.rs` | copy |
| Event loop: `Wake::{Event, Incoming}`, `App::handle_key -> Vec<Action>`, background tasks send `Incoming` | `src/tui/mod.rs`, `src/tui/app/*` | copy the shape |
| Themes (9 palettes, `mix()` for surfaces, `fade()` for unfocused panes) | `src/tui/theme.rs`, `src/tui/ui.rs` | copy |
| Input row with cursor on char boundaries | `src/tui/field.rs` | copy |
| `:` command palette with completion and history | `src/tui/palette.rs`, `src/tui/complete.rs` | copy |
| `ctrl-k` fuzzy jump | `src/tui/jump.rs`, `src/fuzzy.rs` | copy |
| `$EDITOR` compose | `src/tui/compose.rs` | copy |
| Spinner and empty-state motion | `src/tui/motion.rs` | copy |
| TypeSafe Jev typed judgments | `src/typesafe.rs` | copy |
| Config TOML, cache dir, `update` command, `build.rs` git hash | `src/config.rs`, `src/cache.rs`, `src/update.rs`, `build.rs` | copy |
| Tests: `wiremock` for HTTP, `insta` snapshots on `TestBackend`, `assert_cmd` for the CLI | `tests/`, `src/tui/app/tests.rs` | copy the setup |

New work, in order of size:

1. Diff rendering: parse unified diffs, fold files and hunks, intra-line word diff, comment anchors in the gutter (`03-ui-ux.md`).
2. GitLab client: queue via GraphQL, MR details, diffs, discussions, draft notes, approvals (`02-gitlab-api.md`).
3. Review state machine: drafts, publish, resolve, approve, viewed files (`01-architecture.md`).
4. Anthropic freeform assistant next to the Jev typed one (`05-ai.md`).

GitLab's API is friendlier than Slack's: it is documented, token based, and has a draft-notes endpoint that maps exactly onto "review mode".
Everything was probed live against gitlab.com 19.5 on 2026-09-22; shapes are recorded in `02-gitlab-api.md`.

Estimate for an agent team following `06-roadmap.md`: M1 (read-only review) in a few sessions, M2 (write) and M3 (polish) each about the same, M4 (AI) one session since the modules exist.

## Non-goals for v1

- Creating or editing MRs, editing descriptions, changing reviewers.
- Reading CI logs. Pipeline status and a link are enough.
- Anything but macOS. The keychain path is macOS specific, as in slack-tui.
- A web or GUI front. The terminal is the product.
- Offline writes. Reads come from cache instantly; writes need the network and say so.

## Principles

- **Keyboard grammar, not key soup.** Vim motions, `z` for folds, `[` `]` for jumps, `:` for commands, `?` lists everything. One verb per key, the same verb everywhere.
- **Instant, then fresh.** Every screen paints from cache in under 50 ms, then refreshes in the background. A stale view is marked, never blocked.
- **Nothing leaves silently.** Drafts stay drafts until `P`. AI is off until enabled. The status line always says what is pending.
- **Secure by construction.** Tokens live in the keychain and go to one host only. See `04-security.md`.
- **Gorgeous by default.** Typography, whitespace and colour are designed, not defaulted. See `03-ui-ux.md`.
- **Small and pure.** The `App` is a pure state machine; the network lives in `Action` handlers at the edge. That is what makes it testable and what made slack-tui pleasant to grow.

## Vocabulary

One word per concept, used in code, docs and UI:

| Word | Means |
|---|---|
| queue | The list of MRs on the left, grouped in sections |
| section | `To review`, `Mine`, `Watching` |
| MR | One merge request (a pull request on GitHub), addressed as `group/project!42` or `owner/repo#42`; in code an `MrKey { project, number }` |
| forge | The host's code platform: GitLab or GitHub |
| file | One changed file in the MR diff |
| hunk | One `@@` block inside a file |
| line | One diff row with an old and/or new number |
| thread | One discussion (resolvable or not) |
| note | One comment inside a thread |
| draft | A note written locally, or held by the forge (GitLab draft note, GitHub pending review comment), not yet published |
| review | The set of drafts, published together with `P` |
| viewed | A file the reviewer marked as read; it folds |
