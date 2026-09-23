//! End-to-end runs of the `revu` binary: arguments, help and failures that need no network.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use assert_cmd::Command;
use predicates::prelude::*;

/// The binary in a throwaway home, so no run reads or moves the real config, cache or tokens.
fn revu() -> Command {
    let home = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mut cmd = Command::cargo_bin("revu").unwrap();
    cmd.env("HOME", home).env_remove("GITLAB_TOKEN").env_remove("GITLAB_HOST").env_remove("GITHUB_TOKEN").env_remove("GH_TOKEN");
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
