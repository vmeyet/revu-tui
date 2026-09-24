# Share an MR

`Y` posts the MR under the cursor to a place you choose: a chat channel, a webhook, a script.
revu fills a message and pipes it to a command; it knows no chat tool.

## Set it up

Give revu the command, and the message to send.

```toml
[share]
command = "slack send '#review'"
template = "[{ref} {title}]({url})\n_{note}_"
```

The command runs without a shell.
The message arrives on its stdin, so quotes in a title cannot break it.
Without `template`, revu uses the one above.

## Share

1. Press `Y` on an MR, in the queue or in the diff, or type `:share`.
2. Type a note, or leave it empty.
3. Read the message revu will send.
4. Press `y` to send it, `e` to change the note, `esc` to cancel.

Nothing leaves before that `y`.
`revu share !42 --note "needs a second look"` does the same from the shell; `--dry-run` only prints.

## Placeholders

| Placeholder | Becomes |
|---|---|
| `{ref}` | `!42` on GitLab, `#42` on GitHub |
| `{iid}` | `42` |
| `{title}` | The MR title |
| `{url}` | The MR link |
| `{author}` | The author's handle |
| `{project}` | `acme/widgets` |
| `{branch}` | The MR's branch |
| `{note}` | Your note |

A line with `{note}` is left out when the note is empty.
A typo like `{titel}` stops revu at start, with the file and the target named.

## Several places

Name each place under `[share.targets]`.
`Y` then asks which one; `:share team` or `--target team` picks it directly.

```toml
[share.targets.review]
command = "slack send '#review'"

[share.targets.team]
command = "slack send '#team'"
template = "{ref} {title} by {author}: {url}"
```

A target without `template` uses `[share] template`, then the built-in one.

## Examples

| Place | Command |
|---|---|
| A Slack channel | `slack send '#review'` |
| A webhook | `curl -sS --data-binary @- https://hooks.acme.dev/review` |
| A GitHub issue | `gh issue comment 7 --repo acme/widgets --body-file -` |
| Your own script | `~/bin/post-review` |

## Share and READY together

If your [ready source](ready-source.md) reads the channel you share to, the MR shows up in READY on the next refresh.

## See also

- [Ready source](ready-source.md).
- [Config reference](../reference/config.md).
