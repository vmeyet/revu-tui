# Update revu

`revu update` installs the latest version from GitHub.
When brew installed revu, it runs `brew upgrade vmeyet/tap/revu` instead, and `-f` runs `brew reinstall`.

```sh
revu update       # does nothing when you already run the latest commit
revu update -f    # rebuilds anyway
revu --version    # shows the version and the commit
```

## See what is new

After an update, revu lists what changed since the version you had.
Each entry says what is new, where it applies and how to use it.

```sh
revu news                  # every version, newest first
revu news --since 0.5.0    # only what came after 0.5.0
revu news --json           # for a script
```

Inside revu, `:news` opens the same page.
`j` `k` scroll it, and `esc` closes it.
`● new for you` marks the versions that came after the one you last ran.
On the first start after an update, a line at the bottom says how much is new, once.

A build from `main` has the number of the last release.
What it adds shows under `Unreleased`, and revu counts the entries added since your last build.

## How long it takes

| Update | Time |
|---|---|
| The first one | A few minutes: every library is built |
| The next ones | About half a minute: only revu is built |

revu keeps the built libraries in `~/Library/Caches/revu/cargo_target`.
An update that brings a new or changed library builds that one again.

## See also

- [What's new](../../CHANGELOG.md), the same page on the web.
- [Commands: `revu news`](../reference/commands.md).
- [Troubleshooting: slow update](../troubleshooting.md).
