//! Emoji reactions on a note, and how mine toggles their count.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReactionChange {
    Add,
    Remove,
}

impl ReactionChange {
    pub(crate) fn undone(self) -> Self {
        match self {
            Self::Add => Self::Remove,
            Self::Remove => Self::Add,
        }
    }
}

/// A reaction: one of the eight both forges share (GitHub's, which GitLab has under other names),
/// or any other emoji, which only GitLab takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Emoji {
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
    pub(crate) const ALL: [Emoji; 8] =
        [Emoji::ThumbsUp, Emoji::ThumbsDown, Emoji::Laugh, Emoji::Confused, Emoji::Heart, Emoji::Hooray, Emoji::Rocket, Emoji::Eyes];

    /// The picture a terminal with emoji shows.
    pub(crate) fn glyph(self) -> &'static str {
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
    pub(crate) fn text(self) -> &'static str {
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
    pub(crate) fn gitlab(self) -> &'static str {
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
    pub(crate) fn github(self) -> Option<&'static str> {
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
    pub(in crate::forge) fn from_github(content: &str) -> Option<Emoji> {
        Emoji::ALL.into_iter().find(|e| e.github() == Some(content))
    }

    /// The emoji a GitLab award name stands for: one of the eight by its GitLab name, else any
    /// emoji by its short code; GitLab's custom emoji are left out.
    pub(crate) fn from_gitlab(name: &str) -> Option<Emoji> {
        Emoji::ALL.into_iter().find(|e| e.gitlab() == name).or_else(|| emojis::get_by_shortcode(name).map(Emoji::Other))
    }

    /// Every emoji outside the eight, for a forge that takes any.
    pub(crate) fn others() -> impl Iterator<Item = Emoji> {
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
pub(crate) struct Reaction {
    pub(crate) emoji: Emoji,
    pub(crate) count: u32,
    pub(crate) mine: bool,
}

/// Reactions tallied from `(emoji name, was it me)` pairs, in the order of `Emoji::ALL`.
pub(crate) fn tally(awards: impl IntoIterator<Item = (Emoji, bool)>) -> Vec<Reaction> {
    let mut counts = std::collections::BTreeMap::<Emoji, (u32, bool)>::new();
    for (emoji, mine) in awards {
        let entry = counts.entry(emoji).or_default();
        entry.0 += 1;
        entry.1 |= mine;
    }
    counts.into_iter().map(|(emoji, (count, mine))| Reaction { emoji, count, mine }).collect()
}

/// A note with my reaction `emoji` added or taken off, counts kept right.
pub(crate) fn toggled(reactions: &[Reaction], emoji: Emoji, change: ReactionChange) -> Vec<Reaction> {
    let mut all: Vec<Reaction> = reactions.to_vec();
    match (change, all.iter_mut().find(|r| r.emoji == emoji)) {
        (ReactionChange::Add, Some(r)) if !r.mine => *r = Reaction { count: r.count + 1, mine: true, ..*r },
        (ReactionChange::Remove, Some(r)) if r.mine => *r = Reaction { count: r.count.saturating_sub(1), mine: false, ..*r },
        (ReactionChange::Add, None) => all.push(Reaction { emoji, count: 1, mine: true }),
        _ => {}
    }
    all.retain(|r| r.count > 0);
    all.sort_by_key(|r| r.emoji);
    all
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let off = toggled(&now, Emoji::ThumbsUp, ReactionChange::Remove);
        assert_eq!(off[0], Reaction { emoji: Emoji::ThumbsUp, count: 1, mine: false });
        let on = toggled(&now, Emoji::Eyes, ReactionChange::Add);
        assert_eq!(on.last().copied(), Some(Reaction { emoji: Emoji::Eyes, count: 1, mine: true }));
        assert_eq!(toggled(&on, Emoji::Eyes, ReactionChange::Remove), now, "on then off gives back what was there");
        assert_eq!(toggled(&now, Emoji::Rocket, ReactionChange::Remove), now, "taking off a reaction I never gave changes nothing");
    }
}
