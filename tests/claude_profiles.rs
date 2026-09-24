#![cfg(unix)]
use std::process::Command;
fn scenario(script: &str, name: &str) {
    let output = Command::new("python3")
        .arg(format!(
            "{}/tests/fixtures/{script}.py",
            env!("CARGO_MANIFEST_DIR")
        ))
        .arg(assert_cmd::cargo_bin!("dispatch"))
        .arg(name)
        .env("DISPATCH_FIXTURE_PROVIDER", "claude")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .expect("Python standard-library fixture");
    assert!(
        output.status.success(),
        "{name}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
macro_rules! portfolio {
    ($($name:ident => $case:literal),* $(,)?) => {$(#[test] fn $name() { scenario("claude_profiles", $case); })*};
}
portfolio! {
    claude_only_cli_identity_usage_review => "direct",
    portfolio_preference_and_explicit_constraints => "selection",
    portfolio_missing_optional_and_explicit_failure => "missing",
    claude_funding_hazards_before_launch => "funding",
    claude_protocol_failures_do_not_recover => "protocol",
    claude_profile_lifecycle_preserves_history => "lifecycle",
    claude_final_preflight_invalidates_epoch => "launch_change",
    claude_effective_settings_preserve_context_without_hooks => "settings",
    claude_malformed_configuration_and_expired_evidence => "configuration",
    claude_deadline_bounds_preflight_and_attempt => "deadline",
}
