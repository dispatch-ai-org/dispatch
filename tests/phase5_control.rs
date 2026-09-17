#![cfg(unix)]
use std::process::Command;
fn scenario(name: &str) {
    let output = Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/phase5_control.py"
        ))
        .arg(assert_cmd::cargo_bin!("dispatch"))
        .arg(name)
        .output()
        .expect("python3 standard-library protocol fixture");
    assert!(
        output.status.success(),
        "{name}\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn standalone_verified_delivery_preserves_human_review() {
    scenario("smoke");
}
#[test]
fn durable_submit_retry_and_payload_conflict() {
    scenario("idempotency");
}
#[test]
fn duplex_clarification_and_idempotent_answer() {
    scenario("clarify");
}
#[test]
fn scoped_authority_and_stale_mutations() {
    scenario("authority");
}
#[test]
fn explicit_checkpoint_recovery_keeps_original_limits() {
    scenario("recovery");
}
#[test]
fn pending_wait_does_not_block_cancellation() {
    scenario("cancel");
}
#[test]
fn owner_eof_cancels_execution() {
    scenario("disconnect");
}
#[test]
fn owner_eof_closes_question() {
    scenario("question_eof");
}
#[test]
fn read_only_follower_disconnect_does_not_cancel_owner() {
    scenario("observer");
}
#[test]
fn semantic_waits_and_ordered_event_replay() {
    scenario("waits");
}
#[test]
fn bounded_versioned_frames_and_no_unintended_execution() {
    scenario("framing");
}

#[test]
fn lost_answer_reply_recovers_only_explicitly() {
    scenario("lost_answer");
}
#[test]
fn changed_funding_blocks_continuation() {
    scenario("funding");
}
#[test]
fn unclassified_questions_require_human_authority() {
    scenario("unclassified");
}
#[test]
fn recovery_rejects_live_owner_and_expired_budget() {
    scenario("recovery_limits");
}
#[test]
fn slow_output_cancels_without_blocking_child_drainage() {
    scenario("slow");
}
#[test]
fn broken_output_pipe_cancels_owner() {
    scenario("broken_pipe");
}
#[test]
fn hangup_cancels_waiting_question() {
    scenario("hangup");
}
#[test]
fn owner_eof_during_admission_never_spawns() {
    scenario("queued_eof");
}
#[test]
fn control_and_existing_foreground_share_admission() {
    scenario("overlap");
}
#[test]
fn receipt_failure_rolls_back_submit_effect_and_event() {
    scenario("atomic");
}
#[test]
fn resolved_artifact_paths_and_resource_scope_are_enforced() {
    scenario("paths");
}

#[test]
fn human_tui_and_control_share_allowance_without_review_transfer() {
    scenario("tui_overlap");
}
#[test]
fn overlapping_changed_mappings_still_exclude_control() {
    scenario("mapped_overlap");
}
#[test]
fn migration_preserves_existing_questions_attempts_and_authorizations() {
    scenario("migration");
}

#[test]
fn cancelled_answer_receipt_never_reopens_work() {
    scenario("answer_cancel");
}
#[test]
fn recovery_checks_original_source_lineage() {
    scenario("recovery_drift");
}
#[test]
fn broken_pipe_cancels_clarification() {
    scenario("broken_question");
}
#[test]
fn broken_pipe_cancels_queued_admission() {
    scenario("broken_admission");
}
#[test]
fn observer_bound_leaves_cancellation_responsive() {
    scenario("limits");
}
