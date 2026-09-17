use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn bare_non_tty_is_predictable_and_creates_no_state() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    Command::new(assert_cmd::cargo_bin!("dispatch"))
        .args(["--plain", "--ascii", "--no-color", "--state-dir"])
        .arg(&state)
        .write_stdin("do not execute this\n")
        .assert()
        .code(2)
        .stdout(predicate::str::contains("Usage:"))
        .stdout(predicate::str::contains("\u{1b}").not());
    assert!(!state.exists());
}

#[test]
fn presentation_flags_do_not_pollute_machine_results() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("file.txt"), "baseline").unwrap();
    let output = Command::new(assert_cmd::cargo_bin!("dispatch"))
        .arg("--state-dir")
        .arg(temp.path().join("state"))
        .args([
            "--plain",
            "--ascii",
            "--no-color",
            "run",
            "test fixture",
            "--source",
        ])
        .arg(source)
        .args(["--harnesses", "fake-good", "--jsonl"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.stdout.contains(&0x1b));
    for line in String::from_utf8(output.stdout).unwrap().lines() {
        serde_json::from_str::<serde_json::Value>(line).unwrap();
    }
}
