# 04 · Security

The token is the only secret the app holds, plus optional AI keys.
Every rule here exists so that a `cargo install` of this tool is not scarier than `glab`.
`glab` keeps its token in plaintext at `~/Library/Application Support/glab-cli/config.yml`; this tool does better.

## Tokens live in the keychain

- Service `revu`, account `<host>` (for example `gitlab.com`), value the raw PAT.
- Written and read through `/usr/bin/security`, copied from slack-tui `src/auth/store.rs`:
  - Write: `security -i` with the command on stdin and the secret hex encoded (`-X`). The secret never appears in `argv` (visible in `ps`) and never reads from the tty.
  - Read: `find-generic-password -w`; exit code 44 means absent.
  - Delete: `delete-generic-password`, absent is success (idempotent logout).
- Why not the `keyring` crate: items created by the binary are ACL bound to that binary's signature, so every `cargo install` triggers a keychain prompt. Items created by Apple's signed `security` are not.
- One token per host. `revu login gitlab.example.com` adds a second account under the same service.

## Login flows

```
revu login [host]                prompts for the token, echo off, verifies with GET /user, stores
revu login --from-glab [host]    reads the token from `glab auth status --show-token`, same verify, same store; prints a one-line hint that glab still keeps its own copy
revu login --token - [host]      token on stdin, for scripts
revu logout [host]               deletes the keychain item, the cache dir for that host, the host entry in config
```

The prompt says which scope to create the token with (`api`) and links to `https://<host>/-/user_settings/personal_access_tokens?scopes=api`.
Verification runs before anything is written, so a typo never leaves a bad entry.
Expiry: `GET /personal_access_tokens/self` gives `expires_at`; the TUI status line warns 7 days before.

## Env override

`GITLAB_TOKEN` (and `GITLAB_HOST`) win over the keychain, for CI and scripts.
`revu whoami` says which source it used.

## The token goes to one host

`api::Client` is built with the host and refuses any request whose URL host differs, before adding the header.
Redirects are not followed (`reqwest::redirect::Policy::none()`), so a 302 to another host cannot carry the header.
TLS via rustls with webpki roots; no `danger_accept_invalid_certs` option exists.

## Never on disk, never in logs

- Config, cache, snapshots and fixtures hold no token. `Config` has no token field so it cannot be serialised by accident.
- Error chains are scrubbed in `api::Client` before leaving the module: the header value is replaced by `<redacted>`.
- `Debug` on `Credentials` prints `Credentials { host, token: "<redacted>" }`.
- No telemetry, no crash reporting, no update check beyond `git ls-remote` on the public repo (as slack-tui).
- Test fixtures use `glpat-XXXX`.

## Files

- `~/.cache/revu/` directory 0700, files 0600, atomic renames.
- Editor temp files for compose live in `$TMPDIR`, 0600, removed after the editor exits, even on cancel.
- The cache holds MR content, which is confidential to the project. `revu logout` and `revu cache clear` wipe it.

## AI keys and what leaves the machine

- Each provider is off until its own table switches it on: `[ai.anthropic] enabled = true`, `[ai.typesafe] enabled = true`.
- Keys are secrets, never config: the keychain holds them under service `revu`, accounts `anthropic` and `typesafe`, set with `revu ai login <provider>` (hidden prompt or `--token -`, never argv). `ANTHROPIC_API_KEY` and `TYPESAFE_API_KEY` override the keychain.
- `revu ai login typesafe` offers to copy the key slack-tui keeps (service `typesafe`); `revu ai login anthropic` checks the key against `GET /v1/models` before storing it.
- A config holding a key field fails to load: the `[ai.*]` tables only know `enabled` and `model`.
- What is sent to Claude: the MR title, description, file list, the selected file or hunk with context, the selected thread, and the question. Never a forge token, never other MRs.
- What is sent to Jev: for each queue MR its title, description, age, pipeline, labels and counts; for an opened MR each file's path and its first hunks. Never a forge token.
- The first AI call in a session shows a toast naming the provider and the model; `:ai off` stops it for the session.
- Answers are cached per `(project, iid, head_sha, question)` in the cache dir with the same permissions as MR content.

## Dependencies

- `cargo audit` in CI; `cargo deny` with a licence allowlist (MIT, Apache-2.0, BSD, ISC, MPL-2.0 for rustls).
- reqwest without default features: no native-tls, no cookies, no gzip beyond what is needed.
- The dependency list in `01-architecture.md` is the budget; adding one needs a line in the MR description saying why.

## Repository hygiene

The repo may become public like slack-tui.
Never commit: a host that is not `gitlab.com`, a real project id or path, a real MR title, a work email.
Fixtures are invented (`acme/widgets!42`).
The git identity for this repo is the noreply address, checked before every push (see slack-tui memory).
