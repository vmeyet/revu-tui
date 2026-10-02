# 10 · Symbol outline

Goal: see which functions, methods and classes an MR changes, and how they call each other, without reading every hunk.

## Opening it

`O` in the diff, or `:outline`, opens the outline in the right pane, as the pipeline does; `O`, `esc` or `x` closes it.
It replaces whatever held the pane, and the pane takes the keys.

## What it reads

- Every changed file whose language has a tags query: TypeScript, TSX, JavaScript, Python. JSON, Markdown, SQL and binary files are skipped silently.
- Both sides of each file come through the forge's file read at the base and head commits, cached forever per commit as `^v` does; an added file has no base side, a deleted one no head side.
- Definitions come from `tree-sitter-tags` with each grammar crate's own `TAGS_QUERY`; TypeScript and TSX join the JavaScript query with TypeScript's, which only adds signatures and abstract classes. No query is written by hand.
- Only functions, methods and classes list. A Python function inside a class is a method, named `Class.name`.
- A function held by an object literal's property (`{ reload: () => … }`) is not a symbol, though the query tags it.
- Reading and parsing run in the background; the pane shows a spinner until they end, and a failure with `r to retry`.

## Comparing the two sides

A symbol's signature is its definition from its start up to its body, whitespace collapsed, a trailing `:` dropped; in TypeScript and JavaScript it starts at the `export` holding the definition, so dropping the `export` changes it.
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

The pane shows public symbols; `a` switches to every symbol and back, the title says which.

- TypeScript, JavaScript: inside an `export` statement.
- Python: a name without a leading `_`.

A symbol public at base counts as public: one that loses its `export` is a breaking signature change.

## Code and tests

The pane has two sections under the queue's section rules: `CODE` and `TESTS`; a section with nothing to show is left out.
A test file is a path with a `test`, `tests` or `__tests__` folder, or named `*.test.*`, `*.spec.*`, `test_*.py` or `*_test.py`.

- `CODE` holds the tree below (or the list under `t`) built from the other files only; a call from code into a test file links nowhere. Its header counts its changes, and the badge under it counts code only.
- `TESTS` is folded at first; `zo`, `za` or `enter` on its header opens it, `zc` closes it. Every changed test symbol is a root, whatever `a` says, and under it the code symbols it calls, as in `CODE` (changes in full, bridges, `s`): what each test covers. It always reads as calls; nothing links into a test file.

## Call tree

The tree is the default; `t` switches to the flat list by file and back.

- Edges come from the `@reference.call` captures of the same tags queries; nothing is written by hand.
  A plain call to a name the caller binds itself (a parameter, a variable or destructured name, an inner function; in Python a parameter or an assignment) calls that local and links nowhere; `obj.name()` still links.
  A call belongs to the innermost definition around it, and links by name to the symbols of the MR's changed files on its own side: a call read on head (from an added, changed or unchanged symbol) links only to symbols that exist on head, a call from a removed symbol only to symbols that exist on base.
- Roots are entry points, so the tree reads as the MR's call stack: a root is a changed symbol (public, or any with `a`) that no other changed or bridge symbol calls, in the flat list's order. A shown change that every caller of hides (only private ones call it, and `a` is off) is a root too.
- Every other change hangs in full under its first caller: state, signature diff and children. Only its later appearances are dimmed reference rows.
- An unchanged function of a changed file shows dimmed as `· name()` only when it links two changed symbols. At most two of them stand in a row; a longer chain folds into a dimmed `… N calls` row, N the unchanged calls folded, with the changed symbol under it.
- A name several symbols on the call's side carry (methods, the same name in two files) links to each of them, marked `?`.
- `s` shows the whole stack: every unchanged function the tree reaches inside the changed files, dimmed, with no limit of two; each shows in full once, then bare, and cycles are cut. The title ends in `· stack`.
- A call back to a symbol above shows `↺ name` and stops there.
- `u` turns every branch around: under each symbol, the changed or bridge symbols that call it, which matters most for removed or re-signed ones.
- Rows are drawn with `├─ └─ │`, the state signs above, and the signature word diff under `~` rows shown in full.

## Drawing

The rows, word diffs included, are built once each time the pane's state changes (an answer, `a`, `t`, `u`, `s`, a fold); a frame only draws the rows in view.

## Order and badge

Grouped by file in diff order; inside a file: public removals, public signature changes, public additions, renames, body changes, then private removals, signatures and additions.
The first row counts what is listed: `2 breaking · 3 added · 1 renamed`, zeros left out; breaking is a public symbol removed or with a changed signature.

## Keys

`j` `k` move, `g` `G` first and last, `^d` `^u` half a page, `a` public or all, `t` tree or list, `u` calls or called by, `s` whole stack, `zo` `zc` `za` open, close, toggle the branch under the cursor, `r` reads again.
`enter` on a section header folds or opens it; elsewhere it hands the keys to the diff with its cursor, file and hunk opened, on the call linking a row to its parent; on a root, a row of the list, or a row after a fold, on the symbol's first line the diff shows (head, base for a removed one).
A call on a line the diff does not show says so in the status line.
