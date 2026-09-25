<!-- Written by `revu docs` from the code. Edit the code, then run `revu docs`. -->
# Keys

Every key revu reads, grouped by task.
Press `?` in revu for the keys of the pane you are in, `?` again for this whole list, with your own keys.
A key like `]n` is two keys in a row: `]`, then `n`.

## Move

| Keys | Does |
|---|---|
| `j` `k` | move |
| `g` `G` | first, last |
| `^d` `^u` | half a page; PgDn PgUp space too |
| `h` `l` | pane to the left, to the right |
| `enter` | open, or toggle the fold |
| `esc` `x` | close the pane, back to the queue |
| `tab` `S-tab` | next, previous file |
| `]c` `[c` | next, previous hunk |
| `]n` `[n` | next, previous open conversation |
| `]N` `[N` | next, previous conversation, resolved too |
| `]f` `[f` | next, previous file with a thread |
| `]r` `[r` | next, previous MR that needs you |
| `]m` `[m` | next, previous MR in the queue |

## Queue

| Keys | Does |
|---|---|
| `/` | filter: words @author !42 ~label |
| `'` `1-9` | a saved view, by letter or rank |
| `*` | this repo, or every project |
| `s` | next sort order |
| `S` | group by author |
| `zo` `zc` | open, fold the section or stack |

## View

| Keys | Does |
|---|---|
| `i` | the MR cover page |
| `D` | inline diff, or side by side |
| `t` | file tree |
| `T` | every thread of the MR |
| `p` | pipeline |
| `zz` | zen: the diff alone, quiet |
| `w` `W` | wrap long lines, hide whitespace changes |
| `+` | more lines; on a thread, react |
| `v` | the file in your own program |
| `za` `zc` `zo` | toggle, close, open the fold |
| `zM` `zR` | fold, unfold every file |
| `zh` | fold the MR header |
| `zv` | file viewed: it folds away |

## Comment & publish

| Keys | Does |
|---|---|
| `c` | comment on the line |
| `C` | comment on the old side |
| `V` | select lines |
| `s` | suggest a change |
| `E` | write in $EDITOR |
| `⌘enter` `^s` | in the box: post now, no draft |
| `⌥enter` `^o` | in the box: new line, $EDITOR |
| `⌥←` `⌥→` `⌥⌫` | in the box: word back, on, delete |
| `esc` | drop the selection, leave the box |
| `P` | publish every draft |
| `A` `M` | approve or unapprove, merge mine |
| `H` | mark mine draft or ready |
| `Y` | share the MR, after a preview |

## Thread pane

| Keys | Does |
|---|---|
| `r` | reply, as a draft |
| `R` | resolve, unresolve |
| `J` `K` | next, previous thread |
| `T` | every thread; enter goes to it |
| `e` `d` | edit, delete my draft |
| `S` `+` | apply the suggestion, react |
| `u` | open the first link |

## Ask claude

| Keys | Does |
|---|---|
| `a` `e` `r` `s` | explain, risks, summary |
| `a` `t` `c` `a` | thread, comment, anything |
| `c` `⏎` `R` `y` | answer: draft, follow up, again, copy |

## Search & app

| Keys | Does |
|---|---|
| `^k` | MRs · / files · > commands |
| `:` | commands: :go !42, :set theme=nord |
| `o` `y` | open in the browser, copy the link |
| `r` | refresh |
| `?` | this help |
| `q` `^c` | quit: twice; q closes a pane first |

## AZERTY

With `[keys] layout = "azerty"`, these keys change.
The old ones keep working.

| Default | AZERTY |
|---|---|
| `]c` `[c` | `)c` `(c` |
| `]n` `[n` | `)n` `(n` |
| `]N` `[N` | `)N` `(N` |
| `]f` `[f` | `)f` `(f` |
| `]r` `[r` | `)r` `(r` |
| `]m` `[m` | `)m` `(m` |

## Actions you can bind

Give an action more keys under `[keys.bind]`, as in `next_thread = "N"`.
A bound key adds to the default key, it does not replace it.

| Action | Default key |
|---|---|
| `next_thread` | `]n` |
| `prev_thread` | `[n` |
| `next_any_thread` | `]N` |
| `prev_any_thread` | `[N` |
| `next_hunk` | `]c` |
| `prev_hunk` | `[c` |
| `next_file_unresolved` | `]f` |
| `prev_file_unresolved` | `[f` |
| `next_review` | `]r` |
| `prev_review` | `[r` |
| `next_file` | `tab` |
| `prev_file` | `backtab` |
| `fold_toggle` | `za` |
| `fold_open` | `zo` |
| `fold_close` | `zc` |
| `fold_all` | `zM` |
| `unfold_all` | `zR` |
| `fold_header` | `zh` |
| `viewed` | `zv` |
| `zen` | `zz` |
| `prev_mr` | `[m` |
| `next_mr` | `]m` |
| `side_by_side` | `D` |
| `tree` | `t` |
| `every_thread` | `T` |
| `pipeline` | `p` |
| `wrap` | `w` |
| `whitespace` | `W` |
| `more_context` | `+` |
| `view_file` | `v` |
| `description` | `i` |
| `comment` | `c` |
| `comment_old` | `C` |
| `select` | `V` |
| `suggest` | `s` |
| `editor` | `E` |
| `resolve` | `R` |
| `publish` | `P` |
| `approve` | `A` |
| `merge` | `M` |
| `ready` | `H` |
| `share` | `Y` |
| `react` | `+` |
| `open_browser` | `o` |
| `copy_link` | `y` |
| `scope` | `*` |
| `sort_queue` | `s` |
| `group_by_author` | `S` |
| `filter` | `/` |
| `palette` | `:` |
| `views` | `'` |
| `jump` | `ctrl-k` |
| `help` | `?` |
| `quit` | `q` |
