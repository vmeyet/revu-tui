//! The little language the palette, the queue filter and saved views share:
//! `@loic ~infra slack` is Loïc's MRs labelled infra that mention slack.
//!
//! | Term | Keeps MRs |
//! |---|---|
//! | `word`, `"two words"` | whose title, author, project or branch hold it |
//! | `@name`, `@me` | by that author (several `@` are any of them) |
//! | `!123`, `#12` | of that number (several are any of them) |
//! | `~label`, `~"two words"` | carrying that label (several are all of them) |
//! | `draft:yes`, `draft:no` | draft, or not |
//! | `size:small`, `size:large` | up to 100 changed lines, or 500 and more |
//! | `is:failing` | whose pipeline failed or that conflict |
//! | `is:mine` | I wrote |
use crate::forge::QueueMr;

/// Up to this many changed lines is a small MR.
const SMALL: u32 = 100;
/// From this many changed lines on is a large MR.
const LARGE: u32 = 500;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub words: Vec<String>,
    authors: Vec<String>,
    numbers: Vec<u64>,
    labels: Vec<String>,
    draft: Option<bool>,
    size: Option<Size>,
    failing: bool,
    mine: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Size {
    Small,
    Large,
}

impl Query {
    /// Every term must make sense, or the error says which does not: saved views are checked this way.
    pub fn parse(text: &str) -> Result<Self, String> {
        tokens(text).into_iter().try_fold(Self::default(), |query, token| query.with(&token))
    }

    /// What is being typed: a term that does not make sense yet (`draft:`, a lone `@`) is left out.
    pub fn lenient(text: &str) -> Self {
        tokens(text).into_iter().fold(Self::default(), |query, token| query.clone().with(&token).unwrap_or(query))
    }

    /// The MR passes every term but the free words, which callers match their own way.
    pub fn keeps(&self, mr: &QueueMr, me: &str) -> bool {
        let author = |name: &String| {
            let name = if name == "me" { me.to_lowercase() } else { name.to_lowercase() };
            mr.author.to_lowercase().contains(&name) || mr.author_name.to_lowercase().contains(&name)
        };
        let label = |wanted: &String| mr.labels.iter().any(|l| l.to_lowercase().contains(wanted.as_str()));
        let size = mr.additions + mr.deletions;
        (self.authors.is_empty() || self.authors.iter().any(author))
            && (self.numbers.is_empty() || self.numbers.contains(&mr.number))
            && self.labels.iter().all(label)
            && self.draft.is_none_or(|draft| mr.draft == draft)
            && self.size.is_none_or(|wanted| match wanted {
                Size::Small => size <= SMALL,
                Size::Large => size >= LARGE,
            })
            && (!self.failing || mr.conflicts || mr.pipeline.as_deref().is_some_and(|p| p.eq_ignore_ascii_case("failed")))
            && (!self.mine || mr.author == me)
    }

    /// Every free word appears in the title, the author, the project or the branch.
    pub fn words_found(&self, mr: &QueueMr) -> bool {
        let haystack = format!("{} {} {} {} {}", mr.title, mr.author, mr.author_name, mr.project, mr.source_branch).to_lowercase();
        self.words.iter().all(|word| haystack.contains(word.as_str()))
    }

    /// The queue filter: every term, words included.
    pub fn matches(&self, mr: &QueueMr, me: &str) -> bool {
        self.keeps(mr, me) && self.words_found(mr)
    }

    fn with(mut self, token: &str) -> Result<Self, String> {
        let lower = token.to_lowercase();
        if let Some(name) = lower.strip_prefix('@') {
            return nonempty(name, token, "an author, like @nina or @me").map(|name| {
                self.authors.push(name);
                self
            });
        }
        if let Some(label) = lower.strip_prefix('~') {
            return nonempty(label, token, "a label, like ~infra").map(|label| {
                self.labels.push(label);
                self
            });
        }
        if let Some(number) = lower.strip_prefix(['!', '#']) {
            let number = number.parse().map_err(|_| format!("`{token}` is not a number, like !42"))?;
            self.numbers.push(number);
            return Ok(self);
        }
        match lower.split_once(':') {
            Some(("draft", value)) => {
                self.draft = Some(yes_no(value).ok_or_else(|| format!("`{token}`: draft: takes yes or no"))?);
            }
            Some(("size", "small")) => self.size = Some(Size::Small),
            Some(("size", "large")) => self.size = Some(Size::Large),
            Some(("size", _)) => return Err(format!("`{token}`: size: takes small or large")),
            Some(("is", "failing")) => self.failing = true,
            Some(("is", "mine")) => self.mine = true,
            Some(("is", _)) => return Err(format!("`{token}`: is: takes failing or mine")),
            _ => self.words.push(lower),
        }
        Ok(self)
    }
}

fn nonempty(value: &str, token: &str, what: &str) -> Result<String, String> {
    if value.is_empty() { Err(format!("`{token}` needs {what}")) } else { Ok(value.to_owned()) }
}

fn yes_no(value: &str) -> Option<bool> {
    match value {
        "yes" | "true" => Some(true),
        "no" | "false" => Some(false),
        _ => None,
    }
}

/// Whitespace-separated terms; double quotes keep spaces inside one (`~"needs review"`).
fn tokens(text: &str) -> Vec<String> {
    let mut tokens = vec![];
    let mut current = String::new();
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn mr(number: u64, author: &str, title: &str) -> QueueMr {
        let at = chrono::DateTime::parse_from_rfc3339("2026-09-20T10:00:00Z").unwrap().to_utc();
        QueueMr {
            host: None,
            number,
            project: "acme/widgets".into(),
            title: title.into(),
            description: String::new(),
            draft: false,
            web_url: String::new(),
            updated_at: at,
            created_at: at,
            source_branch: "feat/thing".into(),
            target_branch: "main".into(),
            conflicts: false,
            author: author.into(),
            author_name: author.to_uppercase(),
            approved: false,
            approved_by: vec![],
            approvals_left: None,
            reviewers: vec![],
            pipeline: None,
            additions: 10,
            deletions: 5,
            files: 1,
            unresolved: 0,
            labels: vec!["infra".into(), "needs review".into()],
            notes: 0,
            commenters: vec![],
            reason: None,
        }
    }

    #[test]
    fn terms_split_on_spaces_and_quotes_keep_them() {
        assert_eq!(tokens(r#"@loic ~"needs review"  slack "two words""#), ["@loic", "~needs review", "slack", "two words"]);
        assert!(tokens("   ").is_empty());
    }

    #[test]
    fn every_prefix_parses_and_combines() {
        let q = Query::parse("@Loic ~infra !42 #7 draft:no size:small is:failing is:mine Slack").unwrap();
        assert_eq!(q.authors, ["loic"]);
        assert_eq!(q.labels, ["infra"]);
        assert_eq!(q.numbers, [42, 7]);
        assert_eq!((q.draft, q.size, q.failing, q.mine), (Some(false), Some(Size::Small), true, true));
        assert_eq!(q.words, ["slack"]);
        assert_eq!(Query::parse("feat: slack").unwrap().words, ["feat:", "slack"], "a colon alone is a word, as in titles");
    }

    #[test]
    fn strict_parsing_names_the_term_that_makes_no_sense() {
        let cases = [
            ("@", "`@` needs an author, like @nina or @me"),
            ("~", "`~` needs a label, like ~infra"),
            ("!abc", "`!abc` is not a number, like !42"),
            ("draft:maybe", "`draft:maybe`: draft: takes yes or no"),
            ("size:huge", "`size:huge`: size: takes small or large"),
            ("is:weird", "`is:weird`: is: takes failing or mine"),
        ];
        for (text, error) in cases {
            assert_eq!(Query::parse(text), Err(error.to_owned()), "{text}");
        }
    }

    #[test]
    fn lenient_parsing_skips_what_is_still_being_typed() {
        assert_eq!(Query::lenient("@ ~infra draft:").labels, ["infra"]);
        assert_eq!(Query::lenient("draft:"), Query::default());
    }

    #[test]
    fn matching_follows_each_term() {
        let loic = mr(42, "loic", "feat(slack): batch the users sync");
        let nina = QueueMr { additions: 900, conflicts: true, draft: true, labels: vec![], ..mr(7, "nina", "fix: flaky cache") };
        let keeps = |text: &str, which: &QueueMr| Query::parse(text).unwrap().matches(which, "nina");
        assert!(keeps("@loic slack", &loic) && !keeps("@loic slack", &nina));
        assert!(keeps("@LOIC", &loic), "the display name counts too, case aside");
        assert!(keeps("@me", &nina) && keeps("is:mine", &nina) && !keeps("is:mine", &loic));
        assert!(keeps("@nina @loic", &loic), "several authors are any of them");
        assert!(keeps("~infra ~needs", &loic) && !keeps("~infra ~ops", &loic), "several labels are all of them");
        assert!(keeps("!42", &loic) && keeps("#7 !42", &nina) && !keeps("!8", &nina));
        assert!(keeps("draft:yes", &nina) && keeps("draft:no", &loic));
        assert!(keeps("size:small", &loic) && keeps("size:large", &nina) && !keeps("size:large", &loic));
        assert!(keeps("is:failing", &nina) && !keeps("is:failing", &loic));
        assert!(keeps("feat/thing", &loic), "the branch counts");
        assert!(keeps("", &loic));
    }
}
