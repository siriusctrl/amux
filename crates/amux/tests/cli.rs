use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn help_mentions_agent_sessions() {
    let mut cmd = Command::cargo_bin("amux").unwrap();
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "persistent local and remote agent sessions",
        ));
}

#[test]
fn target_list_includes_local_target() {
    let mut cmd = Command::cargo_bin("amux").unwrap();
    cmd.args(["target", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("local"))
        .stdout(predicate::str::contains("Local machine"));
}
