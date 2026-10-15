# GitLab, GitHub and several hosts

revu works with GitLab and GitHub, and can show MRs from every host you log in to.

## Log in to each host

```sh
revu login --from-glab                    # gitlab.com
revu login github.com --from-gh           # github.com
revu login gitlab.acme.dev --token - < token.txt
```

`GITLAB_TOKEN`, `GITHUB_TOKEN` or `GH_TOKEN` override the keychain for scripts, on one host only.

| Variable | Host it is sent to |
|---|---|
| `GITLAB_TOKEN` | `--host`, else `GITLAB_HOST`, when it is a GitLab host; else gitlab.com |
| `GITHUB_TOKEN`, `GH_TOKEN` | `--host`, else `GITLAB_HOST`, when it is a GitHub host; else github.com |

`host` in the config never gets a token variable: to send one to `gitlab.acme.dev`, set `GITLAB_HOST=gitlab.acme.dev` too.
Every other host uses the token `revu login` stored for it.

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
