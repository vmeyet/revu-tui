//! End-to-end runs of the `mr` binary: arguments, help and failures that need no network.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use assert_cmd::Command;
use predicates::prelude::*;

fn mr() -> Command {
    let mut cmd = Command::cargo_bin("mr").unwrap();
    cmd.env_remove("GITLAB_TOKEN").env_remove("GITLAB_HOST");
    cmd
}

#[test]
fn help_lists_the_subcommands() {
    mr().arg("--help").assert().success().stdout(predicate::str::contains("login")).stdout(predicate::str::contains("whoami"));
}

#[test]
fn version_carries_the_commit() {
    mr().arg("--version").assert().success().stdout(predicate::str::is_match(r"mr \d+\.\d+\.\d+ \(\w+\)").unwrap());
}

#[test]
fn whoami_with_a_bad_env_token_fails_before_any_prompt() {
    mr().args(["--host", "localhost:1", "whoami"])
        .env("GITLAB_TOKEN", "glpat-xxxx")
        .assert()
        .failure()
        .stderr(predicate::str::contains("✗"));
}

#[test]
fn login_refuses_a_token_in_argv() {
    mr().args(["login", "--token", "glpat-xxxx"]).assert().failure().stderr(predicate::str::contains("ps"));
}

#[test]
fn completions_render_for_zsh() {
    mr().args(["completions", "zsh"]).assert().success().stdout(predicate::str::contains("_mr"));
}

#[test]
fn list_help_mentions_the_cached_flag() {
    mr().args(["list", "--help"]).assert().success().stdout(predicate::str::contains("--cached"));
}

#[test]
fn show_rejects_a_reference_before_touching_the_network() {
    mr().args(["--host", "localhost:1", "show", "nonsense"])
        .env("GITLAB_TOKEN", "glpat-xxxx")
        .assert()
        .failure()
        .stderr(predicate::str::contains("group/project!42"));
}

#[test]
fn write_commands_explain_themselves() {
    mr().args(["comment", "--help"]).assert().success().stdout(predicate::str::contains("--at"));
    mr().args(["approve", "--help"]).assert().success().stdout(predicate::str::contains("--undo"));
    mr().args(["publish", "--help"]).assert().success().stdout(predicate::str::contains("draft"));
}

#[test]
fn comment_needs_a_text() {
    mr().args(["--host", "localhost:1", "comment", "acme/widgets!42"]).env("GITLAB_TOKEN", "glpat-xxxx").assert().failure().code(2);
}

#[test]
fn update_help_mentions_force() {
    mr().args(["update", "--help"]).assert().success().stdout(predicate::str::contains("--force"));
}
