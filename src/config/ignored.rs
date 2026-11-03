//! Keys a config holds that this revu does not read: written for another revu, or a secret.
//! A newer revu's config must not stop an older one, so each is a warning; only a secret still fails.
use super::Config;
use anyhow::{Result, bail};

/// Words no setting is named after, since secrets live in the keychain.
const SECRET_WORDS: [&str; 4] = ["key", "token", "secret", "password"];

/// The config `text` holds, and one warning for each key it ignored.
pub(super) fn parse(text: &str) -> Result<(Config, Vec<String>)> {
    let mut ignored = vec![];
    let config: Config = serde_ignored::deserialize(toml::Deserializer::parse(text)?, |path| ignored.push(path.to_string()))?;
    ignored.extend(crate::keymap::unknown_actions(&config.keys).map(|action| format!("keys.bind.{action}")));
    if let Some(secret) = ignored.iter().find(|key| is_secret(key)) {
        bail!("`{secret}` looks like a secret: revu keeps keys in the keychain, never in the config (`revu login`, `revu ai login`)");
    }
    Ok((config, ignored.iter().map(|key| warning(key)).collect()))
}

fn is_secret(key: &str) -> bool {
    let name = key.rsplit('.').next().unwrap_or(key).to_ascii_lowercase();
    SECRET_WORDS.iter().any(|word| name.strip_suffix(word).is_some_and(|rest| rest.is_empty() || rest.ends_with('_')))
}

fn warning(key: &str) -> String {
    format!("`{key}` is not a setting this revu knows; it is ignored")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn warned(text: &str) -> Vec<String> {
        parse(text).unwrap().1
    }

    #[test]
    fn an_unknown_table_is_ignored_with_a_warning_naming_it() {
        let (config, warnings) = parse("[tui]\ntheme = \"nord\"\n\n[sync]\nevery = 5\n").unwrap();
        assert_eq!(config.tui.theme.as_deref(), Some("nord"), "what it knows still loads");
        assert_eq!(warnings, ["`sync` is not a setting this revu knows; it is ignored"]);
    }

    #[test]
    fn unknown_keys_are_named_by_their_full_path_at_any_depth() {
        assert_eq!(warned("[tui]\nsparkles = true\n"), ["`tui.sparkles` is not a setting this revu knows; it is ignored"]);
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
    fn a_binding_on_an_unknown_action_is_ignored_with_a_warning_naming_it() {
        let (config, warnings) = parse("[keys.bind]\nnext_thred = \"F\"\nnext_thread = \"f\"\n").unwrap();
        assert_eq!(warnings, ["`keys.bind.next_thred` is not a setting this revu knows; it is ignored"]);
        assert_eq!(config.keys.bind.len(), 2, "the file loads whole; the keymap leaves the unknown one out");
    }

    #[test]
    fn a_key_that_looks_like_a_secret_fails_to_load() {
        let err = parse("[ai.anthropic]\nkey = \"sk-ant\"\n").unwrap_err().to_string();
        assert!(err.contains("`ai.anthropic.key`") && err.contains("keychain"), "{err}");
        assert!(parse("token = \"glpat-xxxx\"\n").is_err());
        assert!(parse("[tui]\napi_key = \"x\"\n").is_err());
        assert!(parse("[tui]\nkeyboard = \"x\"\n").is_ok(), "only a whole word counts");
        assert!(parse("[keys.bind]\napi_key = \"sk-ant\"\n").is_err(), "a binding on no action is held to the same rule");
    }
}
