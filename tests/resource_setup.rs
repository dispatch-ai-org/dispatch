#![cfg(unix)]
use std::process::Command;
#[test]
fn cli_and_tui_setup_auth_cancel_revalidation_and_goal_preservation() {
    let output = Command::new("python3")
        .arg(format!(
            "{}/tests/fixtures/resource_setup.py",
            env!("CARGO_MANIFEST_DIR")
        ))
        .arg(assert_cmd::cargo_bin!("dispatch"))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
