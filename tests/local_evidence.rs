use std::{fs, io::ErrorKind, net::TcpListener, path::Path};

use anyhow::Result;
use assert_cmd::cargo_bin_cmd;
use chrono::{TimeZone, Utc};
use dispatch::{
    BenchmarkPrior, RoutingHumanEvaluation, RoutingHumanOutcome, RunRecord, TaskKind, TaskScope,
    db::{Database, SyncRecordType},
    router::rank_harnesses,
    source::fingerprint_tree,
    state::State,
    sync::preview_payload_for_revision,
};
use rusqlite::Connection;
use serde_json::json;

fn run(id: &str, source: &Path, harness: &str, process: &str, checks: &[&str]) -> RunRecord {
    let checks = checks
        .iter()
        .map(|status| {
            json!({
                "name": "verify", "phase": "verify", "command": "private check command",
                "status": status, "exit_code": null, "duration_ms": 1,
                "stdout_path": "private/check-out", "stderr_path": "private/check-err"
            })
        })
        .collect::<Vec<_>>();
    let candidate = json!({
        "id": format!("candidate-{id}"), "label": "A", "harness_id": harness,
        "harness_version": "fixture-v1", "model": "actual-model",
        "status": process, "workspace_path": "private/workspace", "prompt_path": "private/prompt",
        "stdout_path": "private/stdout", "stderr_path": "private/stderr", "diff_path": "private/diff",
        "duration_ms": 12, "exit_code": 0, "timed_out": process == "timed_out",
        "tokens": null, "cost_usd": null, "error": null,
        "diff_stats": {"files_changed": 0, "lines_added": 0, "lines_removed": 0},
        "checks": checks
    });
    serde_json::from_value(json!({
        "id": id, "task": "private task", "exact_prompt": "private prompt",
        "source_path": source, "source_kind": "directory", "source_git_head": null,
        "source_fingerprint": "same-content-in-two-different-repositories",
        "baseline_path": "private/baseline", "baseline_commit": "baseline",
        "status": "ready_for_evaluation", "created_at": "2026-09-04T12:00:01Z",
        "completed_at": "2026-09-04T12:00:02Z",
        "environment": {
            "dispatch_version": "0.1.1", "os": "test", "architecture": "test",
            "execution_backend": "local", "timeout_secs": 30, "cpus": 1.0,
            "memory": "1g", "max_parallel": 1
        },
        "routing": {
            "version": 1, "selected_harness": harness,
            "task_features": {"language": "rust", "task_kind": "bug_fix", "scope": "unknown"},
            "successes": 37, "attempts": 50, "specificity": 0,
            "source": "harbor-framework/harbor", "dataset": "terminal-bench",
            "dataset_version": "2.0", "model": "public-model"
        },
        "candidates": [candidate],
        "evaluation": null, "applied_candidate": null
    }))
    .unwrap()
}

fn human(outcome: RoutingHumanOutcome) -> RoutingHumanEvaluation {
    RoutingHumanEvaluation {
        outcome,
        reasons: vec!["correctness".into()],
        explanation: Some("private human explanation".into()),
        evaluated_at: Utc.with_ymd_and_hms(2026, 9, 4, 12, 0, 3).unwrap(),
    }
}

fn prior(harness: &str, successes: u64) -> BenchmarkPrior {
    BenchmarkPrior {
        source: "harbor-framework/harbor".into(),
        dataset: "terminal-bench".into(),
        dataset_version: "2.0".into(),
        harness: harness.into(),
        model: None,
        language: None,
        task_kind: TaskKind::Unknown,
        scope: TaskScope::Unknown,
        successes,
        attempts: 100,
        updated_at: Utc::now(),
    }
}

#[test]
fn human_and_mechanical_counts_are_independent_and_unevaluated_is_not_a_vote() -> Result<()> {
    let source = Path::new("local-source");
    let mut db = Database::open_in_memory()?;
    for (id, checks, outcome) in [
        (
            "accepted",
            vec!["failed"],
            Some(RoutingHumanOutcome::Accepted),
        ),
        (
            "rejected",
            vec!["passed"],
            Some(RoutingHumanOutcome::Rejected),
        ),
        ("unevaluated", vec!["passed", "passed"], None),
    ] {
        let run = run(id, source, "codex", "completed", &checks);
        db.sync_run(&run)?;
        if let Some(outcome) = outcome {
            db.save_routing_human_evaluation(id, &human(outcome))?;
        }
    }
    let groups = db.local_routing_evidence(source)?;
    assert_eq!(groups.len(), 1);
    let e = &groups[0];
    assert_eq!(e.total_runs, 3);
    assert_eq!(
        (
            e.human_evaluated,
            e.human_accepted,
            e.human_rejected,
            e.human_unevaluated
        ),
        (2, 1, 1, 1)
    );
    assert_eq!(
        (
            e.verification_observed,
            e.verification_passed,
            e.verification_failed
        ),
        (3, 2, 1)
    );
    assert_eq!(e.process_completed, 3);
    Ok(())
}

#[test]
fn process_timeout_verification_timeout_not_run_and_unknown_stay_distinct() -> Result<()> {
    let source = Path::new("source");
    let mut db = Database::open_in_memory()?;
    for (id, process, checks) in [
        ("no-checks", "completed", vec![]),
        ("check-not-run", "completed", vec!["passed", "not_run"]),
        ("check-timeout", "completed", vec!["passed", "timed_out"]),
        ("process-timeout", "timed_out", vec![]),
        ("process-failure", "failed", vec![]),
        ("cancelled", "cancelled", vec![]),
        ("missing", "missing_harness", vec![]),
        ("not-terminal", "running", vec![]),
    ] {
        db.sync_run(&run(id, source, "cursor", process, &checks))?;
    }
    let e = db.local_routing_evidence(source)?.remove(0);
    assert_eq!(e.total_runs, 7);
    assert_eq!((e.human_evaluated, e.human_unevaluated), (0, 7));
    assert_eq!(
        (
            e.verification_observed,
            e.verification_passed,
            e.verification_failed
        ),
        (1, 0, 0)
    );
    assert_eq!(
        (
            e.verification_timed_out,
            e.verification_not_run,
            e.verification_unknown
        ),
        (1, 1, 5)
    );
    assert_eq!(
        (e.process_completed, e.process_failed, e.process_timed_out),
        (3, 1, 1)
    );
    assert_eq!((e.process_cancelled, e.process_missing_harness), (1, 1));
    Ok(())
}

#[test]
fn groups_use_exact_morphology_harness_and_source_not_content_or_version() -> Result<()> {
    let source = Path::new("source-one");
    let other_source = Path::new("source-two");
    let mut db = Database::open_in_memory()?;
    let base = run("base", source, "codex", "completed", &[]);
    db.sync_run(&base)?;
    let mut next = run("next", source, "codex", "completed", &[]);
    next.source_fingerprint = "new-source-content".into();
    next.candidates[0].model = Some("new-model".into());
    next.candidates[0].harness_version = Some("fixture-v2".into());
    db.sync_run(&next)?;
    db.sync_run(&next)?; // Reprocessing is not a new attempt.
    db.sync_run(&run("cursor", source, "cursor", "completed", &[]))?;
    db.sync_run(&run("other-repo", other_source, "codex", "completed", &[]))?;
    for dimension in ["language", "kind", "scope"] {
        let mut different = run(dimension, source, "codex", "completed", &[]);
        let features = &mut different.routing.as_mut().unwrap().task_features;
        match dimension {
            "language" => features.language = None,
            "kind" => features.task_kind = TaskKind::Feature,
            _ => features.scope = TaskScope::Localized,
        }
        db.sync_run(&different)?;
    }
    let groups = db.local_routing_evidence(source)?;
    assert_eq!(groups.len(), 5);
    assert_eq!(groups.iter().map(|e| e.total_runs).sum::<u64>(), 6);
    let base_group = groups
        .iter()
        .find(|e| {
            e.harness == "codex" && e.task_features == base.routing.as_ref().unwrap().task_features
        })
        .unwrap();
    assert_eq!(base_group.total_runs, 2);
    assert_eq!(db.local_routing_evidence(other_source)?[0].total_runs, 1);
    assert_eq!(groups, db.local_routing_evidence(source)?);
    let observation = db.routing_observation_for_run("next")?.unwrap();
    assert_eq!(observation.model.as_deref(), Some("new-model"));
    assert_eq!(observation.harness_version.as_deref(), Some("fixture-v2"));
    Ok(())
}

#[test]
fn latest_numeric_revision_wins_without_counting_history_or_mutating_records() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let state = State::discover(Some(temp.path().join("state")))?;
    let source = temp.path().join("source");
    fs::create_dir(&source)?;
    let mut db = Database::open(state.db_path())?;
    for (id, outcomes) in [
        (
            "FIRST",
            [RoutingHumanOutcome::Accepted, RoutingHumanOutcome::Rejected],
        ),
        (
            "SECOND",
            [RoutingHumanOutcome::Rejected, RoutingHumanOutcome::Accepted],
        ),
    ] {
        let run = run(id, &source, "codex", "completed", &[]);
        db.sync_run(&run)?;
        state.save_run(&run)?;
        db.enable_sync(
            "fixture-contributor",
            Utc.with_ymd_and_hms(2026, 9, 4, 12, 0, 0).unwrap(),
        )?;
        let (_, parent) = preview_payload_for_revision(
            &state,
            id,
            Some(SyncRecordType::RoutingObservationV1),
            None,
        )?;
        db.mark_sync_succeeded(&format!("routing-observation-{id}"), Utc::now())?;
        for outcome in outcomes {
            db.save_routing_human_evaluation(id, &human(outcome))?;
        }
        let events = db.routing_feedback_for_run(id)?;
        assert_eq!(events.len(), 2);
        let connection = Connection::open(state.db_path())?;
        // Historical local snapshot is stale. Revision, not parent state or clock time, wins.
        connection.execute("UPDATE routing_observations SET observation_json = json_set(observation_json, '$.human_evaluation', NULL) WHERE run_id = ?1", [id])?;
        connection.execute("UPDATE routing_feedback_events SET revision = 10, payload_json = json_set(payload_json, '$.revision', 10) WHERE id = ?1", [&events[1].feedback_event_id])?;
        let history = db.routing_feedback_for_run(id)?;
        let before = db.routing_observation_for_run(id)?;
        let groups = db.local_routing_evidence(&source)?;
        let e = &groups[0];
        if id == "FIRST" {
            assert_eq!((e.total_runs, e.human_rejected), (1, 1));
        } else {
            assert_eq!(
                (
                    e.total_runs,
                    e.human_evaluated,
                    e.human_accepted,
                    e.human_rejected
                ),
                (2, 2, 1, 1)
            );
        }
        assert_eq!(db.routing_feedback_for_run(id)?, history);
        assert_eq!(db.routing_observation_for_run(id)?, before);
        assert_eq!(db.synced_routing_payload(id)?.unwrap(), parent);
    }
    Ok(())
}

#[test]
fn local_inspection_excludes_public_priors_and_explicit_runs_and_cannot_change_router() -> Result<()>
{
    let source = Path::new("source");
    let mut db = Database::open_in_memory()?;
    db.upsert_benchmark_prior(&prior("codex", 90))?;
    db.upsert_benchmark_prior(&prior("cursor", 50))?;
    let mut explicit = run("explicit", source, "codex", "completed", &["passed"]);
    explicit.routing = None;
    db.sync_run(&explicit)?;
    assert!(db.local_routing_evidence(source)?.is_empty());
    let routed = run("routed", source, "cursor", "completed", &["passed"]);
    let features = &routed.routing.as_ref().unwrap().task_features;
    let harnesses = vec!["codex".into(), "cursor".into()];
    let before = rank_harnesses(&db, features, &harnesses)?;
    db.sync_run(&routed)?;
    db.save_routing_human_evaluation(&routed.id, &human(RoutingHumanOutcome::Accepted))?;
    assert_eq!(db.local_routing_evidence(source)?[0].total_runs, 1);
    assert_eq!(rank_harnesses(&db, features, &harnesses)?, before);
    assert_eq!(before[0].harness, "codex");
    Ok(())
}

#[test]
fn cli_is_source_scoped_offline_unranked_and_keeps_recommend_output_identical() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("source");
    fs::create_dir(&source)?;
    fs::write(source.join("main.rs"), "fn main() {}\n")?;
    // Inspecting evidence must not load configuration or run verification/harnesses.
    fs::write(
        source.join("dispatch.yml"),
        "deliberately invalid configuration: [",
    )?;
    let canonical = source.canonicalize()?;
    let state = temp.path().join("state");
    let mut db = Database::open(state.join("dispatch.db"))?;
    db.upsert_benchmark_prior(&prior("codex", 90))?;
    db.upsert_benchmark_prior(&prior("cursor", 50))?;
    let recommend = || {
        cargo_bin_cmd!("dispatch")
            .env("DISPATCH_HOME", &state)
            .arg("recommend")
            .arg(&source)
            .args(["--task", "Fix the bug"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone()
    };
    let before = recommend();
    for (id, harness, outcome) in [
        ("one", "codex", RoutingHumanOutcome::Rejected),
        ("two", "cursor", RoutingHumanOutcome::Accepted),
    ] {
        db.sync_run(&run(id, &canonical, harness, "completed", &["passed"]))?;
        db.save_routing_human_evaluation(id, &human(outcome))?;
    }
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    db.enable_sync("fixture-contributor", Utc::now())?;
    db.set_sync_token("fixture-token-not-for-transmission")?;
    let fingerprint = fingerprint_tree(&source)?;
    let output = cargo_bin_cmd!("dispatch")
        .env("DISPATCH_HOME", &state)
        .env(
            "DISPATCH_CLOUD_URL",
            format!("http://{}", listener.local_addr()?),
        )
        .env("PATH", "") // No external programs are needed.
        .args(["evidence", "local"])
        .arg(source.join("."))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(output)?;
    assert!(text.contains("Local routing evidence"));
    assert!(text.contains("rust / bug_fix / unknown"));
    assert!(text.contains("accepted: 0/1 (0.0%)"));
    assert!(text.contains("accepted: 1/1 (100.0%)"));
    assert!(text.contains("passed: 1/1 (100.0%)"));
    assert!(text.find("codex").unwrap() < text.find("cursor").unwrap());
    assert!(text.contains("not a ranking"));
    assert!(!text.contains("confidence"));
    assert!(!text.contains(&canonical.display().to_string()));
    for private in [
        "private task",
        "private prompt",
        "private human explanation",
        "private/check",
    ] {
        assert!(!text.contains(private));
    }
    assert_eq!(fingerprint_tree(&source)?, fingerprint);
    assert_eq!(listener.accept().unwrap_err().kind(), ErrorKind::WouldBlock);
    assert_eq!(recommend(), before);
    assert!(db.pending_sync_records()?.is_empty());
    Ok(())
}

#[test]
fn cli_no_observations_is_successful_without_creating_state_or_source_files() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("source");
    fs::create_dir(&source)?;
    let state = source.join(".dispatch");
    cargo_bin_cmd!("dispatch")
        .arg("--state-dir")
        .arg(&state)
        .args(["evidence", "local"])
        .arg(&source)
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "No local routing observations for this source.",
        ));
    assert!(!state.exists());
    let mut db = Database::open(state.join("dispatch.db"))?;
    db.sync_run(&run(
        "unknown",
        &source.canonicalize()?,
        "codex",
        "completed",
        &[],
    ))?;
    cargo_bin_cmd!("dispatch")
        .arg("--state-dir")
        .arg(&state)
        .args(["evidence", "local"])
        .arg(&source)
        .assert()
        .success()
        .stdout(predicates::str::contains("accepted: not observed (0/0)"))
        .stdout(predicates::str::contains("passed: not observed (0/0)"))
        .stdout(predicates::str::contains("unevaluated: 1"))
        .stdout(predicates::str::contains("verification unknown: 1"));
    Ok(())
}

#[test]
fn empty_verification_is_unknown_and_mixed_checks_follow_documented_precedence() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("dispatch.db");
    let source = Path::new("source");
    let mut db = Database::open(&path)?;
    for (id, checks) in [
        ("empty", vec![]),
        (
            "mixed-fail",
            vec!["passed", "failed", "timed_out", "not_run"],
        ),
        ("mixed-timeout", vec!["timed_out", "not_run"]),
    ] {
        db.sync_run(&run(id, source, "codex", "completed", &checks))?;
    }
    Connection::open(&path)?.execute(
        "UPDATE routing_observations SET observation_json = json_set(observation_json, '$.verification', json('[]')) WHERE run_id = 'empty'", [],
    )?;
    let e = db.local_routing_evidence(source)?.remove(0);
    assert_eq!((e.total_runs, e.verification_observed), (3, 2));
    assert_eq!(
        (
            e.verification_failed,
            e.verification_timed_out,
            e.verification_unknown
        ),
        (1, 1, 1)
    );
    assert_eq!(e.verification_passed, 0);
    Ok(())
}

#[test]
fn invalid_feedback_fails_closed_instead_of_inventing_a_human_outcome() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("dispatch.db");
    let source = Path::new("source");
    let mut db = Database::open(&path)?;
    db.sync_run(&run("run", source, "codex", "completed", &[]))?;
    let connection = Connection::open(&path)?;
    let event = json!({
        "schema_version": 1, "feedback_event_id": "event", "observation_id": "routing-observation-run",
        "contributor_id": "fixture", "revision": 1,
        "consent": {"scope": "routing-observation-v1", "enabled_at": "2026-09-04T12:00:00Z"},
        "outcome": "accept", "reasons": [], "explanation": null, "evaluated_at": "2026-09-04T12:00:03Z"
    });
    for (key, value) in [
        ("outcome", json!("winner")),
        ("revision", json!(2)),
        ("schema_version", json!(2)),
        ("observation_id", json!("wrong-parent")),
        ("feedback_event_id", json!("wrong-event")),
    ] {
        let mut invalid = event.clone();
        invalid[key] = value;
        connection.execute(
            "INSERT OR REPLACE INTO routing_feedback_events(id, observation_id, run_id, revision, created_at, payload_json) VALUES ('event', 'routing-observation-run', 'run', 1, '2026-09-04T12:00:03Z', ?1)",
            [invalid.to_string()],
        )?;
        assert!(
            db.local_routing_evidence(source).is_err(),
            "invalid {key} was accepted"
        );
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn local_acceptance_cannot_change_routed_execution_selection() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir()?;
    let source = temp.path().join("source");
    fs::create_dir(&source)?;
    fs::write(source.join("main.rs"), "fn main() {}\n")?;
    let executable = temp.path().join("fixture-agent");
    fs::write(
        &executable,
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then printf 'fixture 1.0\\n'; exit 0; fi\nprintf '{\"type\":\"result\",\"model\":\"fixture-model\"}\\n'\n",
    )?;
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))?;
    fs::write(
        source.join("dispatch.yml"),
        format!(
            "harnesses:\n  cursor:\n    executable: '{}'\n  codex:\n    executable: '{}'\nchecks:\n  verify: []\n",
            executable.display(),
            executable.display()
        ),
    )?;
    let state = State::discover(Some(temp.path().join("state")))?;
    let mut db = Database::open(state.db_path())?;
    db.upsert_benchmark_prior(&prior("cursor", 90))?;
    db.upsert_benchmark_prior(&prior("codex", 50))?;
    let execute = || -> Result<()> {
        let stdout = cargo_bin_cmd!("dispatch")
            .arg("--state-dir")
            .arg(&state.root)
            .env(
                "DISPATCH_CLOUD_URL",
                "http://cloud-must-not-be-used.invalid",
            )
            .arg("run")
            .arg(&source)
            .args(["--route", "--task", "Fix the bug", "--allow-unsafe-local"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let stdout = String::from_utf8(stdout)?;
        assert!(stdout.contains("Agent\n  Cursor"));
        assert!(stdout.contains("Selection\n  Evidence-based"));
        let id = stdout
            .lines()
            .find_map(|line| line.strip_prefix("RUN "))
            .unwrap();
        let run = state.load_run(id)?;
        assert_eq!(run.candidates.len(), 1);
        assert_eq!(run.candidates[0].harness_id, "cursor");
        Ok(())
    };
    execute()?;
    let canonical = source.canonicalize()?;
    for id in ["local1", "local2", "local3"] {
        let mut local = run(id, &canonical, "codex", "completed", &["passed"]);
        local.routing.as_mut().unwrap().task_features =
            dispatch::classifier::classify_task(&canonical, "Fix the bug")?;
        db.sync_run(&local)?;
        db.save_routing_human_evaluation(id, &human(RoutingHumanOutcome::Accepted))?;
    }
    let groups = db.local_routing_evidence(&canonical)?;
    assert_eq!(
        groups
            .iter()
            .find(|e| e.harness == "codex")
            .unwrap()
            .human_accepted,
        3
    );
    execute()?;
    Ok(())
}
