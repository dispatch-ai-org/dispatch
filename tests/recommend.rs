use std::{
    fs,
    io::ErrorKind,
    net::TcpListener,
    path::{Path, PathBuf},
};

use assert_cmd::cargo_bin_cmd;
use chrono::{TimeZone, Utc};
use dispatch::{
    BenchmarkPrior, TaskKind, TaskScope, db::Database, public_priors::PublicPriorSnapshotV1,
    source::fingerprint_tree,
};

fn source(root: &Path) -> PathBuf {
    let source = root.join("source");
    for relative in ["src/lib.rs", "src/main.rs", "tests/retry.rs"] {
        let path = source.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "source\n").unwrap();
    }
    source
}

#[allow(clippy::too_many_arguments)]
fn prior(
    source: &str,
    dataset: &str,
    harness: &str,
    model: Option<&str>,
    language: Option<&str>,
    task_kind: TaskKind,
    successes: u64,
    attempts: u64,
) -> BenchmarkPrior {
    BenchmarkPrior {
        source: source.to_owned(),
        dataset: dataset.to_owned(),
        dataset_version: "fixture-v1".to_owned(),
        harness: harness.to_owned(),
        model: model.map(str::to_owned),
        language: language.map(str::to_owned),
        task_kind,
        scope: TaskScope::Unknown,
        successes,
        attempts,
        updated_at: Utc.with_ymd_and_hms(2026, 9, 1, 12, 0, 0).unwrap(),
    }
}

fn cache(state: &Path, priors: &[BenchmarkPrior]) -> anyhow::Result<()> {
    let mut database = Database::open(state.join("dispatch.db"))?;
    let unsupported = prior(
        "fixture-public-source",
        "fixture-public-dataset",
        "unsupported-public-agent",
        None,
        None,
        TaskKind::Unknown,
        1,
        1,
    );
    database.replace_distributed_public_priors(
        &PublicPriorSnapshotV1::from_priors(vec![unsupported])?,
        "downloaded",
    )?;
    for prior in priors {
        database.upsert_benchmark_prior(prior)?;
    }
    Ok(())
}

#[test]
fn recommend_classifies_and_displays_ranked_generic_harbor_evidence_offline() -> anyhow::Result<()>
{
    let temp = tempfile::tempdir()?;
    let source = source(temp.path());
    let state = temp.path().join("state");
    cache(
        &state,
        &[
            prior(
                "harbor-framework/harbor",
                "terminal-bench/terminal-bench-2",
                "codex",
                Some("openai/gpt-5"),
                None,
                TaskKind::Unknown,
                37,
                50,
            ),
            prior(
                "harbor-framework/harbor",
                "terminal-bench/terminal-bench-2",
                "cursor",
                None,
                None,
                TaskKind::Unknown,
                31,
                48,
            ),
            prior(
                "not-routing-evidence",
                "offline-fake",
                "fake-good",
                None,
                None,
                TaskKind::Unknown,
                1,
                1,
            ),
        ],
    )?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let fingerprint = fingerprint_tree(&source)?;

    let output = cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .env(
            "DISPATCH_CLOUD_URL",
            format!("http://{}", listener.local_addr()?),
        )
        .arg("recommend")
        .arg(&source)
        .args(["--task", "Fix the retry race in the worker."])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let output = String::from_utf8(output)?;

    assert!(output.contains("Task\n  language: rust\n  kind: bug_fix\n  scope: unknown"));
    assert!(output.contains("1. codex"));
    assert!(output.contains("benchmark success: 37/50 (74.0%)"));
    assert!(output.contains("evidence specificity: 0/3 (generic)"));
    assert!(output.contains("source: harbor-framework/harbor"));
    assert!(output.contains("dataset: terminal-bench/terminal-bench-2"));
    assert!(output.contains("model: openai/gpt-5"));
    assert!(output.contains("2. cursor"));
    assert!(output.contains("benchmark success: 31/48 (64.6%)"));
    assert!(output.contains("model: <unknown>"));
    assert!(!output.contains("fake-good"));
    assert!(output.find("1. codex").unwrap() < output.find("2. cursor").unwrap());
    assert_eq!(fingerprint_tree(&source)?, fingerprint);
    assert_eq!(listener.accept().unwrap_err().kind(), ErrorKind::WouldBlock);
    Ok(())
}

#[test]
fn recommend_uses_router_specificity_instead_of_a_presentation_score() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = source(temp.path());
    let state = temp.path().join("state");
    cache(
        &state,
        &[
            prior(
                "harbor-framework/harbor",
                "terminal-bench/terminal-bench-2",
                "codex",
                Some("openai/gpt-5"),
                None,
                TaskKind::Unknown,
                90,
                100,
            ),
            prior(
                "specific-public-evidence",
                "rust-repairs",
                "codex",
                Some("openai/gpt-5.1"),
                Some("rust"),
                TaskKind::BugFix,
                1,
                2,
            ),
        ],
    )?;

    let output = cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .arg("recommend")
        .arg(&source)
        .args(["--task", "Fix the retry race in the worker."])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let output = String::from_utf8(output)?;

    assert!(output.contains("benchmark success: 1/2 (50.0%)"));
    assert!(output.contains("evidence specificity: 2/3 (partial)"));
    assert!(output.contains("source: specific-public-evidence"));
    assert!(output.contains("dataset: rust-repairs"));
    assert!(output.contains("model: openai/gpt-5.1"));
    assert!(!output.contains("90/100"));
    Ok(())
}

#[test]
fn recommend_ties_preserve_router_harness_order() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = source(temp.path());
    let state = temp.path().join("state");
    cache(
        &state,
        &[
            prior(
                "harbor-framework/harbor",
                "terminal-bench",
                "cursor",
                None,
                None,
                TaskKind::Unknown,
                1,
                2,
            ),
            prior(
                "harbor-framework/harbor",
                "terminal-bench",
                "codex",
                None,
                None,
                TaskKind::Unknown,
                1,
                2,
            ),
        ],
    )?;

    let output = cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .arg("recommend")
        .arg(&source)
        .args(["--task", "Fix the retry race."])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let output = String::from_utf8(output)?;

    assert!(output.find("1. codex").unwrap() < output.find("2. cursor").unwrap());
    Ok(())
}

#[test]
fn no_or_zero_attempt_evidence_is_a_successful_explicit_no_recommendation() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = source(temp.path());
    let state = temp.path().join("state");
    let task_file = temp.path().join("task.md");
    fs::write(&task_file, "Introduce structured logging")?;
    cache(&state, &[])?;

    let no_evidence = cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .arg("recommend")
        .arg(&source)
        .arg("--task-file")
        .arg(&task_file)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let no_evidence = String::from_utf8(no_evidence)?;
    assert!(no_evidence.contains("kind: feature"));
    assert!(no_evidence.contains("No compatible routing evidence is available."));
    assert!(!no_evidence.contains("Recommendations"));

    cache(
        &state,
        &[prior(
            "zero-attempt-source",
            "empty",
            "codex",
            None,
            None,
            TaskKind::Unknown,
            0,
            0,
        )],
    )?;
    let zero_attempts = cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .arg("recommend")
        .arg(&source)
        .args(["--task", "Introduce structured logging"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert!(
        String::from_utf8(zero_attempts)?.contains("No compatible routing evidence is available.")
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn recommend_does_not_invoke_configured_harnesses() -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir()?;
    let source = source(temp.path());
    let state = temp.path().join("state");
    let marker = temp.path().join("harness-invoked");
    let trap = temp.path().join("trap-harness");
    fs::write(
        &trap,
        format!("#!/bin/sh\ntouch '{}'\nexit 99\n", marker.display()),
    )?;
    fs::set_permissions(&trap, fs::Permissions::from_mode(0o700))?;
    fs::write(
        source.join("dispatch.yml"),
        format!(
            "harnesses:\n  codex:\n    executable: {}\n  cursor:\n    executable: {}\n",
            trap.display(),
            trap.display()
        ),
    )?;

    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .arg("recommend")
        .arg(&source)
        .args(["--task", "Fix the retry race."])
        .assert()
        .success();

    assert!(!marker.exists());
    Ok(())
}
