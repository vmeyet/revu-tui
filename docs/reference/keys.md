<!-- Written by `revu docs` from the code. Edit the code, then run `revu docs`. -->
# Keys

Every key revu reads, grouped by task.
Press `?` in revu for the same list, with your own keys.
A key like `]n` is two keys in a row: `]`, then `n`.

## Move

| Keys | Does |
|---|---|
| `j` `k` | move |
| `g` `G` | first, last |
| `^d` `^u` | half a page |
| `h` `l` | pane to the left, to the right |
| `enter` | open, or toggle the fold |
| `esc` `x` | close the pane, back to the queue |
| `tab` `S-tab` | next, previous file |
| `]c` `[c` | next, previous hunk |
| `]n` `[n` | next, previous conversation |
| `]f` `[f` | next, previous file with a thread |
| `]r` `[r` | next, previous MR that needs you |

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
| `D` | changed words inline, or split |
| `t` | file tree |
| `p` | pipeline |
| `zz` | zen: the diff alone, quiet |
| `←` `→` | in zen: previous, next MR |
| `w` `W` | wrap long lines, hide whitespace changes |
| `+` | more lines around the hunk |
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
| `⌥enter` `^o` | in the box: new line, $EDITOR |
| `esc` | drop the selection, leave the box |
| `P` | publish every draft |
| `A` | approve, unapprove |
| `Y` | share the MR, after a preview |

## Thread pane

| Keys | Does |
|---|---|
| `r` | reply, as a draft |
| `R` | resolve, unresolve |
| `J` `K` | next, previous thread on the line |
| `e` `d` | edit, delete my draft |
| `S` | apply the suggestion, after a y |
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
| `q` `^c` | quit |

## AZERTY

With `[keys] layout = "azerty"`, these keys change.
The old ones keep working.

| Default | AZERTY |
|---|---|
| `]c` `[c` | `)c` `(c` |
| `]n` `[n` | `)n` `(n` |
| `]f` `[f` | `)f` `(f` |
| `]r` `[r` | `)r` `(r` |

## Actions you can bind

Give an action more keys under `[keys.bind]`, as in `next_thread = "N"`.
A bound key adds to the default key, it does not replace it.

| Action | Default key |
|---|---|
| `next_thread` | `]n` |
| `prev_thread` | `[n` |
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
| `prev_mr` | `left` |
| `next_mr` | `right` |
| `split` | `D` |
| `tree` | `t` |
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
| `share` | `Y` |
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
