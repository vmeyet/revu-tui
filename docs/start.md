# Start here

Install revu, log in, and publish your first review in five minutes.

## 1. Install

```sh
cargo install --git https://github.com/vmeyet/revu-tui
revu --version
```

You need macOS and Rust 1.90 or newer.

## 2. Log in

Pick the line for your forge.

```sh
revu login --from-glab            # GitLab, with the token glab already holds
revu login github.com --from-gh   # GitHub, with the token gh already holds
revu whoami
```

The token goes to the macOS keychain, never to a file.
Without `glab` or `gh`, run `revu login` and paste a token.

## 3. Open the queue

```sh
cd ~/code/widgets
revu
```

The left pane is the [queue](concepts.md#queue): the MRs of this repo, in [sections](concepts.md#section).
Move with `j` and `k`.
Press `enter` on an MR to open its diff.

## 4. Read the diff

| Key | Does |
|---|---|
| `]c` `[c` | Next, previous change |
| `tab` | Next file |
| `za` | Fold or unfold the file or change under the cursor |
| `i` | The MR cover: description, checks, reviews, open threads, files |

## 5. Leave a comment

1. Put the cursor on a line of the diff.
2. Press `c`.
3. Type your comment in the box at the bottom of the right pane.
4. Press `enter`.

Your comment is now a [draft](concepts.md#draft): only you see it.

## 6. Publish

1. Press `P`.
2. Check the list of drafts.
3. Press `enter`.

Every draft is now public, as one [review](concepts.md#review).

## See also

- [Review and publish](guides/review-and-publish.md), for replies, suggestions and approvals.
- [Keys](reference/keys.md), for every key.
- [Troubleshooting](troubleshooting.md), when a step fails.
