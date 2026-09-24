<!-- Written by `revu docs` from the code. Edit the code, then run `revu docs`. -->
# Config

revu reads `~/.config/revu/config.toml`, or `$XDG_CONFIG_HOME/revu/config.toml`.
Every key is optional.
A key revu does not know stops it at start, with the file and the key named.
Tokens never go here: they live in the macOS keychain.

## Top level

| Key | Type | Default | Does |
|---|---|---|---|
| `host` | text | gitlab.com | The host revu uses outside a checkout. |
| `username` | text | set by `revu login` | Your name on that host. |

## `[queue]`

| Key | Type | Default | Does |
|---|---|---|---|
| `watch_labels` | list of text | `[]` | MRs with one of these labels go to WATCHING. |
| `prefetch` | number | `5` | How many MRs that need you load ahead, so opening them needs no network; 0 turns it off. |

## `[queue.ready]`

| Key | Type | Default | Does |
|---|---|---|---|
| `command` | command line | none | A program that prints MR links; the open ones that need you fill READY. |

## `[queue.rules]`

| Key | Type | Default | Does |
|---|---|---|---|
| `enabled` | true or false | `true` | Keep To review, Watching and Open to what needs you; the rest moves to OTHER, folded. |
| `stale_days` | number | `14` | An MR with no activity for longer moves to OTHER. |
| `reviewed_comments` | number | `3` | An MR with this many comments from others and none from you sorts last. |
| `not_ready` | list of text | `["wip", "do not review", "don't review", "not ready"]` | Words in the title or the description's first paragraph that mark an MR as not ready. |

## `[share]`

| Key | Type | Default | Does |
|---|---|---|---|
| `command` | command line | none | What `Y` posts through: the message arrives on its stdin. |
| `template` | text with placeholders | `[{ref} {title}]({url})` then `_{note}_` | The message; a line with `{note}` goes when the note is empty. |

## `[share.targets]`

| Key | Type | Default | Does |
|---|---|---|---|
| `<name>` | table | none | One more place to post to; `Y` asks which when there are several. |

## `[share.targets.<name>]`

| Key | Type | Default | Does |
|---|---|---|---|
| `command` | command line | none | What this target posts through: the message arrives on its stdin. |
| `template` | text with placeholders | `[share] template` | This target's message. |

## `[queue.views]`

| Key | Type | Default | Does |
|---|---|---|---|
| `<name>` | search query | none | A saved filter; `'` and its first letter applies it. |

## `[review]`

| Key | Type | Default | Does |
|---|---|---|---|
| `fold` | list of globs | `[]` | Files that open folded. |
| `inline_max_words` | number | 2 | At most this many changed words per side show on one row. |
| `inline_min_same` | percent | 60 | Both lines keep at least this share of text to show on one row. |

## `[tui]`

| Key | Type | Default | Does |
|---|---|---|---|
| `theme` | text | `default` | The colour palette. |
| `queue` | `comfortable` or `compact` | `comfortable` | Two lines per MR in the queue, or one. |
| `images` | true or false | true | Draw pictures from comments where the terminal can. |
| `zen_width` | number | 70 % of the screen, 100 at least | How wide the diff reads in zen, `zz`, in columns; 60 at least. Set, it stays fixed. |
| `ascii` | true or false | false | Reactions in plain words (`+1 2`) for terminals that draw emoji at the wrong width. |

## `[open]`

| Key | Type | Default | Does |
|---|---|---|---|
| `default` | command | `$VISUAL`, `$EDITOR`, then `less` | The program `v` opens a file with. |

## `[open.files]`

| Key | Type | Default | Does |
|---|---|---|---|
| `<glob>` | command | none | The program for files that match; the longest glob wins. |

## `[notify]`

| Key | Type | Default | Does |
|---|---|---|---|
| `enabled` | true or false | true | A macOS notification when an MR lands in TO REVIEW. |

## `[usage]`

| Key | Type | Default | Does |
|---|---|---|---|
| `enabled` | true or false | `false` | Count the actions you use and the time per screen, on this machine only, for `revu usage`. |

## `[keys]`

| Key | Type | Default | Does |
|---|---|---|---|
| `layout` | `qwerty` or `azerty` | `qwerty` | With `azerty`, `(` and `)` work like `[` and `]`. |
| `quit_confirm` | true or false | `true` | `q` and `ctrl-c` quit on a second press; `false` quits on the first. |

## `[keys.bind]`

| Key | Type | Default | Does |
|---|---|---|---|
| `<action>` | key or list of keys | none | Extra keys for an action; see the actions table. |

## `[ai.typesafe]`

| Key | Type | Default | Does |
|---|---|---|---|
| `enabled` | true or false | false | Jev marks urgent MRs and risky files. |

## `[ai.anthropic]`

| Key | Type | Default | Does |
|---|---|---|---|
| `enabled` | true or false | false | `a` asks Claude about the diff. |
| `model` | text | `claude-opus-5` | The Claude model to ask. |

## `[hosts."<host>"]`

| Key | Type | Default | Does |
|---|---|---|---|
| `forge` | `gitlab` or `github` | from the host name | The forge of a host whose name does not say it. |
| `username` | text | set by `revu login` | Your name on that host. |

## Example

```toml
host = "gitlab.com"
username = "nina"

[queue]
watch_labels = ["infra"]
prefetch = 5

[queue.ready]
command = "slack messages '#review' --since 14d --json"

[queue.rules]
enabled = true
stale_days = 14
reviewed_comments = 3
not_ready = ["wip", "not ready"]

[share]
command = "slack send '#review'"
template = "[{ref} {title}]({url})\n_{note}_"

[share.targets]
mine = { command = "slack send '#team'" }

[share.targets.team]
command = "slack send '#team'"
template = "{ref} {title} {url}"

[queue.views]
mine = "@me ~frontend"

[review]
fold = ["*.lock", "**/generated/**"]
inline_max_words = 2
inline_min_same = 60

[tui]
theme = "catppuccin"
queue = "compact"
images = false
zen_width = 100
ascii = false

[open]
default = "hx"

[open.files]
"*.md" = "glow -p"

[notify]
enabled = false

[usage]
enabled = true

[keys]
layout = "azerty"
quit_confirm = true

[keys.bind]
next_thread = "N"

[ai.typesafe]
enabled = true

[ai.anthropic]
enabled = true
model = "claude-opus-5"

[hosts."git.acme.dev"]
forge = "github"
username = "nina"
```
