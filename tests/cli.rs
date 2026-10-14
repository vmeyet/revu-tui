//! End-to-end runs of the `revu` binary: arguments, help and failures that need no network.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use assert_cmd::Command;
use predicates::prelude::*;

/// The binary in a throwaway home, so no run reads or moves the real config, cache or tokens.
fn revu() -> Command {
    let home = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mut cmd = Command::cargo_bin("revu").unwrap();
    cmd.env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("GITLAB_TOKEN")
        .env_remove("GITLAB_HOST")
        .env_remove("GITHUB_TOKEN")
        .env_remove("GH_TOKEN");
    cmd
}

#[test]
fn help_lists_the_subcommands() {
    revu().arg("--help").assert().success().stdout(predicate::str::contains("login")).stdout(predicate::str::contains("whoami"));
}

#[test]
fn version_carries_the_commit() {
    revu().arg("--version").assert().success().stdout(predicate::str::is_match(r"revu \d+\.\d+\.\d+ \(\w+\)").unwrap());
}

#[test]
fn whoami_with_a_bad_env_token_fails_before_any_prompt() {
    revu()
        .args(["--host", "localhost:1", "whoami"])
        .env("GITLAB_TOKEN", "glpat-xxxx")
        .assert()
        .failure()
        .stderr(predicate::str::contains("✗"));
}

#[test]
fn login_refuses_a_token_in_argv() {
    revu().args(["login", "--token", "glpat-xxxx"]).assert().failure().stderr(predicate::str::contains("ps"));
}

#[test]
fn completions_render_for_zsh() {
    revu().args(["completions", "zsh"]).assert().success().stdout(predicate::str::contains("_revu"));
}

#[test]
fn list_help_mentions_the_cached_flag() {
    revu().args(["list", "--help"]).assert().success().stdout(predicate::str::contains("--cached"));
}

#[test]
fn show_rejects_a_reference_before_touching_the_network() {
    revu()
        .args(["--host", "localhost:1", "show", "nonsense"])
        .env("GITLAB_TOKEN", "glpat-xxxx")
        .assert()
        .failure()
        .stderr(predicate::str::contains("group/project!42"));
}

#[test]
fn write_commands_explain_themselves() {
    revu().args(["comment", "--help"]).assert().success().stdout(predicate::str::contains("--at"));
    revu().args(["approve", "--help"]).assert().success().stdout(predicate::str::contains("--undo"));
    revu().args(["ready", "--help"]).assert().success().stdout(predicate::str::contains("--undo"));
    revu().args(["publish", "--help"]).assert().success().stdout(predicate::str::contains("draft"));
}

#[test]
fn comment_needs_a_text() {
    revu().args(["--host", "localhost:1", "comment", "acme/widgets!42"]).env("GITLAB_TOKEN", "glpat-xxxx").assert().failure().code(2);
}

#[test]
fn update_help_mentions_force() {
    revu().args(["update", "--help"]).assert().success().stdout(predicate::str::contains("--force"));
}

#[test]
fn ai_login_refuses_a_key_in_argv() {
    revu().args(["ai", "login", "anthropic", "--token", "sk-ant-x"]).assert().failure().stderr(predicate::str::contains("ps"));
}

#[test]
fn ai_status_names_both_providers_without_a_key() {
    revu()
        .args(["ai", "status"])
        .env("ANTHROPIC_API_KEY", "sk-ant-secret")
        .env_remove("TYPESAFE_API_KEY")
        .assert()
        .success()
        .stdout(predicate::str::contains("typesafe").and(predicate::str::contains("anthropic")))
        .stdout(predicate::str::contains("ANTHROPIC_API_KEY"))
        .stdout(predicate::str::contains("sk-ant-secret").not());
}

#[test]
fn ai_ask_says_claude_is_off_before_touching_the_network() {
    revu()
        .args(["--host", "localhost:1", "ai", "ask", "acme/widgets!42", "why?"])
        .env("GITLAB_TOKEN", "glpat-xxxx")
        .env("ANTHROPIC_API_KEY", "sk-ant-xxxx")
        .assert()
        .failure()
        .stderr(predicate::str::contains("[ai.anthropic] enabled = true"));
}

#[test]
fn merge_help_mentions_yes() {
    revu().args(["merge", "--help"]).assert().success().stdout(predicate::str::contains("--yes"));
}

#[test]
fn a_bare_number_outside_a_checkout_fails_before_the_tui() {
    revu()
        .args(["--host", "localhost:1", "42"])
        .current_dir(std::env::temp_dir())
        .env("GITLAB_TOKEN", "glpat-xxxx")
        .assert()
        .failure()
        .stderr(predicate::str::contains("group/project!42"));
}

#[test]
fn tui_rejects_a_reference_before_the_screen_changes() {
    revu()
        .args(["--host", "localhost:1", "tui", "nonsense"])
        .env("GITLAB_TOKEN", "glpat-xxxx")
        .assert()
        .failure()
        .stderr(predicate::str::contains("group/project!42"));
}

#[test]
fn news_prints_every_version_newest_first() {
    let page = revu().arg("news").assert().success().get_output().stdout.clone();
    let page = String::from_utf8(page).unwrap();
    let (newer, older) = (page.find("0.6.0 · 2026-10-06").unwrap(), page.find("0.2.0 · 2026-09-30").unwrap());
    assert!(newer < older, "{page}");
    assert!(page.contains("  New\n  • "), "{page}");
}

#[test]
fn news_since_keeps_the_newer_versions_only() {
    revu()
        .args(["news", "--since", "0.5.0"])
        .assert()
        .success()
        .stdout(predicate::str::contains("0.6.0 · 2026-10-06").and(predicate::str::contains("0.5.0 · ").not()));
    revu().args(["news", "--since", "99.0.0"]).assert().success().stdout("Nothing new since 99.0.0.\n");
    revu().args(["news", "--since", "banana"]).assert().failure().code(2).stderr(predicate::str::contains("write it like 0.6.0"));
}

#[test]
fn news_as_json_lists_what_where_and_how() {
    let output = revu().args(["news", "--since", "0.5.0", "--json"]).assert().success().get_output().stdout.clone();
    let sections: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let release = sections.as_array().unwrap().iter().find(|s| s["version"] == "0.6.0").unwrap();
    assert_eq!(release["date"], "2026-10-06");
    let entry = &release["new"][0];
    assert!(entry["what"].is_string() && entry["where"].is_string() && entry["how"].is_string(), "{entry}");
    assert!(release["fixed"].is_array());
}

#[test]
fn the_update_tail_prints_nothing_when_nothing_is_new() {
    revu().args(["news", "--after-update", "--since", "99.0.0"]).assert().success().stdout("");
    revu()
        .args(["news", "--after-update", "--since", "0.5.0"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("\nNew since 0.5.0\n").and(predicate::str::contains("revu news")));
}

#[test]
fn completions_know_the_news_command() {
    revu()
        .args(["completions", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("news").and(predicate::str::contains("--since")));
}

#[test]
fn a_config_written_for_a_newer_revu_warns_once_and_the_command_runs() {
    let config = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(config.path().join("revu")).unwrap();
    std::fs::write(config.path().join("revu/config.toml"), "[tui]\nsparkles = true\n\n[sync.later]\nevery = 5\n").unwrap();
    let warned = predicate::str::contains("`tui.sparkles` is not a setting this revu knows; it is ignored — written for a newer revu?")
        .and(predicate::str::contains("`sync` is not a setting"))
        .and(predicate::str::contains("config.toml"));
    revu().env("XDG_CONFIG_HOME", config.path()).arg("usage").assert().success().stderr(warned);
    revu().env("XDG_CONFIG_HOME", config.path()).arg("--version").assert().success().stderr("");
}
