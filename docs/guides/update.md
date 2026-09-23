# Update revu

`revu update` installs the latest version from GitHub.

```sh
revu update       # does nothing when you already run the latest commit
revu update -f    # rebuilds anyway
revu --version    # shows the version and the commit
```

## How long it takes

| Update | Time |
|---|---|
| The first one | A few minutes: every library is built |
| The next ones | About half a minute: only revu is built |

revu keeps the built libraries in `~/Library/Caches/revu/cargo_target`.
An update that brings a new or changed library builds that one again.

## See also

- [Troubleshooting: slow update](../troubleshooting.md).
