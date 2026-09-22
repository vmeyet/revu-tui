use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: u64,
    pub username: String,
    pub name: String,
    #[serde(default)]
    pub avatar_url: Option<String>,
}

/// One merge request as `GET /projects/:id/merge_requests/:iid` returns it, plus its approvals.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mr {
    pub id: u64,
    pub iid: u64,
    pub project_id: u64,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub state: String,
    pub draft: bool,
    pub author: User,
    pub source_branch: String,
    pub target_branch: String,
    pub web_url: String,
    pub updated_at: DateTime<Utc>,
    pub sha: String,
    pub diff_refs: DiffRefs,
    #[serde(default)]
    pub head_pipeline: Option<Pipeline>,
    /// GitLab sends this count as a string, `"9"` or `"1000+"`.
    #[serde(default)]
    pub changes_count: Option<String>,
    #[serde(default)]
    pub has_conflicts: bool,
    #[serde(default = "yes")]
    pub blocking_discussions_resolved: bool,
    #[serde(default)]
    pub reviewers: Vec<User>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub approvals: Approvals,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffRefs {
    pub base_sha: String,
    pub head_sha: String,
    pub start_sha: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pipeline {
    pub status: String,
    #[serde(default)]
    pub web_url: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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

fn nested_users<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<User>, D::Error> {
    #[derive(Deserialize)]
    struct Approver {
        user: User,
    }
    Ok(Vec::<Approver>::deserialize(d)?.into_iter().map(|a| a.user).collect())
}

/// One element of `GET …/diffs`: the unified diff body of one file and how the file changed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffFile {
    pub diff: String,
    pub old_path: String,
    pub new_path: String,
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
    #[serde(default)]
    pub too_large: bool,
    #[serde(default)]
    pub collapsed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Discussion {
    pub id: String,
    #[serde(default)]
    pub individual_note: bool,
    pub notes: Vec<Note>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub id: u64,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
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

/// GitLab answers errors as `{"message": …}` or `{"error": …}`; anything else is shown as is.
pub fn error_message(body: &str) -> String {
    let parsed: Option<serde_json::Value> = serde_json::from_str(body).ok();
    parsed
        .as_ref()
        .and_then(|v| v.get("message").or_else(|| v.get("error")))
        .map(|m| match m {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_else(|| body.trim().to_owned())
}

pub fn from_fixture<T: DeserializeOwned>(json: &str) -> T {
    serde_json::from_str(json).expect("fixture parses")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_message_reads_both_shapes() {
        assert_eq!(error_message(r#"{"message":"401 Unauthorized"}"#), "401 Unauthorized");
        assert_eq!(error_message(r#"{"error":"invalid_token"}"#), "invalid_token");
        assert_eq!(error_message("<html>gateway</html>"), "<html>gateway</html>");
    }

    #[test]
    fn approvals_flatten_the_nested_users() {
        let approvals: Approvals = from_fixture(
            r#"{"approved": true, "approvals_left": 0, "approved_by": [{"user": {"id": 2, "username": "nina", "name": "Nina"}, "approved_at": "2026-09-22T10:00:00Z"}]}"#,
        );
        assert_eq!(approvals.approved_by.iter().map(|u| u.username.as_str()).collect::<Vec<_>>(), ["nina"]);
    }

    #[test]
    fn mr_level_note_has_no_position_and_a_diff_note_has_one() {
        let plain: Discussion = from_fixture(include_str!("fixtures/discussions.json"));
        assert_eq!(plain.notes[0].kind, None);
        assert!(!plain.notes[0].resolvable);
        assert_eq!(plain.notes[0].position, None);
        let diff: Discussion = from_fixture(include_str!("fixtures/diff_note.json"));
        let position = diff.notes[0].position.as_ref().unwrap();
        assert_eq!((position.new_path.as_deref(), position.new_line, position.old_line), (Some("src/pay/charge.rs"), Some(57), None));
    }
}
