# Troubleshooting

Find your problem in the left column, then apply the fix.

| Problem | Fix |
|---|---|
| `⌘K` does nothing | Use `ctrl-k`, or a terminal that forwards `⌘`: Ghostty, Kitty, WezTerm or iTerm2 |
| `⌘enter` does not post | Use `ctrl-s`. Terminal.app never sends `⌘`; iTerm2 and Kitty keep `⌘enter` for themselves; in Ghostty add `keybind = super+enter=unbind` |
| `]n` does nothing on AZERTY | Set `[keys] layout = "azerty"` and use `)n`, or see the Option key row |
| The Option key types nothing | Let Option type characters: in Ghostty `macos-option-as-alt = false`, in iTerm2 set Left Option to Normal |
| Dragging the mouse copies only the code or the comment | That is revu's own selection; hold shift while dragging (Ghostty, Kitty, WezTerm) or ⌥ (iTerm2, Terminal.app) to select whole screen rows |
| `revu update` is slow | The first update builds every library; the next ones build only revu |
| Pictures in comments do not show | Use a terminal that draws pictures, and check `[tui] images` is not `false` |
| `no token for gitlab.com` | Run `revu login --from-glab`, or set `GITLAB_TOKEN` |
| `token rejected` at login | The token expired or lacks a scope: GitLab needs `api`, GitHub needs `repo` |
| My config seems ignored | It moved to `~/.config/revu/config.toml`; revu moves an old one there once |
| revu stops at start with a config error | Fix the key the message names; revu refuses keys it does not know |
| `a` does nothing | Switch Claude on and store its key; see the AI guide |
| `q` does not quit | Press it twice: the first press asks, so a stray `q` never ends your session; with a pane open, `q` closes it first; set `[keys] quit_confirm = false` to quit at once |

## Still stuck

Run the command again with `--json` to see the raw answer.
Open an issue with the message you see.

## See also

- [Keys and AZERTY](guides/keys-and-azerty.md).
- [AI](guides/ai.md).
- [Update revu](guides/update.md).
