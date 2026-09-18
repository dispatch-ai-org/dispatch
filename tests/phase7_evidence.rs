#![cfg(unix)]
use std::process::Command;
fn scenario(name: &str) {
    let output = Command::new("python3")
        .arg(format!(
            "{}/tests/fixtures/phase7_evidence.py",
            env!("CARGO_MANIFEST_DIR")
        ))
        .arg(assert_cmd::cargo_bin!("dispatch"))
        .arg(name)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .expect("standard-library fixture");
    assert!(
        output.status.success(),
        "{name}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn private_fixture_shadow_activate_rollback() {
    scenario("smoke");
}
#[test]
fn private_c_feature_cohort_preserves_historical_unknown_and_trial_scope() {
    scenario("c_cohort");
}
#[test]
fn private_outcomes_and_smoke_attribution() {
    scenario("outcomes");
}
#[test]
fn private_feedback_revision_stales_proposal() {
    scenario("revisions");
}
#[test]
fn private_behavior_and_check_compatibility() {
    scenario("compatibility");
}
#[test]
fn private_explicit_provenance_not_interface_inference() {
    scenario("ordinary");
}
#[test]
fn private_bounded_large_history() {
    scenario("bounds");
}

#[test]
fn private_promoted_preference_preserves_constraints_and_scope() {
    scenario("safety");
}
#[test]
fn private_stale_active_evidence_falls_back_after_restart() {
    scenario("stale_active");
}
#[test]
fn private_policy_transition_is_atomic_and_revision_fenced() {
    scenario("atomic");
}

#[test]
fn private_activation_preserves_inflight_and_queued_bindings() {
    scenario("inflight");
}

#[test]
fn private_optional_analytics_failure_never_waives_authority() {
    scenario("analytics_failure");
}

#[test]
fn private_owner_helper_bounds_timeout_eof_and_normal_cleanup() {
    let output = Command::new("python3")
        .arg(format!(
            "{}/tests/fixtures/phase7_owner_test.py",
            env!("CARGO_MANIFEST_DIR")
        ))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .expect("standard-library helper regressions");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn review_scenarios(names: &[&str]) {
    let output = Command::new("python3")
        .arg(format!(
            "{}/tests/fixtures/phase7_review.py",
            env!("CARGO_MANIFEST_DIR")
        ))
        .arg(assert_cmd::cargo_bin!("dispatch"))
        .args(names)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .expect("synthetic owner review PTY");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn private_review_opt_in_cancel_duplicate_and_plain() {
    review_scenarios(&[
        "accept",
        "reject",
        "attest",
        "reject_attest",
        "cancel",
        "back",
        "duplicate",
        "plain",
    ]);
}
#[test]
fn private_review_revision_delivery_provenance_and_write_failure() {
    review_scenarios(&[
        "revision",
        "delivery",
        "provenance",
        "failure",
        "synthetic",
        "scripted_smoke",
        "pending",
    ]);
}
#[test]
fn private_review_fixed_session_and_apply_blocked() {
    review_scenarios(&["other_session", "blocked"]);
}
