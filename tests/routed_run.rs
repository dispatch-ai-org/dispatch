use std::{
    fs,
    path::{Path, PathBuf},
};

use assert_cmd::cargo_bin_cmd;
use chrono::{TimeZone, Utc};
use dispatch::{
    BenchmarkPrior, CandidateStatus, CheckStatus, RoutingHumanOutcome, TaskKind, TaskScope,
    db::Database, source::fingerprint_tree,
};
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
    configure_with_check(
        source,
        cursor,
        codex,
        claude,
        Some("test -f cursor-routed.txt"),
    )
}

fn configure_with_check(
    source: &Path,
    cursor: &Path,
    codex: &Path,
    claude: &Path,
    check: Option<&str>,
) -> anyhow::Result<()> {
    let checks = check.map_or_else(
        || "checks:\n  verify: []\n".to_owned(),
        |check| format!("checks:\n  verify:\n    - {check}\n"),
    );
    fs::write(
        source.join("dispatch.yml"),
        format!(
            r#"execution:
  timeout_secs: 2
  max_parallel: 3
{checks}harnesses:
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
    drop(connection);

    let run_id = metadata["id"].as_str().unwrap();
    let database = Database::open(state.join("dispatch.db"))?;
    let observation = database
        .routing_observation_for_run(run_id)?
        .expect("routed run records one observation");
    assert_eq!(observation.run_id, run_id);
    assert_eq!(observation.candidate_id, metadata["candidates"][0]["id"]);
    assert_eq!(observation.prediction.selected_harness, "cursor");
    assert_eq!(
        observation.prediction.task_features.language.as_deref(),
        Some("rust")
    );
    assert_eq!(observation.prediction.successes, 1);
    assert_eq!(observation.prediction.attempts, 2);
    assert_eq!(
        observation.harness_version.as_deref(),
        Some("cursor fixture 1.0")
    );
    assert_eq!(observation.model.as_deref(), Some("fixture-cursor"));
    assert_eq!(observation.candidate_status, CandidateStatus::Completed);
    assert_eq!(
        observation.verification.as_ref().unwrap()[0],
        CheckStatus::Passed
    );
    assert!(observation.human_evaluation.is_none());
    let observation_json = serde_json::to_string(&observation)?;
    assert!(!observation_json.contains(&source.display().to_string()));
    assert!(!observation_json.contains("Fix the retry race in the worker."));
    let observation_id = observation.id.clone();
    drop(database);

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
    assert!(shown.contains("Routing observation"));
    assert!(shown.contains("Process         completed"));
    assert!(shown.contains("Verification    PASS"));
    assert!(shown.contains("Human evaluation not recorded"));
    assert!(shown.contains("Candidate A"));

    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .args(["compare", run_id, "--winner", "A"])
        .assert()
        .success();
    let database = Database::open(state.join("dispatch.db"))?;
    let evaluated = database
        .routing_observation_for_run(run_id)?
        .expect("existing evaluation does not replace the observation");
    assert_eq!(evaluated.id, observation_id);
    assert!(evaluated.human_evaluation.is_none());
    let original_prediction = evaluated.prediction.clone();
    let original_verification = evaluated.verification.clone();
    drop(database);

    let explanation_path = temp.path().join("routed-evaluation.txt");
    let explanation = "Accepted after local review.\nKeep this verbatim.  \n";
    fs::write(&explanation_path, explanation)?;
    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .env(
            "DISPATCH_CLOUD_URL",
            "http://routing-must-not-contact.invalid",
        )
        .args(["evaluate", run_id, "--outcome", "accept"])
        .args(["--reason", "Correctness", "--reason", "tests"])
        .arg("--explanation-file")
        .arg(&explanation_path)
        .assert()
        .success();
    let database = Database::open(state.join("dispatch.db"))?;
    let accepted = database.routing_observation_for_run(run_id)?.unwrap();
    assert_eq!(accepted.id, observation_id);
    assert_eq!(accepted.prediction, original_prediction);
    assert_eq!(accepted.verification, original_verification);
    let human = accepted.human_evaluation.as_ref().unwrap();
    assert_eq!(human.outcome, RoutingHumanOutcome::Accepted);
    assert_eq!(human.reasons, ["correctness", "tests"]);
    assert_eq!(human.explanation.as_deref(), Some(explanation));
    let accepted_updated_at = accepted.updated_at;
    let accepted_evaluated_at = human.evaluated_at;
    drop(database);

    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .args(["evaluate", run_id, "--outcome", "accept"])
        .args(["--reason", "correctness", "--reason", "tests"])
        .arg("--explanation-file")
        .arg(&explanation_path)
        .assert()
        .success();
    let database = Database::open(state.join("dispatch.db"))?;
    let repeated = database.routing_observation_for_run(run_id)?.unwrap();
    assert_eq!(repeated.updated_at, accepted_updated_at);
    assert_eq!(
        repeated.human_evaluation.as_ref().unwrap().evaluated_at,
        accepted_evaluated_at
    );
    drop(database);

    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .args(["evaluate", run_id, "--outcome", "accept"])
        .args(["--reason", "cleaner-change"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "only valid for candidate comparison",
        ));

    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .args(["evaluate", run_id, "--outcome", "reject"])
        .args(["--reason", "correctness"])
        .args(["--explanation", "Needs another revision."])
        .assert()
        .success();
    let database = Database::open(state.join("dispatch.db"))?;
    let rejected = database.routing_observation_for_run(run_id)?.unwrap();
    assert_eq!(rejected.id, observation_id);
    assert_eq!(rejected.prediction, original_prediction);
    assert_eq!(rejected.verification, original_verification);
    assert_ne!(rejected.updated_at, accepted_updated_at);
    let human = rejected.human_evaluation.unwrap();
    assert_eq!(human.outcome, RoutingHumanOutcome::Rejected);
    assert_eq!(human.reasons, ["correctness"]);
    assert_eq!(
        human.explanation.as_deref(),
        Some("Needs another revision.")
    );

    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .args(["show", run_id])
        .assert()
        .success()
        .stdout(predicates::str::contains("Human evaluation rejected"))
        .stdout(predicates::str::contains("Reasons         correctness"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn routed_run_records_verification_failure_without_a_human_label() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = source(temp.path());
    let state = temp.path().join("state");
    let marker = temp.path().join("cursor-invocations");
    let cursor = temp.path().join("cursor-fixture");
    let missing = temp.path().join("not-installed");
    executable(&cursor, &marker)?;
    configure_with_check(
        &source,
        &cursor,
        &missing,
        &missing,
        Some("test -f deliberately-absent.txt"),
    )?;
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
        .env(
            "DISPATCH_CLOUD_URL",
            "http://routing-must-not-contact.invalid",
        )
        .arg("run")
        .arg(&source)
        .args([
            "--task",
            "Fix the retry race.",
            "--route",
            "--allow-unsafe-local",
        ])
        .assert()
        .success();

    let (_, metadata) = only_metadata(&state)?;
    let run_id = metadata["id"].as_str().unwrap();
    let database = Database::open(state.join("dispatch.db"))?;
    let observation = database.routing_observation_for_run(run_id)?.unwrap();
    assert_eq!(observation.candidate_status, CandidateStatus::Completed);
    assert_eq!(
        observation.verification.as_ref().unwrap()[0],
        CheckStatus::Failed
    );
    assert!(observation.human_evaluation.is_none());
    let original_prediction = observation.prediction.clone();
    drop(database);

    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .env(
            "DISPATCH_CLOUD_URL",
            "http://routing-must-not-contact.invalid",
        )
        .args(["evaluate", run_id, "--outcome", "accept"])
        .args(["--reason", "tests"])
        .assert()
        .success();
    let database = Database::open(state.join("dispatch.db"))?;
    let accepted = database.routing_observation_for_run(run_id)?.unwrap();
    assert_eq!(accepted.prediction, original_prediction);
    assert_eq!(accepted.candidate_status, CandidateStatus::Completed);
    assert_eq!(
        accepted.verification.as_ref().unwrap()[0],
        CheckStatus::Failed
    );
    assert_eq!(
        accepted.human_evaluation.unwrap().outcome,
        RoutingHumanOutcome::Accepted
    );
    drop(database);

    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .args(["evaluate", run_id, "--outcome", "reject"])
        .args(["--reason", "tests"])
        .assert()
        .success();
    let database = Database::open(state.join("dispatch.db"))?;
    let rejected = database.routing_observation_for_run(run_id)?.unwrap();
    assert_eq!(rejected.prediction, original_prediction);
    assert_eq!(
        rejected.verification.as_ref().unwrap()[0],
        CheckStatus::Failed
    );
    assert_eq!(
        rejected.human_evaluation.unwrap().outcome,
        RoutingHumanOutcome::Rejected
    );

    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .args(["show", run_id])
        .assert()
        .success()
        .stdout(predicates::str::contains("Verification    FAIL"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn routed_run_without_configured_verification_records_unknown() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = source(temp.path());
    let state = temp.path().join("state");
    let marker = temp.path().join("cursor-invocations");
    let cursor = temp.path().join("cursor-fixture");
    let missing = temp.path().join("not-installed");
    executable(&cursor, &marker)?;
    configure_with_check(&source, &cursor, &missing, &missing, None)?;
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
        .env(
            "DISPATCH_CLOUD_URL",
            "http://routing-must-not-contact.invalid",
        )
        .arg("run")
        .arg(&source)
        .args([
            "--task",
            "Fix the retry race.",
            "--route",
            "--allow-unsafe-local",
        ])
        .assert()
        .success();

    let (_, metadata) = only_metadata(&state)?;
    let run_id = metadata["id"].as_str().unwrap();
    let database = Database::open(state.join("dispatch.db"))?;
    let observation = database.routing_observation_for_run(run_id)?.unwrap();
    assert_eq!(observation.candidate_status, CandidateStatus::Completed);
    assert!(observation.verification.is_none());
    assert!(observation.human_evaluation.is_none());
    let observation_id = observation.id;
    let original_prediction = observation.prediction;
    drop(database);

    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .env(
            "DISPATCH_CLOUD_URL",
            "http://routing-must-not-contact.invalid",
        )
        .args(["evaluate", run_id, "--outcome", "accept"])
        .assert()
        .success();
    let database = Database::open(state.join("dispatch.db"))?;
    let accepted = database.routing_observation_for_run(run_id)?.unwrap();
    assert_eq!(accepted.prediction, original_prediction);
    assert!(accepted.verification.is_none());
    assert_eq!(
        accepted.human_evaluation.unwrap().outcome,
        RoutingHumanOutcome::Accepted
    );
    drop(database);

    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .args(["evaluate", run_id, "--outcome", "reject"])
        .assert()
        .success();
    let database = Database::open(state.join("dispatch.db"))?;
    let rejected = database.routing_observation_for_run(run_id)?.unwrap();
    assert_eq!(rejected.id, observation_id);
    assert_eq!(rejected.prediction, original_prediction);
    assert!(rejected.verification.is_none());
    assert_eq!(
        rejected.human_evaluation.unwrap().outcome,
        RoutingHumanOutcome::Rejected
    );
    drop(database);

    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .args(["show", run_id])
        .assert()
        .success()
        .stdout(predicates::str::contains("Verification    unknown"));
    Ok(())
}

#[test]
fn explicit_non_routed_run_creates_no_routing_observation() -> anyhow::Result<()> {
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
            "Exercise the explicit workflow.",
            "--harnesses",
            "fake-good",
        ])
        .assert()
        .success();

    let (_, metadata) = only_metadata(&state)?;
    let database = Database::open(state.join("dispatch.db"))?;
    assert!(
        database
            .routing_observation_for_run(metadata["id"].as_str().unwrap())?
            .is_none()
    );
    let run_id = metadata["id"].as_str().unwrap();
    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state)
        .args(["evaluate", run_id, "--outcome", "accept"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "This run was not predictively routed.\nUse `dispatch compare",
        ));
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
