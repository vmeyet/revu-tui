//! `Y`: post an MR somewhere by piping a message to a command of the user's choosing.
//! revu fills a template and hands the text to the command's stdin; it knows no chat tool.
use crate::config;
use crate::forge::{Kind, Mr, QueueMr};
use anyhow::{Result, bail};
use std::time::Duration;

/// A link titled with the MR's ref, then the note in italics when there is one.
pub const DEFAULT_TEMPLATE: &str = "[{ref} {title}]({url})\n_{note}_";

const PLACEHOLDERS: [&str; 8] = ["ref", "iid", "title", "url", "author", "project", "branch", "note"];
const TIMEOUT: Duration = Duration::from_secs(10);

/// What a template can say about one MR.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fields {
    /// `!1797` on GitLab, `#12` on GitHub.
    pub reference: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub author: String,
    pub project: String,
    pub branch: String,
}

impl Fields {
    pub fn from_queue(mr: &QueueMr, kind: Kind) -> Self {
        Self {
            reference: format!("{}{}", kind.sigil(), mr.number),
            number: mr.number,
            title: mr.title.clone(),
            url: mr.web_url.clone(),
            author: mr.author.clone(),
            project: mr.project.clone(),
            branch: mr.source_branch.clone(),
        }
    }

    pub fn from_mr(mr: &Mr, kind: Kind) -> Self {
        Self {
            reference: format!("{}{}", kind.sigil(), mr.number),
            number: mr.number,
            title: mr.title.clone(),
            url: mr.web_url.clone(),
            author: mr.author.username.clone(),
            project: mr.project.clone(),
            branch: mr.source_branch.clone(),
        }
    }
}

/// One place to post to: the command that posts and the message it gets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    /// `None` for the bare `[share]` target, the key of `[share.targets.<name>]` otherwise.
    pub name: Option<String>,
    pub command: String,
    pub template: String,
}

impl Target {
    /// What the toast says after a post: `shared !42`, or `shared !42 to review`.
    pub fn done(&self, reference: &str) -> String {
        match &self.name {
            Some(name) => format!("shared {reference} to {name}"),
            None => format!("shared {reference}"),
        }
    }
}

/// The targets the config names, the bare one first, then named ones in name order.
pub fn targets(share: &config::Share) -> Vec<Target> {
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
pub fn render(template: &str, fields: &Fields, note: &str) -> String {
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
pub fn check_template(template: &str, key: &str) -> Result<()> {
    if template.trim().is_empty() {
        bail!("`{key}` is empty");
    }
    for word in placeholders(template) {
        if !PLACEHOLDERS.contains(&word) {
            bail!("`{key}` names `{{{word}}}`, which revu does not fill; the placeholders are {}", known());
        }
    }
    Ok(())
}

fn placeholders(template: &str) -> impl Iterator<Item = &str> {
    template.split('{').skip(1).filter_map(|rest| {
        let word = rest.split_once('}')?.0;
        (!word.is_empty() && word.chars().all(|c| c.is_ascii_lowercase() || c == '_')).then_some(word)
    })
}

fn known() -> String {
    PLACEHOLDERS.iter().map(|p| format!("{{{p}}}")).collect::<Vec<_>>().join(" ")
}

/// Pipes `message` to the target's command; its output is not read further than for errors.
pub async fn send(target: &Target, message: &str) -> Result<()> {
    crate::program::run(&target.command, Some(message), "share command", TIMEOUT).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::config::ShareTarget;
    use std::collections::BTreeMap;

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
        assert!(super::targets(&config::Share::default()).is_empty());
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
