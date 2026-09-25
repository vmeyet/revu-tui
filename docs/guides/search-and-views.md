# Search and saved views

`⌘K` finds any MR, file or command, and `/` filters the queue with the same words.

## Search with ⌘K

Press `⌘K` or `ctrl-k`.
Type to search.
The first character picks what you search.

| You type | Finds |
|---|---|
| `charge` | MRs whose title, author, project or branch hold the word |
| `/charge.rs` | Files of the open MR |
| `>publish` | Commands |

`:` opens the search on commands directly.
`↑` `↓` pick a command from the list, and `enter` runs it.
A command that needs more, like `go`, waits for you to finish the line.
`ctrl-p` and `ctrl-n` bring back the commands you ran before.
`⌫` on an empty line goes back to MRs, then closes.

## Filter the queue

Press `/` in the queue and type a query.
`esc` clears it.

| Term | Keeps MRs |
|---|---|
| `word`, `"two words"` | With the word in the title, author, project or branch |
| `@nina`, `@me` | By that author |
| `!42`, `#42` | Of that number |
| `~infra` | With that label |
| `draft:yes`, `draft:no` | That are drafts, or not |
| `size:small`, `size:large` | Up to 100 changed lines, or 500 and more |
| `is:failing` | Whose pipeline failed, or that conflict |
| `is:mine` | That you wrote |

Terms add up: `@nina ~infra slack` keeps Nina's infra MRs about slack.

## Save a view

Add views to `~/.config/revu/config.toml`.

```toml
[queue.views]
mine = "is:mine"
failing = "is:failing"
small = "size:small draft:no"
```

Press `'` then the first letter of a view to apply it.
`1` to `9` apply views in name order.
Two views may not start with the same letter.

## Sort and group

| Key in the queue | Does |
|---|---|
| `s` | Next order: updated, oldest, author, size |
| `S` | Group OPEN and DRAFTS by author |
| `*` | This project, or every project |

## See also

- [Concepts: queue and section](../concepts.md#queue).
- [Config reference](../reference/config.md).
