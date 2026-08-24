use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn help_describes_interactive_run_and_server_modes() {
    let mut command = Command::cargo_bin("ariadne").unwrap();
    command.arg("--help");

    command.assert().success().stdout(
        predicate::str::contains("A local-first AI agent")
            .and(predicate::str::contains("chat"))
            .and(predicate::str::contains("run"))
            .and(predicate::str::contains("serve")),
    );
}
