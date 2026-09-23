//! A funding refusal holds until the profile is re-authorized, and then the
//! profile launches again (0.4.1 S6b). See `tests/fixtures/reauthorization.py`.
#![cfg(unix)]
use std::process::Command;

fn scenario(provider: &str) {
    let output = Command::new("python3")
        .arg(format!(
            "{}/tests/fixtures/reauthorization.py",
            env!("CARGO_MANIFEST_DIR")
        ))
        .arg(assert_cmd::cargo_bin!("dispatch"))
        .arg(provider)
        .env("DISPATCH_FIXTURE_PROVIDER", provider)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .expect("Python standard-library fixture");
    assert!(
        output.status.success(),
        "{provider}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn codex_launches_again_after_reauthorization() {
    scenario("codex");
}

#[test]
fn claude_launches_again_after_reauthorization() {
    scenario("claude");
}
