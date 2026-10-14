//! Keys a config holds that this revu does not read: written for a newer revu, moved since, or a secret.
//! A newer revu's config must not stop an older one, so each is a warning; only a secret still fails.
use super::Config;
use anyhow::{Result, bail};

const AI_SWITCH_MOVED: &str = "`[ai] enabled`, `provider` and `model` moved to one table per provider: \
     `[ai.anthropic] enabled = true` (with `model`), `[ai.typesafe] enabled = true`; until then both stay off";

/// The single switch an older revu read, before one table per provider.
const AI_SWITCH: [&str; 3] = ["ai.enabled", "ai.provider", "ai.model"];

/// Words no setting is named after, since secrets live in the keychain.
const SECRET_WORDS: [&str; 4] = ["key", "token", "secret", "password"];

/// The config `text` holds, and one warning for each key it ignored.
pub(super) fn parse(text: &str) -> Result<(Config, Vec<String>)> {
    let mut ignored = vec![];
    let config = serde_ignored::deserialize(toml::Deserializer::parse(text)?, |path| ignored.push(path.to_string()))?;
    if let Some(secret) = ignored.iter().find(|key| is_secret(key)) {
        bail!("`{secret}` looks like a secret: revu keeps keys in the keychain, never in the config (`revu login`, `revu ai login`)");
    }
    Ok((config, warnings(&ignored)))
}

fn is_secret(key: &str) -> bool {
    let name = key.rsplit('.').next().unwrap_or(key).to_ascii_lowercase();
    SECRET_WORDS.iter().any(|word| name.strip_suffix(word).is_some_and(|rest| rest.is_empty() || rest.ends_with('_')))
}

/// One line per ignored key, the moved ones saying where they went, each said once.
fn warnings(ignored: &[String]) -> Vec<String> {
    let mut warnings: Vec<String> = vec![];
    for warning in ignored.iter().map(|key| warning(key)) {
        if !warnings.contains(&warning) {
            warnings.push(warning);
        }
    }
    warnings
}

fn warning(key: &str) -> String {
    if AI_SWITCH.contains(&key) {
        return AI_SWITCH_MOVED.to_owned();
    }
    format!("`{key}` is not a setting this revu knows; it is ignored — written for a newer revu?")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn warned(text: &str) -> Vec<String> {
        parse(text).unwrap().1
    }

    #[test]
    fn an_unknown_table_is_ignored_with_a_warning_naming_it() {
        let (config, warnings) = parse("[tui]\ntheme = \"nord\"\n\n[sync]\nevery = 5\n").unwrap();
        assert_eq!(config.tui.theme.as_deref(), Some("nord"), "what it knows still loads");
        assert_eq!(warnings, ["`sync` is not a setting this revu knows; it is ignored — written for a newer revu?"]);
    }

    #[test]
    fn unknown_keys_are_named_by_their_full_path_at_any_depth() {
        assert_eq!(
            warned("[tui]\nsparkles = true\n"),
            ["`tui.sparkles` is not a setting this revu knows; it is ignored — written for a newer revu?"]
        );
        let nested = warned("[queue.rules]\nstale_days = 30\nhalf_life = 2\n\n[sync.later]\nevery = 5\n");
        assert_eq!(nested.len(), 2, "{nested:?}");
        assert!(nested[0].starts_with("`queue.rules.half_life` is not") && nested[1].starts_with("`sync` is not"), "{nested:?}");
        assert!(warned("[share.targets.team]\ncommand = \"hook\"\nemoji = \"x\"\n")[0].starts_with("`share.targets.team.emoji`"));
    }

    #[test]
    fn a_config_of_known_keys_warns_of_nothing() {
        assert_eq!(warned("host = \"gitlab.com\"\n[agent]\nenabled = true\n[keys.bind]\nnext_thread = \"F\"\n"), [] as [String; 0]);
    }

    #[test]
    fn a_known_key_with_a_wrong_type_still_fails_with_its_name() {
        let err = parse("[tui]\nascii = \"yes\"\n").unwrap_err().to_string();
        assert!(err.contains("ascii") && err.contains("line 2"), "{err}");
        assert!(parse("[notify]\nenabled = 3\nsparkles = true\n").is_err(), "an ignored key does not hide a bad one");
    }

    #[test]
    fn the_old_ai_switch_points_at_the_new_tables_once_and_leaves_both_off() {
        let (config, warnings) = parse("[ai]\nenabled = true\nprovider = \"anthropic\"\nmodel = \"claude-opus-5\"\n").unwrap();
        assert_eq!(warnings, [AI_SWITCH_MOVED]);
        assert!(AI_SWITCH_MOVED.contains("[ai.anthropic] enabled = true"));
        assert!(!config.ai.anthropic.enabled && !config.ai.typesafe.enabled);
    }

    #[test]
    fn a_key_that_looks_like_a_secret_fails_to_load() {
        let err = parse("[ai.anthropic]\nkey = \"sk-ant\"\n").unwrap_err().to_string();
        assert!(err.contains("`ai.anthropic.key`") && err.contains("keychain"), "{err}");
        assert!(parse("token = \"glpat-xxxx\"\n").is_err());
        assert!(parse("[tui]\napi_key = \"x\"\n").is_err());
        assert!(parse("[tui]\nkeyboard = \"x\"\n").is_ok(), "only a whole word counts");
    }
}
