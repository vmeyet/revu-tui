# 10 · Symbol outline

Goal: see which functions, methods and classes an MR changes without reading every hunk.

## Opening it

`O` in the diff, or `:outline`, opens the outline in the right pane, as the pipeline does; `O`, `esc` or `x` closes it.
It replaces whatever held the pane, and the pane takes the keys.

## What it reads

- Every changed file whose language has a tags query: TypeScript, TSX, JavaScript, Python. JSON, Markdown, SQL and binary files are skipped silently.
- Both sides of each file come through the forge's file read at the base and head commits, cached forever per commit as `^v` does; an added file has no base side, a deleted one no head side.
- Definitions come from `tree-sitter-tags` with each grammar crate's own `TAGS_QUERY`; TypeScript and TSX join the JavaScript query with TypeScript's, which only adds signatures and abstract classes. No query is written by hand.
- Only functions, methods and classes list. A Python function inside a class is a method, named `Class.name`.
- Reading and parsing run in the background; the pane shows a spinner until they end, and a failure with `r to retry`.

## Comparing the two sides

A symbol's signature is its definition from its start up to its body, whitespace collapsed, a trailing `:` dropped.
Symbols are matched by kind and name inside one file; a renamed file is read at its old path on base and its new one on head.

| Glyph | State | When |
|---|---|---|
| `-` | removed | only on base |
| `~` | signature changed | on both, signatures differ; a second row shows old → new as the diff's word diff |
| `+` | added | only on head |
| `→` | renamed | one removed and one added of the same kind and class whose body words match at 0.8 or more; below that they stay `-` and `+` |
| `·` | body only | on both, same signature, body differs beyond whitespace |

Unchanged symbols are not listed.
A class lists only when it comes, goes or changes its signature: its methods carry the body changes.

## Public or all

The pane lists public symbols; `a` switches to every symbol and back, the title says which.

- TypeScript, JavaScript: inside an `export` statement.
- Python: a name without a leading `_`.

## Order and badge

Grouped by file in diff order; inside a file: public removals, public signature changes, public additions, renames, body changes, then private removals, signatures and additions.
The first row counts what is listed: `2 breaking · 3 added · 1 renamed`, zeros left out; breaking is a public symbol removed or with a changed signature.

## Keys

`j` `k` move, `g` `G` first and last, `^d` `^u` half a page, `enter` puts the diff's cursor on the symbol (its first line the diff shows, on head, on base for a removed one, its file and hunk opened) and hands the keys to the diff, `a` public or all, `r` reads again.
