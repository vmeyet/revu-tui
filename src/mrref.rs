//! How a merge request is named on the command line: `group/project!42`, `owner/repo#42`, `!42` or
//! `#42`, an MR or PR URL, or nothing.
use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectRef {
    Path(String),
    Id(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MrRef {
    pub project: ProjectRef,
    pub iid: u64,
}

/// What the argument points at once parsed: an MR, or the open MR of a branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Mr(MrRef),
    Branch { project: String, branch: String },
}

impl Target {
    /// `arg` as typed; the git checkout under `dir` fills in what the argument leaves out.
    pub fn from_arg(arg: Option<&str>, dir: &Path) -> Result<Self> {
        if let Some(text) = arg {
            return parse(text, || local_project(dir)).map(Target::Mr);
        }
        let project = local_project(dir).context("not in a checkout: name the MR as group/project!42 or owner/repo#42")?;
        let branch = current_branch(dir).context("no branch checked out")?;
        Ok(Target::Branch { project, branch })
    }
}

/// `local_project` is only asked for the bare `!42` form.
pub fn parse(text: &str, local_project: impl FnOnce() -> Option<String>) -> Result<MrRef> {
    let text = text.trim();
    if let Some(url) = text.strip_prefix("https://").or_else(|| text.strip_prefix("http://")) {
        return parse_url(url).with_context(|| format!("not a merge request URL: {text}"));
    }
    let (project, iid) =
        text.rsplit_once(['!', '#']).with_context(|| format!("not an MR reference: {text} (try group/project!42 or owner/repo#42)"))?;
    let iid: u64 = iid.parse().with_context(|| format!("not an MR number: {iid}"))?;
    let project = match project {
        "" => ProjectRef::Path(local_project().context("`!42` and `#42` need a checkout with an origin remote")?),
        digits if digits.bytes().all(|b| b.is_ascii_digit()) => ProjectRef::Id(digits.parse()?),
        path => ProjectRef::Path(path.to_owned()),
    };
    Ok(MrRef { project, iid })
}

/// `host/group/sub/project/-/merge_requests/42[/diffs][#note_1]` or `host/owner/repo/pull/42[/files]`.
fn parse_url(without_scheme: &str) -> Option<MrRef> {
    let path = without_scheme.split(['?', '#']).next()?;
    let (_, rest) = path.split_once('/')?;
    let (project, tail) = rest.split_once("/-/merge_requests/").or_else(|| rest.split_once("/pull/"))?;
    let iid = tail.split('/').next()?.parse().ok()?;
    Some(MrRef { project: ProjectRef::Path(project.trim_matches('/').to_owned()), iid })
}

/// `git@host:group/project.git` or `https://host/group/project(.git)` → `group/project`.
pub fn project_from_remote(url: &str) -> Option<String> {
    let url = url.trim();
    let path = if let Some((_, rest)) = url.split_once("://") {
        rest.split_once('/')?.1
    } else if let Some((_, rest)) = url.split_once(':') {
        rest
    } else {
        return None;
    };
    let path = path.trim_matches('/').strip_suffix(".git").unwrap_or(path.trim_matches('/'));
    (path.matches('/').count() >= 1).then(|| path.to_owned())
}

fn local_project(dir: &Path) -> Option<String> {
    project_from_remote(&git(dir, ["remote", "get-url", "origin"])?)
}

/// The host of the checkout's `origin` remote under `dir`.
pub fn origin_host(dir: &Path) -> Option<String> {
    remote_host(&git(dir, ["remote", "get-url", "origin"])?).map(str::to_owned)
}

/// The project of the checkout under `dir`, when its origin lives on `host`.
pub fn checkout_project(dir: &Path, host: &str) -> Option<String> {
    let remote = git(dir, ["remote", "get-url", "origin"])?;
    (remote_host(&remote)? == host).then(|| project_from_remote(&remote)).flatten()
}

/// `git@host:group/project.git`, `ssh://git@host:22/…` or `https://user@host/…` → `host`.
pub fn remote_host(url: &str) -> Option<&str> {
    let url = url.trim();
    let authority = match url.split_once("://") {
        Some((_, rest)) => rest.split('/').next()?,
        None => url.split_once(':')?.0,
    };
    let host = authority.rsplit('@').next()?;
    let host = host.split(':').next()?;
    (!host.is_empty()).then_some(host)
}

fn current_branch(dir: &Path) -> Option<String> {
    git(dir, ["rev-parse", "--abbrev-ref", "HEAD"]).filter(|b| b != "HEAD")
}

fn git<const N: usize>(dir: &Path, args: [&str; N]) -> Option<String> {
    let output = Command::new("git").arg("-C").arg(dir).args(args).output().ok()?;
    let text = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && !text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn no_local() -> Option<String> {
        None
    }

    #[test]
    fn every_written_form_parses() {
        let cases = [
            ("acme/widgets!42", ProjectRef::Path("acme/widgets".into()), 42),
            ("acme/sub/widgets!7", ProjectRef::Path("acme/sub/widgets".into()), 7),
            ("123!42", ProjectRef::Id(123), 42),
            ("https://gitlab.com/acme/widgets/-/merge_requests/42", ProjectRef::Path("acme/widgets".into()), 42),
            ("https://gitlab.com/acme/sub/widgets/-/merge_requests/42/diffs#note_9", ProjectRef::Path("acme/sub/widgets".into()), 42),
            ("http://gl.example/acme/widgets/-/merge_requests/3?tab=1", ProjectRef::Path("acme/widgets".into()), 3),
            ("acme/widgets#42", ProjectRef::Path("acme/widgets".into()), 42),
            ("https://github.com/acme/widgets/pull/42", ProjectRef::Path("acme/widgets".into()), 42),
            ("https://github.com/acme/widgets/pull/42/files#diff-abc", ProjectRef::Path("acme/widgets".into()), 42),
        ];
        for (text, project, iid) in cases {
            assert_eq!(parse(text, no_local).unwrap(), MrRef { project, iid }, "{text}");
        }
    }

    #[test]
    fn bare_iid_uses_the_local_project() {
        assert_eq!(
            parse("!42", || Some("acme/widgets".into())).unwrap(),
            MrRef { project: ProjectRef::Path("acme/widgets".into()), iid: 42 }
        );
        assert!(parse("!42", no_local).unwrap_err().to_string().contains("origin remote"));
        assert_eq!(
            parse("#42", || Some("acme/widgets".into())).unwrap(),
            MrRef { project: ProjectRef::Path("acme/widgets".into()), iid: 42 }
        );
    }

    #[test]
    fn garbage_says_what_a_reference_looks_like() {
        for text in [
            "nonsense",
            "acme/widgets!x",
            "https://gitlab.com/acme/widgets/-/issues/1",
            "https://github.com/acme/widgets/issues/1",
            "!",
            "#",
        ] {
            assert!(parse(text, no_local).is_err(), "{text}");
        }
        assert!(parse("nonsense", no_local).unwrap_err().to_string().contains("group/project!42"));
    }

    #[test]
    fn remotes_reduce_to_the_project_path() {
        let cases = [
            ("git@gitlab.com:acme/widgets.git", Some("acme/widgets")),
            ("ssh://git@gitlab.com/acme/sub/widgets.git", Some("acme/sub/widgets")),
            ("https://gitlab.com/acme/widgets", Some("acme/widgets")),
            ("https://oauth2:token@gitlab.com/acme/widgets.git", Some("acme/widgets")),
            ("git@github.com:acme/widgets.git", Some("acme/widgets")),
            ("https://github.com/acme/widgets.git", Some("acme/widgets")),
            ("/local/path", None),
        ];
        for (url, expected) in cases {
            assert_eq!(project_from_remote(url).as_deref(), expected, "{url}");
        }
    }

    #[test]
    fn remote_hosts_come_from_every_remote_shape() {
        for (url, host) in [
            ("git@gitlab.com:acme/widgets.git", Some("gitlab.com")),
            ("ssh://git@gitlab.acme.dev:2222/acme/widgets.git", Some("gitlab.acme.dev")),
            ("https://oauth2:tok@gitlab.com/acme/widgets", Some("gitlab.com")),
            ("https://gitlab.com/acme/widgets.git", Some("gitlab.com")),
            ("/local/path", None),
        ] {
            assert_eq!(remote_host(url), host, "{url}");
        }
    }
}
