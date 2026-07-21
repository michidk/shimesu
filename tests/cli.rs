use assert_cmd::Command;
use predicates::prelude::*;

fn shimesu() -> Command {
    Command::cargo_bin("shimesu").expect("binary should build")
}

#[test]
fn help_lists_the_public_command_surface() {
    shimesu().arg("--help").assert().success().stdout(
        predicate::str::contains("publish")
            .and(predicate::str::contains("site"))
            .and(predicate::str::contains("stack"))
            .and(predicate::str::contains("doctor")),
    );
}

#[test]
fn version_flag_reports_the_crate_version() {
    shimesu()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn stack_destroy_without_confirm_exits_with_usage_code() {
    shimesu()
        .args(["stack", "destroy"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn stack_teardown_without_data_loss_confirmation_exits_with_usage_code() {
    shimesu()
        .args(["stack", "teardown"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn publish_missing_path_reports_structured_validation_error() {
    let output = shimesu()
        .args(["publish", "./does-not-exist", "--yes", "--json"])
        .output()
        .expect("binary should run");

    assert_eq!(output.status.code(), Some(7));
    let error: serde_json::Value =
        serde_json::from_slice(&output.stderr).expect("stderr should be structured JSON");
    assert_eq!(error["schema_version"], "1");
    assert_eq!(error["error"], true);
    assert_eq!(error["error_category"], "validation");
    assert_eq!(error["exit_code"], 7);
}

#[test]
fn publish_invalid_slug_fails_before_any_aws_call() {
    let staging = tempfile::tempdir().expect("tempdir should exist");
    let file_path = staging.path().join("page.html");
    std::fs::write(&file_path, "<html></html>").expect("fixture should write");

    let output = shimesu()
        .args(["publish"])
        .arg(&file_path)
        .args(["--site", "INVALID SLUG", "--yes", "--json"])
        .output()
        .expect("binary should run");

    assert_eq!(output.status.code(), Some(7));
    let error: serde_json::Value =
        serde_json::from_slice(&output.stderr).expect("stderr should be structured JSON");
    assert_eq!(error["error_category"], "validation");
}

#[test]
fn site_delete_noninteractive_without_yes_is_a_usage_error() {
    let output = shimesu()
        .args(["site", "delete", "docs", "--json"])
        .output()
        .expect("binary should run");

    assert_eq!(output.status.code(), Some(2));
    let error: serde_json::Value =
        serde_json::from_slice(&output.stderr).expect("stderr should be structured JSON");
    assert_eq!(error["error_category"], "usage");
    assert_eq!(error["exit_code"], 2);
}
