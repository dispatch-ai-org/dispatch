#![cfg(unix)]
use std::process::Command;
fn scenario(name: &str, provider: &str) {
    let out = Command::new("python3")
        .arg(format!(
            "{}/tests/fixtures/phase8_planning.py",
            env!("CARGO_MANIFEST_DIR")
        ))
        .arg(assert_cmd::cargo_bin!("dispatch"))
        .arg(name)
        .env("DISPATCH_FIXTURE_PROVIDER", provider)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{name}: {}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
macro_rules! cases { ($($case:ident),* $(,)?) => { $(#[test] fn $case() {scenario(stringify!($case),"codex");})* }; }
cases!(
    success,
    malformed,
    truncated,
    nonfinal,
    failed,
    planner_edit,
    duplicate,
    cycle,
    missing_check,
    traversal,
    symlink,
    conflict,
    oversized,
    tight,
    budget,
    four,
    six,
    no_retry,
    repair,
    second_repair,
    dependency,
    assistance,
    already_ready,
    bad_dependency,
    cycle_report,
    stale_report,
    out_of_scope,
    weaken,
    root_fail,
    root_repair,
    question,
    control,
    grant,
    direct,
    cancel,
    disconnect,
    crash,
    deadline,
    fault_plan,
    fault_integration,
    fault_delivery,
    fault_completion,
    fault_fence,
    fault_manifest,
    uncertain_cleanup,
    drift,
    tamper,
    pty_plain,
    pty_narrow,
    pty_wide,
    mixed,
    pinned,
    planning_false,
    no_checks,
    no_planner,
    tight_repair,
    private_attribution,
    manifest_tamper,
    question_deadline,
    pty_question_deadline,
    question_no_retry,
    self_report,
    cross_report,
    fidelity
);
#[test]
fn explicit_override() {
    scenario("override", "codex");
}
#[test]
fn claude_success() {
    scenario("success", "claude");
}
#[test]
fn claude_question() {
    scenario("question", "claude");
}
