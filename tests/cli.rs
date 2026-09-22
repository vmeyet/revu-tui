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
