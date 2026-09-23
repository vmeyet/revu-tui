# Concepts

revu shows the MRs you care about, lets you read them, and turns your comments into one review.

## Words

### MR

A merge request on GitLab, or a pull request on GitHub.
revu calls both an MR.
`acme/widgets!42` is MR 42 of the GitLab project `acme/widgets`, and `acme/widgets#42` is PR 42 on GitHub.

### Forge

The site that hosts the code: GitLab or GitHub.
revu picks it from the host name: `github.com` is GitHub, any other host is GitLab.
`[hosts."<host>"] forge` says it for a host whose name does not tell.

### Queue

The list of MRs in the left pane.
Inside a checkout, it shows that project only.
`*` switches to every project you can see.

### Section

A group of MRs in the queue.

| Section | Holds |
|---|---|
| TO REVIEW | MRs where you are a reviewer and have not approved yet |
| MINE | MRs you wrote |
| READY | MRs a [ready source](guides/ready-source.md) names, when they need you |
| WATCHING | MRs assigned to you, or with a label from `[queue] watch_labels` |
| OPEN | Every other open MR of the project |
| DRAFTS | Other people's draft MRs, folded |
| OTHER | MRs the "needs me" rules moved out, folded |
| DONE | MRs you already approved or reviewed, folded |

MRs by one author that build on each other fold into one stack row.

### Draft

A comment only you can see.
Every comment you write in revu starts as a draft.
Drafts survive a restart.

### Review

All your drafts on one MR, made public at once with `P`.
You can approve in the same step.

| | GitLab | GitHub |
|---|---|---|
| A draft is | a draft note | a comment in your pending review |
| `P` sends | every draft note | the pending review |
| Approve with it | yes | yes |

## How the queue is built

```mermaid
flowchart LR
    F["Forge: your MRs, review requests, the project's MRs"] --> S["Sections"]
    R["Ready source"] --> S
    S --> N["Needs me rules"]
    N --> Q["Query from / or a saved view"]
    Q --> P["Queue pane"]
```

revu asks the forge at start, then again every minute.
It paints from its cache first, so the screen is never empty.

## What "needs me" means

Plain rules keep TO REVIEW, WATCHING and OPEN to what needs you.
An MR that fails a rule moves to OTHER, which is folded.

| Rule | An MR moves when |
|---|---|
| Stale | No activity for 14 days |
| Approved enough | It has an approval and needs no more |
| Not ready | It is a draft, its pipeline fails, or its title says `WIP` |
| Reviewed | Others left 3 comments and you left none: it stays, sorted last |

An MR that asks you by name escapes the stale and approved rules.
Your own MRs are never moved.
The status line says why the selected MR sits where it does, for example `stale 21d`.
`[queue.rules]` changes the numbers, and `enabled = false` turns the rules off.

## How a review goes

```mermaid
flowchart LR
    O["Open an MR"] --> R["Read the diff"]
    R --> C["Comment: a draft"]
    C --> R
    R --> P["P: publish every draft"]
    P --> A["Optionally approve"]
```

## Where revu keeps things

| What | Where |
|---|---|
| Tokens and AI keys | macOS keychain, service `revu` |
| Config | `~/.config/revu/config.toml` |
| Cache | `~/Library/Caches/revu/` |

## See also

- [Start here](start.md), to try it.
- [Config reference](reference/config.md), for every setting.
