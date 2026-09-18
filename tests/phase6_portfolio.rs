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
macro_rules! control {
    ($($name:ident => $case:literal),* $(,)?) => {$(#[test] fn $name() { scenario("phase5_control", $case); })*};
}
control! {
    claude_scoped_control_verified_review => "smoke",
    claude_control_durable_idempotency => "idempotency",
    claude_control_clarification_idempotency => "clarify",
    claude_control_authority => "authority",
    claude_checkpoint_recovery => "recovery",
    claude_control_cancellation => "cancel",
    claude_owner_eof => "disconnect",
    claude_question_eof => "question_eof",
    claude_observer_disconnect => "observer",
    claude_control_framing => "framing",
    claude_control_recovery_limits => "recovery_limits",
    claude_control_unclassified_question => "unclassified",
    claude_control_broken_pipe => "broken_pipe",
    claude_control_lost_answer_receipt => "lost_answer",
    claude_control_changed_funding => "funding",
    claude_control_slow_output => "slow",
    claude_control_recovery_drift => "recovery_drift",
    claude_control_answer_cancel_race => "answer_cancel",
    claude_control_wait_events => "waits",
}
macro_rules! portfolio {
    ($($name:ident => $case:literal),* $(,)?) => {$(#[test] fn $name() { scenario("phase6_portfolio", $case); })*};
}
portfolio! {
    claude_only_cli_identity_usage_review => "direct",
    portfolio_preference_and_explicit_constraints => "selection",
    portfolio_missing_optional_and_explicit_failure => "missing",
    claude_funding_hazards_before_launch => "funding",
    claude_protocol_failures_do_not_recover => "protocol",
    portfolio_cross_harness_recovery_both_directions => "recovery",
    portfolio_grant_cannot_expand_and_receipts_do_not_replay => "grants",
    portfolio_independent_pools_and_same_pool_exclusion => "pools",
    portfolio_retained_capacity_routes_to_other_provider => "capacity",
    claude_profile_lifecycle_preserves_history => "lifecycle",
    claude_final_preflight_invalidates_epoch => "launch_change",
    claude_effective_settings_preserve_context_without_hooks => "settings",
    claude_malformed_configuration_and_expired_evidence => "configuration",
    claude_deadline_bounds_preflight_and_attempt => "deadline",
}
