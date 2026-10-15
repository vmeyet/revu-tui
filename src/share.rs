//! `Y`: post an MR somewhere by piping a message to a command of the user's choosing.
//! revu fills a template and hands the text to the command's stdin; it knows no chat tool.
use crate::cache::{Cache, keys};
use crate::config;
use crate::forge::{Kind, Mr, QueueMr};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

/// A link titled with the MR's ref, then the note in italics when there is one.
pub(crate) const DEFAULT_TEMPLATE: &str = "[{ref} {title}]({url})\n_{note}_";

const PLACEHOLDERS: [&str; 8] = ["ref", "iid", "title", "url", "author", "project", "branch", "note"];
const TIMEOUT: Duration = Duration::from_secs(10);

/// What a template can say about one MR.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Fields {
    /// `!1797` on GitLab, `#12` on GitHub.
    pub(crate) reference: String,
    pub(crate) number: u64,
    pub(crate) title: String,
    pub(crate) url: String,
    pub(crate) author: String,
    pub(crate) project: String,
    pub(crate) branch: String,
}

impl Fields {
    pub(crate) fn from_queue(mr: &QueueMr, kind: Kind) -> Self {
        Self {
            reference: format!("{}{}", kind.sigil(), mr.number),
            number: mr.number,
            title: mr.title.clone(),
            url: mr.web_url.clone(),
            author: mr.author.clone(),
            project: mr.project.to_string(),
            branch: mr.source_branch.clone(),
        }
    }

    pub(crate) fn from_mr(mr: &Mr, kind: Kind) -> Self {
        Self {
            reference: format!("{}{}", kind.sigil(), mr.number),
            number: mr.number,
            title: mr.title.clone(),
            url: mr.web_url.clone(),
            author: mr.author.username.clone(),
            project: mr.project.to_string(),
            branch: mr.source_branch.clone(),
        }
    }
}

/// One place to post to: the command that posts and the message it gets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Target {
    /// `None` for the bare `[share]` target, the key of `[share.targets.<name>]` otherwise.
    pub(crate) name: Option<String>,
    pub(crate) command: String,
    pub(crate) template: String,
}

impl Target {
    /// What the toast says after a post: `shared !42`, or `shared !42 to review`.
    pub(crate) fn done(&self, reference: &str) -> String {
        match &self.name {
            Some(name) => format!("shared {reference} to {name}"),
            None => format!("shared {reference}"),
        }
    }
}

/// The target I last shared to, by host: its name, `None` for the bare `[share]` one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct LastShare(BTreeMap<String, Option<String>>);

impl LastShare {
    #[must_use]
    pub(crate) fn with(&self, host: &str, target: Option<&str>) -> Self {
        let mut hosts = self.0.clone();
        hosts.insert(host.to_owned(), target.map(str::to_owned));
        Self(hosts)
    }

    /// Where the last target of `host` sits in `targets`; the first one when it is unknown or gone.
    pub(crate) fn index_in(&self, host: &str, targets: &[Target]) -> usize {
        let Some(last) = self.0.get(host) else { return 0 };
        targets.iter().position(|target| target.name == *last).unwrap_or(0)
    }
}

/// The last targets kept in the cache; none when the file is missing or broken.
pub(crate) fn last_share(cache: &Cache) -> LastShare {
    cache.read(&keys::last_share()).unwrap_or_default()
}

/// Keeps `target` as the last one shared to on `host`, whatever other revu processes wrote meanwhile.
pub(crate) fn remember(cache: &Cache, host: &str, target: Option<&str>) -> Result<LastShare> {
    cache.update(&keys::last_share(), |last: LastShare| last.with(host, target))
}

/// The targets the config names, the bare one first, then named ones in name order.
pub(crate) fn targets(share: &config::Share) -> Vec<Target> {
    let fallback = share.template.as_deref().unwrap_or(DEFAULT_TEMPLATE);
    let bare = share.command.iter().map(|command| Target { name: None, command: command.clone(), template: fallback.to_owned() });
    let named = share.targets.iter().map(|(name, target)| Target {
        name: Some(name.clone()),
        command: target.command.clone(),
        template: target.template.clone().unwrap_or_else(|| fallback.to_owned()),
    });
    bare.chain(named).collect()
}

/// The message for `fields`; every line naming `{note}` goes when the note is empty, so an
/// optional `_{note}_` line never posts as `__`.
pub(crate) fn render(template: &str, fields: &Fields, note: &str) -> String {
    let note = note.trim();
    template
        .lines()
        .filter(|line| !(note.is_empty() && line.contains("{note}")))
        .map(|line| fill(line, fields, note))
        .collect::<Vec<_>>()
        .join("\n")
}

fn fill(line: &str, fields: &Fields, note: &str) -> String {
    line.replace("{ref}", &fields.reference)
        .replace("{iid}", &fields.number.to_string())
        .replace("{title}", &fields.title)
        .replace("{url}", &fields.url)
        .replace("{author}", &fields.author)
        .replace("{project}", &fields.project)
        .replace("{branch}", &fields.branch)
        .replace("{note}", note)
}

/// A `{word}` revu does not fill is a typo waiting to post as is: refused when the config loads.
/// Braces around anything else (JSON, a code sample) are left alone.
pub(crate) fn check_template(template: &str, key: &str) -> Result<()> {
    if template.trim().is_empty() {
        bail!("`{key}` is empty");
    }
    check_placeholders(template, key, &PLACEHOLDERS)
}

/// Every `{word}` of `template` is one of `known`; `key` names the setting in the error.
pub(crate) fn check_placeholders(template: &str, key: &str, known: &[&str]) -> Result<()> {
    for word in placeholders(template) {
        if !known.contains(&word) {
            let known = known.iter().map(|p| format!("{{{p}}}")).collect::<Vec<_>>().join(" ");
            bail!("`{key}` names `{{{word}}}`, which revu does not fill; the placeholders are {known}");
        }
    }
    Ok(())
}

pub(crate) fn placeholders(template: &str) -> impl Iterator<Item = &str> {
    template.split('{').skip(1).filter_map(|rest| {
        let word = rest.split_once('}')?.0;
        (!word.is_empty() && word.chars().all(|c| c.is_ascii_lowercase() || c == '_')).then_some(word)
    })
}

/// Pipes `message` to the target's command; its output is not read further than for errors.
pub(crate) async fn send(target: &Target, message: &str) -> Result<()> {
    crate::program::run(&target.command, Some(message), "share command", TIMEOUT).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ShareTarget;

    fn fields() -> Fields {
        Fields {
            reference: "!42".into(),
            number: 42,
            title: "charge cards at checkout".into(),
            url: "https://gitlab.com/acme/widgets/-/merge_requests/42".into(),
            author: "nina".into(),
            project: "acme/widgets".into(),
            branch: "feat/checkout".into(),
        }
    }

    #[test]
    fn the_default_template_links_the_mr_and_adds_the_note_in_italics() {
        let message = render(DEFAULT_TEMPLATE, &fields(), "needs a second look");
        assert_eq!(message, "[!42 charge cards at checkout](https://gitlab.com/acme/widgets/-/merge_requests/42)\n_needs a second look_");
    }

    #[test]
    fn an_empty_note_drops_every_line_that_names_it() {
        let template = "{project}: [{ref} {title}]({url})\n_{note}_\nby {author} on {branch} ({iid})\nps: {note}";
        assert_eq!(
            render(template, &fields(), "   "),
            "acme/widgets: [!42 charge cards at checkout](https://gitlab.com/acme/widgets/-/merge_requests/42)\nby nina on feat/checkout (42)"
        );
    }

    #[test]
    fn templates_name_only_known_placeholders() {
        assert!(check_template(DEFAULT_TEMPLATE, "share.template").is_ok());
        assert!(check_template(r#"{"text": "{ref} {title}"}"#, "share.template").is_ok(), "JSON braces are not placeholders");
        let err = check_template("[{ref} {titel}]({url})", "share.targets.review.template").unwrap_err().to_string();
        assert!(err.contains("share.targets.review.template") && err.contains("{titel}") && err.contains("{title}"), "{err}");
        assert!(check_template("  ", "share.template").is_err());
    }

    #[test]
    fn targets_list_the_bare_one_first_and_inherit_the_template() {
        let share = config::Share {
            command: Some("slack send '#tech-review'".into()),
            template: Some("{ref} {title}".into()),
            targets: BTreeMap::from([
                ("team".into(), ShareTarget { command: "slack send '#team'".into(), template: None }),
                ("hook".into(), ShareTarget { command: "hook".into(), template: Some("{url}".into()) }),
            ]),
        };
        let targets = targets(&share);
        let names: Vec<_> = targets.iter().map(|t| t.name.as_deref()).collect();
        assert_eq!(names, [None, Some("hook"), Some("team")]);
        assert_eq!(targets[2].template, "{ref} {title}", "a named target without a template takes [share] template");
        assert_eq!(targets[1].template, "{url}");
        assert_eq!(targets[0].done("!42"), "shared !42");
        assert_eq!(targets[2].done("!42"), "shared !42 to team");
        assert_eq!(super::targets(&config::Share::default()), [] as [Target; 0]);
    }

    fn two() -> Vec<Target> {
        let target =
            |name: Option<&str>| Target { name: name.map(str::to_owned), command: "cat".into(), template: DEFAULT_TEMPLATE.into() };
        vec![target(None), target(Some("team"))]
    }

    #[test]
    fn the_last_target_is_kept_per_host_and_picks_the_first_when_unknown() {
        let last = LastShare::default().with("gitlab.com", Some("team")).with("github.com", None);
        assert_eq!(last.index_in("gitlab.com", &two()), 1);
        assert_eq!(last.index_in("github.com", &two()), 0);
        assert_eq!(last.index_in("gitlab.example.org", &two()), 0, "a host with nothing shared yet");
        assert_eq!(LastShare::default().with("gitlab.com", Some("gone")).index_in("gitlab.com", &two()), 0, "a target since removed");
    }

    #[test]
    fn the_cache_keeps_the_last_target_and_a_missing_or_broken_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::in_dir(dir.path());
        assert_eq!(last_share(&cache), LastShare::default());
        remember(&cache, "gitlab.com", Some("team")).unwrap();
        assert_eq!(remember(&cache, "gitlab.com", Some("team")).unwrap(), last_share(&cache), "twice is once");
        assert_eq!(last_share(&cache).index_in("gitlab.com", &two()), 1);
        std::fs::write(dir.path().join(keys::last_share()), b"[not json").unwrap();
        assert_eq!(last_share(&cache), LastShare::default());
    }

    #[tokio::test]
    async fn send_pipes_the_message_and_reports_the_command_error() {
        let (dir, command) = crate::program::tests::script("cat > \"$(dirname \"$0\")/sent.txt\"");
        let target = Target { name: None, command, template: DEFAULT_TEMPLATE.into() };
        send(&target, "hello\n_world_").await.unwrap();
        assert_eq!(std::fs::read_to_string(dir.path().join("sent.txt")).unwrap(), "hello\n_world_");
        let (_dir, failing) = crate::program::tests::script("cat >/dev/null\necho 'not_in_channel' >&2\nexit 2");
        let err = send(&Target { command: failing, ..target }, "x").await.unwrap_err().to_string();
        assert!(err.contains("share command") && err.contains("not_in_channel"), "{err}");
    }
}
