//! GitLab's JSON as it comes and goes, and its conversion to and from the neutral model.
use crate::forge::{self, LineRef, Refs};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct User {
    pub id: u64,
    pub username: String,
    pub name: String,
}

impl From<User> for forge::User {
    fn from(user: User) -> Self {
        Self { id: user.id, username: user.username, name: user.name }
    }
}

/// One merge request as `GET /projects/:id/merge_requests/:iid` returns it, plus its approvals.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Mr {
    pub iid: u64,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    pub state: String,
    pub draft: bool,
    pub author: User,
    pub source_branch: String,
    pub target_branch: String,
    pub web_url: String,
    pub updated_at: DateTime<Utc>,
    pub diff_refs: DiffRefs,
    #[serde(default)]
    pub head_pipeline: Option<Pipeline>,
    /// GitLab sends this count as a string, `"9"` or `"1000+"`.
    #[serde(default)]
    pub changes_count: Option<String>,
    #[serde(default)]
    pub has_conflicts: bool,
    #[serde(default)]
    pub reviewers: Vec<User>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub approvals: Approvals,
}

impl Mr {
    /// The REST answer does not name its project's path; the caller asked by it and knows it.
    pub fn into_model(self, project: &str) -> forge::Mr {
        forge::Mr {
            project: project.to_owned(),
            number: self.iid,
            title: self.title,
            description: self.description.unwrap_or_default(),
            state: self.state,
            draft: self.draft,
            author: self.author.into(),
            source_branch: self.source_branch,
            target_branch: self.target_branch,
            web_url: self.web_url,
            updated_at: self.updated_at,
            refs: self.diff_refs.into(),
            pipeline: self.head_pipeline.map(|p| forge::Pipeline { status: p.status, web_url: p.web_url }),
            changes_count: self.changes_count,
            conflicts: self.has_conflicts,
            reviewers: self.reviewers.into_iter().map(forge::User::from).collect(),
            labels: self.labels,
            approvals: self.approvals.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffRefs {
    pub base_sha: String,
    pub head_sha: String,
    pub start_sha: String,
}

impl From<DiffRefs> for Refs {
    fn from(refs: DiffRefs) -> Self {
        Self { base: refs.base_sha, start: refs.start_sha, head: refs.head_sha }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Pipeline {
    pub status: String,
    #[serde(default)]
    pub web_url: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Approvals {
    pub approved: bool,
    #[serde(default)]
    pub approvals_left: u32,
    #[serde(default)]
    pub user_has_approved: bool,
    #[serde(default)]
    pub user_can_approve: bool,
    #[serde(default, deserialize_with = "nested_users")]
    pub approved_by: Vec<User>,
}

impl From<Approvals> for forge::Approvals {
    fn from(a: Approvals) -> Self {
        Self {
            approved: a.approved,
            approvals_left: a.approvals_left,
            user_has_approved: a.user_has_approved,
            user_can_approve: a.user_can_approve,
            approved_by: a.approved_by.into_iter().map(forge::User::from).collect(),
        }
    }
}

fn nested_users<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<User>, D::Error> {
    #[derive(Deserialize)]
    struct Approver {
        user: User,
    }
    Ok(Vec::<Approver>::deserialize(d)?.into_iter().map(|a| a.user).collect())
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Discussion {
    pub id: String,
    pub notes: Vec<Note>,
}

impl From<Discussion> for forge::Discussion {
    fn from(d: Discussion) -> Self {
        Self { id: d.id, notes: d.notes.into_iter().map(forge::Note::from).collect() }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Note {
    pub id: u64,
    pub body: String,
    pub author: User,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub system: bool,
    #[serde(default)]
    pub resolvable: bool,
    #[serde(default)]
    pub resolved: Option<bool>,
    #[serde(default)]
    pub position: Option<Position>,
}

impl From<Note> for forge::Note {
    fn from(n: Note) -> Self {
        Self {
            id: n.id,
            body: n.body,
            author: n.author.into(),
            created_at: n.created_at,
            updated_at: n.updated_at,
            system: n.system,
            resolvable: n.resolvable,
            resolved: n.resolved.unwrap_or(false),
            position: n.position.and_then(Position::into_model),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub base_sha: String,
    pub head_sha: String,
    pub start_sha: String,
    pub position_type: String,
    #[serde(default)]
    pub old_path: Option<String>,
    #[serde(default)]
    pub new_path: Option<String>,
    #[serde(default)]
    pub old_line: Option<u32>,
    #[serde(default)]
    pub new_line: Option<u32>,
    #[serde(default)]
    pub line_range: Option<LineRange>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineRange {
    pub start: LineCode,
    pub end: LineCode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineCode {
    #[serde(default)]
    pub line_code: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub old_line: Option<u32>,
    #[serde(default)]
    pub new_line: Option<u32>,
}

impl Position {
    /// Only a note on text lines hangs in the diff; one on an image is a note on the MR here.
    fn into_model(self) -> Option<forge::Position> {
        if self.position_type != "text" {
            return None;
        }
        let line = LineRef { old: self.old_line, new: self.new_line };
        line.number()?;
        let new_path = self.new_path.clone().or_else(|| self.old_path.clone())?;
        Some(forge::Position {
            refs: Refs { base: self.base_sha, start: self.start_sha, head: self.head_sha },
            old_path: self.old_path.unwrap_or_else(|| new_path.clone()),
            new_path,
            line,
            start: self.line_range.map(|range| LineRef { old: range.start.old_line, new: range.start.new_line }),
        })
    }

    /// GitLab's shape for a neutral position: the three SHAs, both paths, and for a range the
    /// `line_code` of each edge.
    pub fn from_model(position: &forge::Position) -> Self {
        Self {
            base_sha: position.refs.base.clone(),
            head_sha: position.refs.head.clone(),
            start_sha: position.refs.start.clone(),
            position_type: "text".into(),
            old_path: Some(position.old_path.clone()),
            new_path: Some(position.new_path.clone()),
            old_line: position.line.old,
            new_line: position.line.new,
            line_range: position.start.map(|start| LineRange { start: code_of(position, start), end: code_of(position, position.line) }),
        }
    }
}

/// One edge of a range: its `line_code`, and `type` "old" for a removed line, "new" for an added one.
fn code_of(position: &forge::Position, line: LineRef) -> LineCode {
    let (path, kind) = match (line.old, line.new) {
        (Some(_), None) => (&position.old_path, Some("old".into())),
        (None, Some(_)) => (&position.new_path, Some("new".into())),
        _ => (&position.new_path, None),
    };
    LineCode { line_code: Some(line_code(path, line.old, line.new)), kind, old_line: line.old, new_line: line.new }
}

/// `sha1(path)_old_new`, GitLab's name for one diff line; a missing side is `0`.
pub fn line_code(path: &str, old: Option<u32>, new: Option<u32>) -> String {
    let digest = sha1_smol::Sha1::from(path.as_bytes()).digest().to_string();
    format!("{digest}_{}_{}", old.unwrap_or(0), new.unwrap_or(0))
}

/// One of my unpublished review comments, as `GET …/draft_notes` returns it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct DraftNote {
    pub id: u64,
    pub note: String,
    #[serde(default)]
    pub discussion_id: Option<String>,
    #[serde(default)]
    pub resolve_discussion: bool,
    #[serde(default)]
    pub line_code: Option<String>,
    /// GitLab sends a position of nulls for a note on the MR itself; that reads as `None`.
    #[serde(default, deserialize_with = "anchored_position")]
    pub position: Option<Position>,
}

impl From<DraftNote> for forge::Draft {
    fn from(d: DraftNote) -> Self {
        Self {
            id: d.id,
            body: d.note,
            position: d.position.and_then(Position::into_model),
            reply_to: d.discussion_id,
            resolve: d.resolve_discussion,
        }
    }
}

fn anchored_position<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Position>, D::Error> {
    let raw = serde_json::Value::deserialize(d)?;
    if raw.get("head_sha").is_none_or(serde_json::Value::is_null) {
        return Ok(None);
    }
    serde_json::from_value(raw).map(Some).map_err(serde::de::Error::custom)
}

/// What `POST …/draft_notes` needs; `position` anchors it to a line, `in_reply_to_discussion_id` to a thread.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct NewDraft {
    pub note: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_reply_to_discussion_id: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub resolve_discussion: bool,
}

impl From<&forge::NewDraft> for NewDraft {
    fn from(draft: &forge::NewDraft) -> Self {
        Self {
            note: draft.body.clone(),
            position: draft.position.as_ref().map(Position::from_model),
            in_reply_to_discussion_id: draft.reply_to.clone(),
            resolve_discussion: draft.resolve,
        }
    }
}

/// GitLab answers errors as `{"message": …}` or `{"error": …}`; anything else is shown as is.
pub fn error_message(body: &str) -> String {
    let parsed: Option<serde_json::Value> = serde_json::from_str(body).ok();
    let found = parsed.as_ref().and_then(|v| v.get("message").or_else(|| v.get("error")));
    match found {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => body.trim().to_owned(),
    }
}

/// GitLab answers turned into the neutral model, for tests anywhere in the crate.
#[cfg(test)]
#[allow(clippy::expect_used)]
pub(crate) mod fixture {
    use super::{super::graphql, Discussion, Mr};
    use crate::forge::{self, MrKey, Queue};
    use serde::de::DeserializeOwned;

    pub fn parse<T: DeserializeOwned>(json: &str) -> T {
        serde_json::from_str(json).expect("fixture parses")
    }

    /// An MR from its REST answer; the project path comes from its `web_url`.
    pub fn mr(json: &str) -> forge::Mr {
        let wire: Mr = parse(json);
        let project = wire.web_url.split("/-/merge_requests/").next().and_then(|u| u.splitn(4, '/').nth(3)).unwrap_or("").to_owned();
        wire.into_model(&project)
    }

    pub fn discussion(json: &str) -> forge::Discussion {
        parse::<Discussion>(json).into()
    }

    /// A queue from a GraphQL answer body.
    pub fn queue(body: &str) -> Queue {
        graphql::queue_from_json(body, None).expect("queue fixture parses")
    }

    /// The same, scoped to `project`: one body carries both answers, `currentUser` and `project`.
    pub fn queue_in(body: &str, project: &str) -> Queue {
        graphql::queue_from_json(body, Some(project)).expect("queue fixture parses")
    }

    /// The MR every fixture talks about.
    pub fn key() -> MrKey {
        MrKey::new("acme/widgets", 42)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::fixture::{discussion, parse};
    use super::*;

    fn sha(path: &str) -> String {
        sha1_smol::Sha1::from(path.as_bytes()).digest().to_string()
    }

    fn refs() -> Refs {
        Refs { base: "a".into(), start: "a".into(), head: "b".into() }
    }

    fn at(line: LineRef, start: Option<LineRef>) -> forge::Position {
        forge::Position { refs: refs(), old_path: "src/pay/charge.rs".into(), new_path: "src/pay/charge.rs".into(), line, start }
    }

    #[test]
    fn error_message_reads_both_shapes() {
        assert_eq!(error_message(r#"{"message":"401 Unauthorized"}"#), "401 Unauthorized");
        assert_eq!(error_message(r#"{"error":"invalid_token"}"#), "invalid_token");
        assert_eq!(error_message("<html>gateway</html>"), "<html>gateway</html>");
    }

    #[test]
    fn approvals_flatten_the_nested_users() {
        let approvals: Approvals = parse(
            r#"{"approved": true, "approvals_left": 0, "approved_by": [{"user": {"id": 2, "username": "nina", "name": "Nina"}, "approved_at": "2026-09-22T10:00:00Z"}]}"#,
        );
        let approvals = forge::Approvals::from(approvals);
        assert_eq!(approvals.approved_by.iter().map(|u| u.username.as_str()).collect::<Vec<_>>(), ["nina"]);
    }

    #[test]
    fn draft_note_with_a_null_position_is_unanchored() {
        let drafts: Vec<DraftNote> = parse(include_str!("fixtures/draft_notes.json"));
        assert_eq!(drafts[1].line_code.as_deref(), Some("2f1d_0_13"));
        let drafts: Vec<forge::Draft> = drafts.into_iter().map(forge::Draft::from).collect();
        assert_eq!(drafts[0].position, None);
        assert_eq!(drafts[0].reply_to.as_deref(), Some("6a9c1750"));
        let anchored = drafts[1].position.as_ref().unwrap();
        assert_eq!((anchored.new_path.as_str(), anchored.line.new), ("src/pay/charge.rs", Some(13)));
    }

    #[test]
    fn new_draft_serialises_only_what_is_set() {
        let plain = serde_json::to_value(NewDraft::from(&forge::NewDraft { body: "hi".into(), ..forge::NewDraft::default() })).unwrap();
        assert_eq!(plain, serde_json::json!({"note": "hi"}));
        let added = LineRef { old: None, new: Some(3) };
        let anchored = forge::NewDraft { body: "hi".into(), position: Some(at(added, None)), ..forge::NewDraft::default() };
        let json = serde_json::to_value(NewDraft::from(&anchored)).unwrap();
        assert_eq!(json["position"]["new_line"], 3);
        assert_eq!(json["position"]["old_line"], serde_json::Value::Null);
        assert_eq!(json["position"]["position_type"], "text");
        assert_eq!((json["position"]["base_sha"].as_str(), json["position"]["head_sha"].as_str()), (Some("a"), Some("b")));
    }

    #[test]
    fn mr_level_note_has_no_position_and_a_diff_note_has_one() {
        let plain = discussion(include_str!("fixtures/discussions.json"));
        assert!(!plain.notes[0].resolvable);
        assert_eq!(plain.notes[0].position, None);
        let diff = discussion(include_str!("fixtures/diff_note.json"));
        let position = diff.notes[0].position.as_ref().unwrap();
        assert_eq!((position.new_path.as_str(), position.line), ("src/pay/charge.rs", LineRef { old: None, new: Some(57) }));
    }

    #[test]
    fn an_image_position_hangs_nowhere() {
        let mut note: Note = parse::<Discussion>(include_str!("fixtures/diff_note.json")).notes.remove(0);
        note.position = note.position.map(|p| Position { position_type: "image".into(), ..p });
        assert_eq!(forge::Note::from(note).position, None);
    }

    #[test]
    fn line_code_hashes_the_path_and_zeroes_the_missing_side() {
        assert_eq!(sha("abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(line_code("abc", None, Some(13)), "a9993e364706816aba3e25717850c26c9cd0d89d_0_13");
        assert_eq!(line_code("abc", Some(13), None), "a9993e364706816aba3e25717850c26c9cd0d89d_13_0");
        assert_eq!(line_code("abc", Some(12), Some(12)), "a9993e364706816aba3e25717850c26c9cd0d89d_12_12");
    }

    #[test]
    fn one_line_positions_carry_the_numbers_of_their_side() {
        let cases = [
            (LineRef { old: Some(12), new: Some(12) }, Some(12), Some(12), "context carries both"),
            (LineRef { old: Some(13), new: None }, Some(13), None, "removed carries old only"),
            (LineRef { old: None, new: Some(13) }, None, Some(13), "added carries new only"),
        ];
        for (line, old, new, why) in cases {
            let wire = Position::from_model(&at(line, None));
            assert_eq!((wire.old_line, wire.new_line), (old, new), "{why}");
            assert_eq!(wire.line_range, None, "{why}");
            assert_eq!((wire.base_sha.as_str(), wire.head_sha.as_str(), wire.start_sha.as_str()), ("a", "b", "a"));
            assert_eq!(wire.clone().into_model(), Some(at(line, None)), "{why}: round trip");
        }
    }

    #[test]
    fn a_range_names_both_edges_with_their_line_codes() {
        let start = LineRef { old: Some(13), new: None };
        let end = LineRef { old: None, new: Some(14) };
        let wire = Position::from_model(&at(end, Some(start)));
        assert_eq!((wire.old_line, wire.new_line), (None, Some(14)), "the end line is an added one");
        let range = wire.line_range.clone().unwrap();
        let path = sha("src/pay/charge.rs");
        assert_eq!(
            range.start,
            LineCode { line_code: Some(format!("{path}_13_0")), kind: Some("old".into()), old_line: Some(13), new_line: None }
        );
        assert_eq!(
            range.end,
            LineCode { line_code: Some(format!("{path}_0_14")), kind: Some("new".into()), old_line: None, new_line: Some(14) }
        );
        let context = LineRef { old: Some(14), new: Some(15) };
        assert_eq!(Position::from_model(&at(end, Some(context))).line_range.unwrap().start.kind, None, "context start");
        assert_eq!(wire.into_model(), Some(at(end, Some(start))), "round trip");
    }
}
