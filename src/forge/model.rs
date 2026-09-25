//! What the app knows about a merge request, whichever forge holds it. Each backend converts its
//! own wire shapes to these at its edge; nothing above `forge` sees a GitLab or GitHub field.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The one way an MR is addressed: the path of its project (`group/sub/project`, `owner/repo`)
/// and its number there.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MrKey {
    /// The host it lives on, when that is not the one `revu` started with: a queue can hold several.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    pub project: String,
    pub number: u64,
}

impl MrKey {
    pub fn new(project: impl Into<String>, number: u64) -> Self {
        Self { host: None, project: project.into(), number }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: u64,
    pub username: String,
    pub name: String,
}

/// One merge request, with what the header and the approvals need.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mr {
    pub project: String,
    pub number: u64,
    pub title: String,
    pub description: String,
    pub state: String,
    pub draft: bool,
    pub author: User,
    pub source_branch: String,
    pub target_branch: String,
    pub web_url: String,
    pub updated_at: DateTime<Utc>,
    pub refs: Refs,
    pub pipeline: Option<Pipeline>,
    /// As the forge counts it; GitLab caps it at `"1000+"`.
    pub changes_count: Option<String>,
    pub conflicts: bool,
    pub reviewers: Vec<User>,
    pub labels: Vec<String>,
    pub approvals: Approvals,
    /// How the forge merges it, from its project settings; the confirmation names the method.
    #[serde(default)]
    pub merge: MergePlan,
    /// I wrote it, as the host that holds it knows me: a queue can mix hosts, and names differ on each.
    #[serde(default)]
    pub mine: bool,
}

impl Mr {
    /// Why I may not merge this MR from revu, or `None` when I may: it is mine, open, not a draft,
    /// green, without conflicts, and approved by someone with no approval left to give.
    pub fn merge_refusal(&self) -> Option<String> {
        let approvals = &self.approvals;
        let reason = match () {
            () if !self.mine => "only your own MRs merge from revu".to_owned(),
            () if self.state != "opened" => format!("this MR is {}", self.state),
            () if self.draft => "a draft cannot merge: mark it ready first".to_owned(),
            () if self.conflicts => "it has conflicts with its target branch".to_owned(),
            () if self.pipeline.as_ref().is_some_and(|p| p.status == "failed") => "its pipeline failed".to_owned(),
            () if approvals.approved_by.is_empty() => "nobody approved it yet".to_owned(),
            () if approvals.approvals_left > 0 => {
                format!("it needs {} more approval{}", approvals.approvals_left, if approvals.approvals_left == 1 { "" } else { "s" })
            }
            () => return None,
        };
        Some(reason)
    }

    /// Why I may not mark this MR a draft or ready from revu, or `None` when I may: it is mine and open.
    pub fn draft_refusal(&self) -> Option<String> {
        match () {
            () if !self.mine => Some("it is not yours".to_owned()),
            () if self.state != "opened" => Some(format!("this MR is {}", self.state)),
            () => None,
        }
    }
}

/// How a forge merges an MR, as its project allows and prefers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeMethod {
    #[default]
    Merge,
    Squash,
    Rebase,
}

impl MergeMethod {
    pub fn word(self) -> &'static str {
        match self {
            MergeMethod::Merge => "merge commit",
            MergeMethod::Squash => "squash",
            MergeMethod::Rebase => "rebase",
        }
    }
}

/// The method, and whether the forge deletes the source branch afterwards.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergePlan {
    pub method: MergeMethod,
    pub remove_branch: bool,
}

/// The diff a review reads: its base, where the branch started (GitLab tells it apart from the
/// base; elsewhere it is the base), and its head. Notes on lines are made against these.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refs {
    pub base: String,
    pub start: String,
    pub head: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pipeline {
    pub status: String,
    pub web_url: Option<String>,
}

/// Where the MR's branch was deployed and can be tried: a review app, a preview, a storybook.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deployment {
    pub environment: String,
    pub url: String,
    /// The commit deployed, which may be older than the MR's head.
    pub sha: String,
}

impl Deployment {
    /// The first of each environment, from deployments listed newest first.
    pub fn newest_each(deployments: impl IntoIterator<Item = Deployment>) -> Vec<Deployment> {
        let mut kept: Vec<Deployment> = vec![];
        for deployment in deployments {
            if kept.iter().all(|k| k.environment != deployment.environment) {
                kept.push(deployment);
            }
        }
        kept
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approvals {
    pub approved: bool,
    pub approvals_left: u32,
    pub user_has_approved: bool,
    pub user_can_approve: bool,
    pub approved_by: Vec<User>,
}

/// One changed file: its unified diff body (hunks from the first `@@`) and how it changed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffFile {
    pub diff: String,
    pub old_path: String,
    pub new_path: String,
    /// File modes when the forge gives them; a mode-only change reads as such.
    #[serde(default)]
    pub a_mode: String,
    #[serde(default)]
    pub b_mode: String,
    #[serde(default)]
    pub new_file: bool,
    #[serde(default)]
    pub renamed_file: bool,
    #[serde(default)]
    pub deleted_file: bool,
    #[serde(default)]
    pub generated_file: bool,
    /// The forge withheld the body: too many lines for it to send.
    #[serde(default)]
    pub too_large: bool,
    #[serde(default)]
    pub collapsed: bool,
}

/// A thread as the forge holds it: its notes in order, the first one carrying where it hangs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Discussion {
    pub id: String,
    pub notes: Vec<Note>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub id: u64,
    pub body: String,
    pub author: User,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Written by the forge itself: "added 2 commits", "approved this merge request".
    pub system: bool,
    pub resolvable: bool,
    pub resolved: bool,
    pub position: Option<Position>,
    /// The suggestions the forge can apply for us, by id; GitHub has none, its suggestions live in the text only.
    #[serde(default)]
    pub suggestions: Vec<Applicable>,
    /// The reactions on it, one entry per emoji that has any.
    #[serde(default)]
    pub reactions: Vec<Reaction>,
    /// The forge's GraphQL id for the note, which reacting needs: `gid://gitlab/DiffNote/…`, GitHub's node id.
    #[serde(default)]
    pub node: Option<String>,
}

/// A reaction: one of the eight both forges share (GitHub's, which GitLab has under other names),
/// or any other emoji, which only GitLab takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Emoji {
    ThumbsUp,
    ThumbsDown,
    Laugh,
    Confused,
    Heart,
    Hooray,
    Rocket,
    Eyes,
    Other(&'static emojis::Emoji),
}

impl Emoji {
    pub const ALL: [Emoji; 8] =
        [Emoji::ThumbsUp, Emoji::ThumbsDown, Emoji::Laugh, Emoji::Confused, Emoji::Heart, Emoji::Hooray, Emoji::Rocket, Emoji::Eyes];

    /// The picture a terminal with emoji shows.
    pub fn glyph(self) -> &'static str {
        match self {
            Emoji::ThumbsUp => "👍",
            Emoji::ThumbsDown => "👎",
            Emoji::Laugh => "😄",
            Emoji::Confused => "😕",
            Emoji::Heart => "💖",
            Emoji::Hooray => "🎉",
            Emoji::Rocket => "🚀",
            Emoji::Eyes => "👀",
            Emoji::Other(emoji) => emoji.as_str(),
        }
    }

    /// The same in plain text, for terminals that draw emoji at the wrong width.
    pub fn text(self) -> &'static str {
        match self {
            Emoji::ThumbsUp => "+1",
            Emoji::ThumbsDown => "-1",
            Emoji::Laugh => ":D",
            Emoji::Confused => ":/",
            Emoji::Heart => "<3",
            Emoji::Hooray => "\\o/",
            Emoji::Rocket => "rocket",
            Emoji::Eyes => "eyes",
            Emoji::Other(emoji) => emoji.shortcode().unwrap_or(emoji.name()),
        }
    }

    /// GitLab's award name.
    pub fn gitlab(self) -> &'static str {
        match self {
            Emoji::ThumbsUp => "thumbsup",
            Emoji::ThumbsDown => "thumbsdown",
            Emoji::Laugh => "laughing",
            Emoji::Confused => "confused",
            Emoji::Heart => "heart",
            Emoji::Hooray => "tada",
            Emoji::Rocket => "rocket",
            Emoji::Eyes => "eyes",
            Emoji::Other(emoji) => emoji.shortcode().unwrap_or(emoji.name()),
        }
    }

    /// GitHub's reaction content, as GraphQL spells it; GitHub has none for the others.
    pub fn github(self) -> Option<&'static str> {
        Some(match self {
            Emoji::ThumbsUp => "THUMBS_UP",
            Emoji::ThumbsDown => "THUMBS_DOWN",
            Emoji::Laugh => "LAUGH",
            Emoji::Confused => "CONFUSED",
            Emoji::Heart => "HEART",
            Emoji::Hooray => "HOORAY",
            Emoji::Rocket => "ROCKET",
            Emoji::Eyes => "EYES",
            Emoji::Other(_) => return None,
        })
    }

    /// The emoji a GitHub content stands for, one of the eight.
    pub fn from_github(content: &str) -> Option<Emoji> {
        Emoji::ALL.into_iter().find(|e| e.github() == Some(content))
    }

    /// The emoji a GitLab award name stands for: one of the eight by its GitLab name, else any
    /// emoji by its short code; GitLab's custom emoji are left out.
    pub fn from_gitlab(name: &str) -> Option<Emoji> {
        Emoji::ALL.into_iter().find(|e| e.gitlab() == name).or_else(|| emojis::get_by_shortcode(name).map(Emoji::Other))
    }

    /// Every emoji outside the eight, for a forge that takes any.
    pub fn others() -> impl Iterator<Item = Emoji> {
        emojis::iter().filter(|e| e.skin_tone().is_none_or(|t| t == emojis::SkinTone::Default)).map(Emoji::Other).filter(|e| {
            let name = e.gitlab();
            Emoji::ALL.iter().all(|known| known.gitlab() != name)
        })
    }
}

/// Stored by name, the eight as before and the others by their picture, so a cached reaction reads back.
impl Serialize for Emoji {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Emoji::Other(emoji) => serializer.serialize_str(emoji.as_str()),
            known => serializer.serialize_str(&format!("{known:?}")),
        }
    }
}

impl<'de> Deserialize<'de> for Emoji {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Emoji::ALL
            .into_iter()
            .find(|known| format!("{known:?}") == text)
            .or_else(|| emojis::get(&text).map(Emoji::Other))
            .ok_or_else(|| serde::de::Error::custom(format!("`{text}` is not an emoji")))
    }
}

/// How many people reacted with `emoji`, me among them or not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reaction {
    pub emoji: Emoji,
    pub count: u32,
    pub mine: bool,
}

/// Reactions tallied from `(emoji name, was it me)` pairs, in the order of `Emoji::ALL`.
pub fn tally(awards: impl IntoIterator<Item = (Emoji, bool)>) -> Vec<Reaction> {
    let mut counts = std::collections::BTreeMap::<Emoji, (u32, bool)>::new();
    for (emoji, mine) in awards {
        let entry = counts.entry(emoji).or_default();
        entry.0 += 1;
        entry.1 |= mine;
    }
    counts.into_iter().map(|(emoji, (count, mine))| Reaction { emoji, count, mine }).collect()
}

/// A note with my reaction `emoji` added (`on`) or taken off, counts kept right.
pub fn toggled(reactions: &[Reaction], emoji: Emoji, on: bool) -> Vec<Reaction> {
    let mut all: Vec<Reaction> = reactions.to_vec();
    match all.iter_mut().find(|r| r.emoji == emoji) {
        Some(r) if on && !r.mine => *r = Reaction { count: r.count + 1, mine: true, ..*r },
        Some(r) if !on && r.mine => *r = Reaction { count: r.count.saturating_sub(1), mine: false, ..*r },
        Some(_) => {}
        None if on => all.push(Reaction { emoji, count: 1, mine: true }),
        None => {}
    }
    all.retain(|r| r.count > 0);
    all.sort_by_key(|r| r.emoji);
    all
}

/// A suggestion to commit on the MR's branch: GitLab applies it by `id`; elsewhere the lines around
/// `line` of `path` are replaced by `text` (`above` lines before it, `below` after).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub id: Option<u64>,
    pub path: String,
    pub line: u32,
    pub above: u32,
    pub below: u32,
    pub text: String,
}

/// A suggestion the forge applies itself when asked by its id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Applicable {
    pub id: u64,
    pub applied: bool,
    /// False once the lines moved under it or the MR closed.
    pub appliable: bool,
}

/// Which side of the diff a line number counts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Side {
    Old,
    New,
}

/// One diff line by its numbers: a removed line has only `old`, an added one only `new`, a
/// context line both.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineRef {
    pub old: Option<u32>,
    pub new: Option<u32>,
}

impl LineRef {
    /// The side a note on this line hangs on: new whenever the line exists there.
    pub fn side(self) -> Side {
        if self.new.is_some() { Side::New } else { Side::Old }
    }

    /// The number on `side()`.
    pub fn number(self) -> Option<u32> {
        self.new.or(self.old)
    }
}

/// Where a note hangs in the diff: one line, or the lines from `start` to `line`, in one file.
/// Each backend builds its own wire shape from this (GitLab's SHAs and `line_range`, GitHub's
/// `side`/`line`/`commit_id`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub refs: Refs,
    pub old_path: String,
    pub new_path: String,
    /// The line the note sits on: the last line of a range.
    pub line: LineRef,
    /// The first line of a range; `None` for a note on one line.
    pub start: Option<LineRef>,
}

impl Position {
    /// The path of the file on the side the note hangs on.
    pub fn path(&self) -> &str {
        match self.line.side() {
            Side::New => &self.new_path,
            Side::Old => &self.old_path,
        }
    }
}

/// A note of mine the forge holds but has not published yet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    pub id: u64,
    pub body: String,
    /// On a line; `None` on the MR itself or in a reply.
    pub position: Option<Position>,
    /// The thread it answers.
    pub reply_to: Option<String>,
    /// Publishing it also resolves the thread it answers.
    pub resolve: bool,
}

/// What a forge needs to hold a new draft, or to replace one whole.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewDraft {
    pub body: String,
    pub position: Option<Position>,
    pub reply_to: Option<String>,
    pub resolve: bool,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn a_line_hangs_on_the_new_side_when_it_exists_there() {
        let context = LineRef { old: Some(12), new: Some(12) };
        let removed = LineRef { old: Some(13), new: None };
        let added = LineRef { old: None, new: Some(14) };
        assert_eq!((context.side(), context.number()), (Side::New, Some(12)));
        assert_eq!((removed.side(), removed.number()), (Side::Old, Some(13)));
        assert_eq!((added.side(), added.number()), (Side::New, Some(14)));
    }

    #[test]
    fn any_gitlab_award_reads_as_an_emoji_that_github_cannot_take_and_caches_by_picture() {
        let hundred = Emoji::from_gitlab("100").unwrap();
        assert_eq!((hundred.glyph(), hundred.gitlab(), hundred.github()), ("💯", "100", None));
        assert_eq!(Emoji::from_gitlab("my_team_logo"), None, "a custom emoji has no picture");
        assert_eq!(Emoji::from_github("100"), None);
        assert!(Emoji::others().all(|e| !Emoji::ALL.contains(&e)), "the eight come once");
        assert!(!Emoji::others().any(|e| e.gitlab() == "tada"), "tada is Hooray");
        let saved = serde_json::to_string(&[Emoji::Rocket, hundred]).unwrap();
        assert_eq!(saved, r#"["Rocket","💯"]"#);
        assert_eq!(serde_json::from_str::<Vec<Emoji>>(&saved).unwrap(), [Emoji::Rocket, hundred]);
    }

    #[test]
    fn the_eight_reactions_have_a_name_on_each_forge_and_toggle_their_count() {
        for emoji in Emoji::ALL {
            assert_eq!(Emoji::from_gitlab(emoji.gitlab()), Some(emoji));
            assert_eq!(emoji.github().and_then(Emoji::from_github), Some(emoji));
        }
        let now = tally([(Emoji::Rocket, false), (Emoji::ThumbsUp, true), (Emoji::ThumbsUp, false)]);
        assert_eq!(
            now,
            [Reaction { emoji: Emoji::ThumbsUp, count: 2, mine: true }, Reaction { emoji: Emoji::Rocket, count: 1, mine: false }]
        );
        let off = toggled(&now, Emoji::ThumbsUp, false);
        assert_eq!(off[0], Reaction { emoji: Emoji::ThumbsUp, count: 1, mine: false });
        let on = toggled(&now, Emoji::Eyes, true);
        assert_eq!(on.last().copied(), Some(Reaction { emoji: Emoji::Eyes, count: 1, mine: true }));
        assert_eq!(toggled(&on, Emoji::Eyes, false), now, "on then off gives back what was there");
        assert_eq!(toggled(&now, Emoji::Rocket, false), now, "taking off a reaction I never gave changes nothing");
    }

    fn mergeable() -> Mr {
        let lea = User { id: 3, username: "lea".into(), name: "Léa".into() };
        Mr {
            project: "acme/widgets".into(),
            number: 42,
            title: "feat: charge cards".into(),
            description: String::new(),
            state: "opened".into(),
            draft: false,
            author: User { id: 5, username: "nina".into(), name: "Nina".into() },
            source_branch: "feat/checkout".into(),
            target_branch: "main".into(),
            web_url: String::new(),
            updated_at: DateTime::default(),
            refs: Refs { base: "a".into(), start: "a".into(), head: "b".into() },
            pipeline: Some(Pipeline { status: "success".into(), web_url: None }),
            changes_count: None,
            conflicts: false,
            reviewers: vec![],
            labels: vec![],
            approvals: Approvals { approved: true, approved_by: vec![lea], ..Approvals::default() },
            merge: MergePlan::default(),
            mine: true,
        }
    }

    #[test]
    fn only_my_open_green_approved_mrs_may_merge_and_each_refusal_says_why() {
        assert_eq!(mergeable().merge_refusal(), None);
        let cases: [(Mr, &str); 7] = [
            (Mr { mine: false, ..mergeable() }, "only your own MRs merge from revu"),
            (Mr { state: "merged".into(), ..mergeable() }, "this MR is merged"),
            (Mr { draft: true, ..mergeable() }, "a draft cannot merge: mark it ready first"),
            (Mr { conflicts: true, ..mergeable() }, "it has conflicts with its target branch"),
            (Mr { pipeline: Some(Pipeline { status: "failed".into(), web_url: None }), ..mergeable() }, "its pipeline failed"),
            (Mr { approvals: Approvals { approved: true, ..Approvals::default() }, ..mergeable() }, "nobody approved it yet"),
            (Mr { approvals: Approvals { approvals_left: 2, ..mergeable().approvals }, ..mergeable() }, "it needs 2 more approvals"),
        ];
        for (mr, reason) in cases {
            assert_eq!(mr.merge_refusal().as_deref(), Some(reason));
        }
        let running = Mr { pipeline: Some(Pipeline { status: "running".into(), web_url: None }), ..mergeable() };
        assert_eq!(running.merge_refusal(), None, "a running pipeline is the forge's call: it may merge when it passes");
    }

    #[test]
    fn only_my_open_mrs_turn_draft_or_ready() {
        assert_eq!(mergeable().draft_refusal(), None);
        assert_eq!(Mr { draft: true, ..mergeable() }.draft_refusal(), None);
        assert_eq!(Mr { mine: false, ..mergeable() }.draft_refusal().as_deref(), Some("it is not yours"));
        assert_eq!(Mr { state: "closed".into(), ..mergeable() }.draft_refusal().as_deref(), Some("this MR is closed"));
    }
}
