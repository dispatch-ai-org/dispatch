//! `result.coherence` over the control protocol (`dispatch control --stdio`),
//! driven by `tests/fixtures/coherence_control.py` with the fake agent and the
//! standard-library client. See docs/control-protocol.md, "coherence (additive)".
#![cfg(unix)]
use std::process::Command;

fn scenario(name: &str) {
    let output = Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/coherence_control.py"
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
fn unmoved_source_has_no_coherence_key() {
    scenario("unchanged");
}

#[test]
fn ignored_build_output_has_no_coherence_key() {
    scenario("ignored");
}

#[test]
fn unrelated_edit_continues_with_a_summary_and_no_reasons() {
    scenario("unrelated");
}

#[test]
fn conflicting_edit_refreshes_with_patch_conflict_and_no_local_paths() {
    scenario("conflict");
}
