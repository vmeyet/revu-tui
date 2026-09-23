# GitLab, GitHub and several hosts

revu works with GitLab and GitHub, and can show MRs from every host you log in to.

## Log in to each host

```sh
revu login --from-glab                    # gitlab.com
revu login github.com --from-gh           # github.com
revu login gitlab.acme.dev --token - < token.txt
```

`GITLAB_TOKEN`, `GITHUB_TOKEN` or `GH_TOKEN` override the keychain for scripts.

## Which host revu uses

| Where you run revu | Host |
|---|---|
| A checkout | The host of its `origin` remote |
| Anywhere else | Every host you logged in to, in one queue |

With several hosts in one queue, each row shows its host.
`--all` or `*` shows every project, even inside a checkout.

## A GitHub Enterprise host

revu cannot tell a GitHub Enterprise host from its name.
Say it in the config.

```toml
[hosts."git.acme.dev"]
forge = "github"
```

## See also

- [Concepts: forge](../concepts.md#forge).
- [Commands: `revu login`](../reference/commands.md).
