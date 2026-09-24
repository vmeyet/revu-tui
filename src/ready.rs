//! The ready source: a command of the user's choosing prints MR links, and those MRs are the
//! ones ready for review. revu knows nothing of where they come from: a chat channel, a
//! tracker, a label search, a script.
use crate::ctx::Home;
use crate::forge::rules::Rules;
use crate::forge::{Forge, MrKey, Queue, QueueMr, Sections};
use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(10);

/// One MR link found in the command's output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub host: String,
    pub project: String,
    pub number: u64,
}

/// Every GitLab (`/-/merge_requests/N`) and GitHub (`/pull/N`) link in `text`, first mention
/// first, each once. JSON-escaped slashes, markdown links and chat link syntax all read.
pub fn links(text: &str) -> Vec<Link> {
    let text = text.replace("\\/", "/");
    let mut found: Vec<Link> = Vec::new();
    for (at, _) in text.match_indices("http") {
        let Some(link) = url_at(&text[at..]).and_then(link_of) else { continue };
        if !found.contains(&link) {
            found.push(link);
        }
    }
    found
}

/// The URL starting at the head of `text`, cut at the first character a URL in prose or JSON
/// cannot hold here, and stripped of the punctuation that ends a sentence.
fn url_at(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("https://").or_else(|| text.strip_prefix("http://"))?;
    let scheme = text.len() - rest.len();
    let end = rest.find(|c: char| c.is_whitespace() || "\"'<>|()[]{},`".contains(c)).unwrap_or(rest.len());
    Some(text[..scheme + end].trim_end_matches(['.', ';', ':', '!', '?']))
}

fn link_of(url: &str) -> Option<Link> {
    let rest = url.split_once("://")?.1;
    let (host, path) = rest.split_once('/')?;
    let (project, tail) = path.split_once("/-/merge_requests/").or_else(|| path.split_once("/pull/"))?;
    let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
    let number = digits.parse().ok()?;
    let project = project.trim_matches('/');
    (!host.is_empty() && project.contains('/')).then(|| Link { host: host.to_owned(), project: project.to_owned(), number })
}

/// Runs `command` (split into words, never through a shell) and returns what it printed.
/// A failure, a timeout or a missing program is an error naming the command.
pub async fn output(command: &str) -> Result<String> {
    output_within(command, TIMEOUT).await
}

async fn output_within(command: &str, limit: Duration) -> Result<String> {
    crate::program::run(command, None, "ready command", limit).await
}

/// What the ready command said last for a scope, and the MRs it named that no list of mine holds.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub output: String,
    pub outside: Vec<QueueMr>,
}

impl Source {
    /// The MR keys the output names, on a host I am logged in to; inside a checkout (`scope`),
    /// only its project's on the main host.
    pub fn named(&self, main_host: &str, others: &[Home], scope: Option<&str>) -> Vec<MrKey> {
        named(&self.output, main_host, others, scope)
    }

    /// The sections with the named MRs moved to Ready.
    pub fn apply(&self, sections: Sections, main_host: &str, others: &[Home], scope: Option<&str>, me: &str, rules: &Rules) -> Sections {
        sections.with_ready(&self.named(main_host, others, scope), self.outside.clone(), me, rules, Utc::now())
    }
}

/// At most this many named MRs outside my lists are fetched on each refresh.
const MAX_OUTSIDE: usize = 30;

fn named(output: &str, main_host: &str, others: &[Home], scope: Option<&str>) -> Vec<MrKey> {
    links(output)
        .into_iter()
        .filter_map(|link| {
            let host = if link.host == main_host { None } else { Some(others.iter().find(|home| home.host == link.host)?.host.clone()) };
            let in_scope = scope.is_none_or(|project| host.is_none() && link.project == project);
            in_scope.then(|| MrKey { host, ..MrKey::new(link.project, link.number) })
        })
        .collect()
}

/// A source from a fresh output: the named MRs no list holds are fetched one by one (outside a
/// checkout only: inside one, every open MR of the project is already listed).
pub async fn resolve(output: String, main: &Forge, others: &[Home], scope: Option<&str>, queues: &[&Queue]) -> Source {
    let listed: HashSet<MrKey> = queues
        .iter()
        .flat_map(|q| q.review_requested.iter().chain(&q.authored).chain(&q.assigned).chain(&q.open))
        .map(QueueMr::key)
        .collect();
    let missing: Vec<MrKey> = if scope.is_some() {
        vec![]
    } else {
        named(&output, main.host(), others, scope).into_iter().filter(|k| !listed.contains(k)).take(MAX_OUTSIDE).collect()
    };
    let forge_of =
        |key: &MrKey| key.host.as_deref().and_then(|h| others.iter().find(|home| home.host == h)).map_or(main, |home| &home.forge);
    let fetched = futures_util::future::join_all(missing.iter().map(|key| forge_of(key).mr(key))).await;
    let outside = missing.iter().zip(fetched).filter_map(|(key, mr)| QueueMr::from_mr(&mr.ok()?, key.host.clone())).collect();
    Source { output, outside }
}

/// The command reads as words; checked when the config loads.
pub fn check(command: &str) -> Result<()> {
    crate::program::check(command, "queue.ready.command")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    type Found = Vec<(String, String, u64)>;

    fn found(text: &str) -> Found {
        links(text).into_iter().map(|l| (l.host, l.project, l.number)).collect()
    }

    fn one(host: &str, project: &str, number: u64) -> Found {
        vec![(host.to_owned(), project.to_owned(), number)]
    }

    #[test]
    fn links_are_found_in_every_shape_a_source_prints() {
        let cases: Vec<(&str, Found)> = vec![
            ("ready: https://gitlab.com/acme/widgets/-/merge_requests/42", one("gitlab.com", "acme/widgets", 42)),
            ("nested https://gitlab.com/acme/shop/api/-/merge_requests/7/diffs", one("gitlab.com", "acme/shop/api", 7)),
            ("see https://github.com/acme/widgets/pull/12.", one("github.com", "acme/widgets", 12)),
            ("files https://github.com/acme/widgets/pull/12/files#diff-1", one("github.com", "acme/widgets", 12)),
            ("[the MR](https://gitlab.com/acme/widgets/-/merge_requests/42)", one("gitlab.com", "acme/widgets", 42)),
            ("<https://gitlab.com/acme/widgets/-/merge_requests/42|!42 charge>", one("gitlab.com", "acme/widgets", 42)),
            (r#"{"text":"https:\/\/gitlab.com\/acme\/widgets\/-\/merge_requests\/42"}"#, one("gitlab.com", "acme/widgets", 42)),
            ("on-prem http://git.acme.dev:8443/team/app/-/merge_requests/3, thanks", one("git.acme.dev:8443", "team/app", 3)),
            ("(https://github.com/acme/widgets/pull/12)", one("github.com", "acme/widgets", 12)),
        ];
        for (text, expected) in cases {
            assert_eq!(found(text), expected, "{text}");
        }
    }

    #[test]
    fn other_links_and_repeats_are_left_out() {
        let text = "https://gitlab.com/acme/widgets/-/issues/9 https://github.com/acme/widgets \
            https://gitlab.com/acme/widgets/-/merge_requests/ https://example.com/pull/3 \
            https://gitlab.com/acme/widgets/-/merge_requests/42 again https://gitlab.com/acme/widgets/-/merge_requests/42 \
            https://github.com/acme/widgets/pull/12";
        assert_eq!(found(text), [one("gitlab.com", "acme/widgets", 42), one("github.com", "acme/widgets", 12)].concat());
    }

    #[test]
    fn named_keys_keep_to_known_hosts_and_to_the_checkout() {
        let output = "https://gitlab.com/acme/widgets/-/merge_requests/42 https://gitlab.com/acme/billing/-/merge_requests/9 \
            https://github.com/acme/widgets/pull/12 https://gitlab.example.com/x/y/-/merge_requests/1";
        let keys = |scope| named(output, "gitlab.com", &[], scope);
        assert_eq!(keys(None), [MrKey::new("acme/widgets", 42), MrKey::new("acme/billing", 9)], "unknown hosts are left out");
        assert_eq!(keys(Some("acme/widgets")), [MrKey::new("acme/widgets", 42)], "a checkout keeps to its project");
    }

    use crate::program::OUTPUT_CAP;
    use crate::program::tests::script;

    #[tokio::test]
    async fn a_command_that_prints_links_is_read() {
        let (_dir, command) = script("echo 'ready https://gitlab.com/acme/widgets/-/merge_requests/42'");
        assert_eq!(links(&output(&command).await.unwrap()).len(), 1);
        assert!(output("echo one two").await.unwrap().contains("one two"), "arguments pass as words");
    }

    #[tokio::test]
    async fn failures_name_the_command_and_say_why() {
        let (_dir, failing) = script("echo 'token expired' >&2\nexit 3");
        let err = output(&failing).await.unwrap_err().to_string();
        assert!(err.contains("failed") && err.contains("token expired"), "{err}");
        let err = output("no-such-program-revu-test").await.unwrap_err().to_string();
        assert!(err.contains("cannot run `no-such-program-revu-test`"), "{err}");
        assert!(output("").await.unwrap_err().to_string().contains("empty"));
        assert!(output("echo 'open").await.is_err(), "an unclosed quote does not split");
    }

    #[tokio::test]
    async fn a_slow_command_is_stopped() {
        let (_dir, slow) = script("sleep 30");
        let started = std::time::Instant::now();
        let err = output_within(&slow, Duration::from_secs(1)).await.unwrap_err().to_string();
        assert!(err.contains("took longer than 1s"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5), "the command is killed, not waited for");
    }

    #[tokio::test]
    async fn a_huge_output_is_cut_at_the_cap() {
        let (_dir, loud) = script("yes https://gitlab.com/acme/widgets/-/merge_requests/42 | head -c 3000000");
        let out = output(&loud).await.unwrap();
        assert_eq!(out.len() as u64, OUTPUT_CAP);
    }

    #[test]
    fn the_config_check_refuses_what_cannot_run() {
        assert!(check("slack messages '#review' --json").is_ok());
        assert!(check("   ").is_err());
        assert!(check("slack 'open").is_err());
    }
}
