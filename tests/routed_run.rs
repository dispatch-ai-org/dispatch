use std::{
    fs,
    path::{Path, PathBuf},
};

use assert_cmd::cargo_bin_cmd;
use chrono::{TimeZone, Utc};
use dispatch::{BenchmarkPrior, TaskKind, TaskScope, db::Database, source::fingerprint_tree};
use rusqlite::Connection;
use serde_json::Value;

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
    dataset_version: &str,
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
        dataset_version: dataset_version.to_owned(),
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
    let database = Database::open(state.join("dispatch.db"))?;
    for prior in priors {
        database.upsert_benchmark_prior(prior)?;
    }
    Ok(())
}

#[cfg(unix)]
fn executable(path: &Path, marker: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::write(
        path,
        format!(
            "#!/bin/sh\n\
             if [ \"$1\" = \"--version\" ]; then\n\
               printf 'cursor fixture 1.0\\n'\n\
               exit 0\n\
             fi\n\
             printf 'run\\n' >> '{}'\n\
             printf 'routed\\n' > cursor-routed.txt\n\
             printf '{{\"type\":\"result\",\"model\":\"fixture-cursor\"}}\\n'\n",
            marker.display()
        ),
    )?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

fn configure(source: &Path, cursor: &Path, codex: &Path, claude: &Path) -> anyhow::Result<()> {
    fs::write(
        source.join("dispatch.yml"),
        format!(
            r#"execution:
  timeout_secs: 2
  max_parallel: 3
checks:
  verify:
    - test -f cursor-routed.txt
harnesses:
  claude:
    executable: "{}"
  codex:
    executable: "{}"
  cursor:
    executable: "{}"
"#,
            claude.display(),
            codex.display(),
            cursor.display()
        ),
    )?;
    Ok(())
}

fn only_metadata(state: &Path) -> anyhow::Result<(PathBuf, Value)> {
    let paths = fs::read_dir(state.join("runs"))?
        .map(|entry| entry.map(|entry| entry.path().join("metadata.json")))
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(paths.len(), 1);
    let path = paths.into_iter().next().unwrap();
    let value = serde_json::from_slice(&fs::read(&path)?)?;
    Ok((path, value))
}

#[cfg(unix)]
#[test]
fn routed_run_skips_unavailable_top_prediction_and_uses_existing_execution_path()
-> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = source(temp.path());
    let state = temp.path().join("state");
    let marker = temp.path().join("cursor-invocations");
    let cursor = temp.path().join("cursor-fixture");
    let missing_codex = temp.path().join("missing-codex");
    let missing_claude = temp.path().join("missing-claude");
    executable(&cursor, &marker)?;
    configure(&source, &cursor, &missing_codex, &missing_claude)?;
    cache(
        &state,
        &[
            prior(
                "unsupported-source",
                "unsupported-dataset",
                "v1",
                "mini-SWE-agent",
                Some("fixture-model"),
                None,
                TaskKind::Unknown,
                1,
                1,
            ),
            prior(
                "harbor-framework/harbor",
                "terminal-bench",
                "2.0",
                "codex",
                Some("openai/gpt-5"),
                None,
                TaskKind::Unknown,
                9,
                10,
            ),
            prior(
                "harbor-framework/harbor",
                "terminal-bench",
                "2.0",
                "cursor",
                Some("anthropic/claude-sonnet-4"),
                None,
                TaskKind::Unknown,
                99,
                100,
            ),
            prior(
                "specific-public-evidence",
                "rust-repairs",
                "2026-08",
                "cursor",
                Some("fixture/specific-model"),
                Some("rust"),
                TaskKind::BugFix,
                1,
                2,
            ),
        ],
    )?;
    let source_fingerprint = fingerprint_tree(&source)?;

    let output = cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .env(
            "DISPATCH_CLOUD_URL",
            "http://routing-must-not-contact.invalid",
        )
        .arg("run")
        .arg(&source)
        .args([
            "--task",
            "Fix the retry race in the worker.",
            "--route",
            "--allow-unsafe-local",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output)?;

    assert!(stdout.contains("Routing\n"));
    assert!(stdout.contains("selected: cursor"));
    assert!(stdout.contains("benchmark success: 1/2 (50.0%)"));
    assert!(stdout.contains("evidence specificity: 2/3 (partial)"));
    assert!(stdout.contains("source: specific-public-evidence"));
    assert!(stdout.contains("Preparing 1 candidate(s)..."));
    assert_eq!(fs::read_to_string(&marker)?, "run\n");
    assert_eq!(fingerprint_tree(&source)?, source_fingerprint);
    assert!(!source.join("cursor-routed.txt").exists());
    let (_, metadata) = only_metadata(&state)?;
    assert_eq!(metadata["status"], "ready_for_evaluation");
    assert_eq!(metadata["candidates"].as_array().unwrap().len(), 1);
    assert_eq!(metadata["candidates"][0]["harness_id"], "cursor");
    assert_eq!(metadata["candidates"][0]["status"], "completed");
    assert_eq!(metadata["candidates"][0]["checks"][0]["status"], "passed");
    assert!(Path::new(metadata["candidates"][0]["diff_path"].as_str().unwrap()).is_file());
    assert!(
        Path::new(
            metadata["candidates"][0]["workspace_path"]
                .as_str()
                .unwrap()
        )
        .join("cursor-routed.txt")
        .is_file()
    );
    assert_eq!(metadata["routing"]["version"], 1);
    assert_eq!(metadata["routing"]["task_features"]["language"], "rust");
    assert_eq!(metadata["routing"]["task_features"]["task_kind"], "bug_fix");
    assert_eq!(metadata["routing"]["selected_harness"], "cursor");
    assert_eq!(metadata["routing"]["successes"], 1);
    assert_eq!(metadata["routing"]["attempts"], 2);
    assert_eq!(metadata["routing"]["specificity"], 2);
    assert_eq!(metadata["routing"]["source"], "specific-public-evidence");
    assert_eq!(metadata["routing"]["dataset"], "rust-repairs");
    assert_eq!(metadata["routing"]["dataset_version"], "2026-08");
    assert_eq!(metadata["routing"]["model"], "fixture/specific-model");

    let connection = Connection::open(state.join("dispatch.db"))?;
    let routing_json: String =
        connection.query_row("SELECT routing_decision_json FROM runs", [], |row| {
            row.get(0)
        })?;
    assert_eq!(
        serde_json::from_str::<Value>(&routing_json)?,
        metadata["routing"]
    );

    let run_id = metadata["id"].as_str().unwrap();
    let shown = cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .args(["show", run_id])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let shown = String::from_utf8(shown)?;
    assert!(shown.contains("Routing\n"));
    assert!(shown.contains("selected: cursor"));
    assert!(shown.contains("Candidate A"));
    Ok(())
}

#[test]
fn route_and_explicit_harnesses_are_mutually_exclusive() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = source(temp.path());
    let state = temp.path().join("state");

    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .arg("run")
        .arg(&source)
        .args([
            "--task",
            "Fix the retry race.",
            "--route",
            "--harnesses",
            "codex,cursor",
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "cannot be used with '--harnesses",
        ));
    Ok(())
}

#[test]
fn routed_run_without_compatible_evidence_fails_before_creating_a_candidate() -> anyhow::Result<()>
{
    let temp = tempfile::tempdir()?;
    let source = source(temp.path());
    let state = temp.path().join("state");
    cache(
        &state,
        &[prior(
            "unsupported-source",
            "unsupported-dataset",
            "v1",
            "mini-SWE-agent",
            None,
            None,
            TaskKind::Unknown,
            1,
            1,
        )],
    )?;

    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .arg("run")
        .arg(&source)
        .args([
            "--task",
            "Fix the retry race.",
            "--route",
            "--allow-unsafe-local",
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "No compatible routing evidence is available.\nUse --harnesses to select harnesses explicitly.",
        ));

    assert_eq!(fs::read_dir(state.join("runs"))?.count(), 0);
    Ok(())
}

#[test]
fn routed_run_with_evidence_but_no_eligible_adapter_fails_before_execution() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = source(temp.path());
    let state = temp.path().join("state");
    let missing = temp.path().join("not-installed");
    configure(&source, &missing, &missing, &missing)?;
    cache(
        &state,
        &[prior(
            "harbor-framework/harbor",
            "terminal-bench",
            "2.0",
            "codex",
            None,
            None,
            TaskKind::Unknown,
            9,
            10,
        )],
    )?;

    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .arg("run")
        .arg(&source)
        .args([
            "--task",
            "Fix the retry race.",
            "--route",
            "--allow-unsafe-local",
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "Compatible routing evidence is available, but no predicted harness is execution-eligible locally.",
        ));

    assert_eq!(fs::read_dir(state.join("runs"))?.count(), 0);
    Ok(())
}

#[cfg(unix)]
#[test]
fn routed_run_preserves_the_existing_unsafe_local_acknowledgement() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = source(temp.path());
    let state = temp.path().join("state");
    let marker = temp.path().join("cursor-invocations");
    let cursor = temp.path().join("cursor-fixture");
    let missing = temp.path().join("not-installed");
    executable(&cursor, &marker)?;
    configure(&source, &cursor, &missing, &missing)?;
    cache(
        &state,
        &[prior(
            "harbor-framework/harbor",
            "terminal-bench",
            "2.0",
            "cursor",
            None,
            None,
            TaskKind::Unknown,
            3,
            4,
        )],
    )?;

    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .arg("run")
        .arg(&source)
        .args(["--task", "Fix the retry race.", "--route"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "explicitly accept the risk with --allow-unsafe-local",
        ));

    assert!(!marker.exists());
    assert_eq!(fs::read_dir(state.join("runs"))?.count(), 0);
    Ok(())
}
