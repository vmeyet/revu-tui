use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: u64,
    pub username: String,
    pub name: String,
    #[serde(default)]
    pub avatar_url: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_message_reads_both_shapes() {
        assert_eq!(error_message(r#"{"message":"401 Unauthorized"}"#), "401 Unauthorized");
        assert_eq!(error_message(r#"{"error":"invalid_token"}"#), "invalid_token");
        assert_eq!(error_message("<html>gateway</html>"), "<html>gateway</html>");
    }
}
