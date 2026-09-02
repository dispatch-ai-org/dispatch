use std::path::PathBuf;

use chrono::{DateTime, TimeZone, Utc};
use dispatch::{
    CandidateRecord, CandidateStatus, CheckPhase, CheckResult, CheckStatus, DiffStats,
    EnvironmentRecord, EvaluationOutcome, EvaluationRecord, RoutingDecision,
    RoutingHumanEvaluation, RoutingHumanOutcome, RunRecord, RunStatus, SourceKind, TaskFeatures,
    TaskKind, TaskScope, db::Database,
};

fn at(second: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, second).unwrap()
}

fn decision() -> RoutingDecision {
    RoutingDecision {
        version: 1,
        task_features: TaskFeatures {
            language: Some("rust".into()),
            task_kind: TaskKind::BugFix,
            scope: TaskScope::Unknown,
        },
        selected_harness: "cursor".into(),
        successes: 37,
        attempts: 50,
        specificity: 2,
        source: "harbor-framework/harbor".into(),
        dataset: "terminal-bench".into(),
        dataset_version: "2.0".into(),
        model: Some("benchmark/model".into()),
    }
}

fn check(status: CheckStatus) -> CheckResult {
    CheckResult {
        name: "verify".into(),
        phase: CheckPhase::Verify,
        command: "cargo test".into(),
        status,
        exit_code: Some(0),
        duration_ms: 12,
        stdout_path: PathBuf::from("/private/check.stdout"),
        stderr_path: PathBuf::from("/private/check.stderr"),
    }
}

fn candidate(status: CandidateStatus, checks: Vec<CheckResult>) -> CandidateRecord {
    CandidateRecord {
        id: "candidate-id".into(),
        label: "A".into(),
        harness_id: "cursor".into(),
        harness_version: Some("cursor-1.2.3".into()),
        model: Some("execution/model".into()),
        status,
        workspace_path: PathBuf::from("/private/workspace"),
        prompt_path: PathBuf::from("/private/prompt"),
        stdout_path: PathBuf::from("/private/stdout"),
        stderr_path: PathBuf::from("/private/stderr"),
        diff_path: PathBuf::from("/private/diff"),
        duration_ms: 1_250,
        exit_code: Some(0),
        timed_out: false,
        tokens: None,
        token_semantics: None,
        cost_usd: None,
        error: None,
        diff_stats: DiffStats::default(),
        checks,
    }
}

fn run(id: &str, routing: Option<RoutingDecision>, mut candidate: CandidateRecord) -> RunRecord {
    candidate.id = format!("candidate-{id}");
    RunRecord {
        id: id.into(),
        task: "private task text that must not enter the observation".into(),
        exact_prompt: "private exact prompt".into(),
        source_path: PathBuf::from("/private/source"),
        source_kind: SourceKind::Directory,
        source_git_head: None,
        source_fingerprint: "private-fingerprint".into(),
        baseline_path: PathBuf::from("/private/baseline"),
        baseline_commit: "private-commit".into(),
        status: RunStatus::ReadyForEvaluation,
        created_at: at(1),
        completed_at: Some(at(2)),
        environment: EnvironmentRecord {
            dispatch_version: "0.1.1".into(),
            os: "test".into(),
            architecture: "test".into(),
            execution_backend: "local".into(),
            timeout_secs: 30,
            cpus: 1.0,
            memory: "1g".into(),
            max_parallel: 1,
            docker_image: None,
            resource_limits_enforced: false,
            unsafe_local: true,
            forwarded_env: Vec::new(),
        },
        baseline_checks: Vec::new(),
        candidates: vec![candidate],
        routing,
        evaluation: None,
        applied_candidate: None,
    }
}

#[test]
fn routed_run_creates_one_idempotent_prediction_and_outcome_snapshot() -> anyhow::Result<()> {
    let mut database = Database::open_in_memory()?;
    let routing = decision();
    let run = run(
        "run-routed",
        Some(routing.clone()),
        candidate(CandidateStatus::Completed, vec![check(CheckStatus::Passed)]),
    );

    database.sync_run(&run)?;
    let first = database
        .routing_observation_for_run(&run.id)?
        .expect("routed terminal candidate creates an observation");
    assert_eq!(first.id, "routing-observation-run-routed");
    assert_eq!(first.run_id, run.id);
    assert_eq!(first.candidate_id, "candidate-run-routed");
    assert_eq!(first.prediction, routing);
    assert_eq!(first.prediction.task_features, decision().task_features);
    assert_eq!(first.prediction.source, "harbor-framework/harbor");
    assert_eq!(first.prediction.dataset, "terminal-bench");
    assert_eq!(first.prediction.dataset_version, "2.0");
    assert_eq!(first.prediction.model.as_deref(), Some("benchmark/model"));
    assert_eq!(first.harness_version.as_deref(), Some("cursor-1.2.3"));
    assert_eq!(first.model.as_deref(), Some("execution/model"));
    assert_eq!(first.candidate_status, CandidateStatus::Completed);
    assert!(!first.timed_out);
    assert_eq!(first.verification.as_ref().unwrap().len(), 1);
    assert_eq!(first.verification.as_ref().unwrap()[0], CheckStatus::Passed);
    assert!(first.human_evaluation.is_none());

    database.sync_run(&run)?;
    let second = database
        .routing_observation(&first.id)?
        .expect("stable observation ID remains queryable");
    assert_eq!(second.id, first.id);
    assert_eq!(second.created_at, first.created_at);
    assert_eq!(second.run_id, first.run_id);
    Ok(())
}

#[test]
fn routed_human_evaluation_updates_only_the_existing_observation() -> anyhow::Result<()> {
    let mut database = Database::open_in_memory()?;
    let run = run(
        "run-human",
        Some(decision()),
        candidate(CandidateStatus::Completed, vec![check(CheckStatus::Passed)]),
    );
    database.sync_run(&run)?;
    let before = database.routing_observation_for_run(&run.id)?.unwrap();

    let accepted = RoutingHumanEvaluation {
        outcome: RoutingHumanOutcome::Accepted,
        reasons: vec!["correctness".into(), "tests".into()],
        explanation: Some("The result is ready to use.".into()),
        evaluated_at: at(5),
    };
    let first = database.save_routing_human_evaluation(&run.id, &accepted)?;
    assert_eq!(first.id, before.id);
    assert_eq!(first.created_at, before.created_at);
    assert_eq!(first.updated_at, at(5));
    assert_eq!(first.prediction, before.prediction);
    assert_eq!(first.candidate_status, before.candidate_status);
    assert_eq!(first.verification, before.verification);
    assert_eq!(first.human_evaluation.as_ref(), Some(&accepted));

    let repeated = database.save_routing_human_evaluation(
        &run.id,
        &RoutingHumanEvaluation {
            evaluated_at: at(6),
            ..accepted.clone()
        },
    )?;
    assert_eq!(repeated, first);

    let rejected = RoutingHumanEvaluation {
        outcome: RoutingHumanOutcome::Rejected,
        reasons: vec!["correctness".into()],
        explanation: Some("The result needs more work.".into()),
        evaluated_at: at(7),
    };
    let revised = database.save_routing_human_evaluation(&run.id, &rejected)?;
    assert_eq!(revised.id, first.id);
    assert_eq!(revised.created_at, first.created_at);
    assert_eq!(revised.updated_at, at(7));
    assert_eq!(revised.prediction, first.prediction);
    assert_eq!(revised.candidate_status, first.candidate_status);
    assert_eq!(revised.verification, first.verification);
    assert_eq!(revised.human_evaluation.as_ref(), Some(&rejected));
    Ok(())
}

#[test]
fn explicit_and_historical_runs_do_not_manufacture_observations() -> anyhow::Result<()> {
    let mut database = Database::open_in_memory()?;
    let explicit = run(
        "run-explicit",
        None,
        candidate(CandidateStatus::Completed, vec![check(CheckStatus::Passed)]),
    );
    database.sync_run(&explicit)?;
    assert!(
        database
            .routing_observation_for_run(&explicit.id)?
            .is_none()
    );

    let mut historical_json = serde_json::to_value(&explicit)?;
    historical_json.as_object_mut().unwrap().remove("routing");
    let historical: RunRecord = serde_json::from_value(historical_json)?;
    database.sync_run(&historical)?;
    assert!(
        database
            .routing_observation_for_run(&historical.id)?
            .is_none()
    );
    Ok(())
}

#[test]
fn verification_unknown_failure_timeout_and_process_failure_remain_distinct() -> anyhow::Result<()>
{
    let mut database = Database::open_in_memory()?;

    let failed_verification = run(
        "run-check-failed",
        Some(decision()),
        candidate(CandidateStatus::Completed, vec![check(CheckStatus::Failed)]),
    );
    database.sync_run(&failed_verification)?;
    let failed = database
        .routing_observation_for_run(&failed_verification.id)?
        .unwrap();
    assert_eq!(failed.candidate_status, CandidateStatus::Completed);
    assert_eq!(failed.verification.unwrap()[0], CheckStatus::Failed);
    assert!(failed.human_evaluation.is_none());

    let unknown_verification = run(
        "run-check-unknown",
        Some(decision()),
        candidate(CandidateStatus::Completed, Vec::new()),
    );
    database.sync_run(&unknown_verification)?;
    let unknown = database
        .routing_observation_for_run(&unknown_verification.id)?
        .unwrap();
    assert_eq!(unknown.candidate_status, CandidateStatus::Completed);
    assert!(unknown.verification.is_none());

    let mut timeout_candidate = candidate(CandidateStatus::TimedOut, Vec::new());
    timeout_candidate.exit_code = None;
    timeout_candidate.timed_out = true;
    let timed_out = run("run-timeout", Some(decision()), timeout_candidate);
    database.sync_run(&timed_out)?;
    let timeout = database
        .routing_observation_for_run(&timed_out.id)?
        .unwrap();
    assert_eq!(timeout.candidate_status, CandidateStatus::TimedOut);
    assert!(timeout.timed_out);
    assert!(timeout.verification.is_none());

    let mut failed_candidate = candidate(CandidateStatus::Failed, Vec::new());
    failed_candidate.exit_code = Some(2);
    let process_failed = run("run-process-failed", Some(decision()), failed_candidate);
    database.sync_run(&process_failed)?;
    let process_failure = database
        .routing_observation_for_run(&process_failed.id)?
        .unwrap();
    assert_eq!(process_failure.candidate_status, CandidateStatus::Failed);
    assert_eq!(process_failure.exit_code, Some(2));
    assert!(!process_failure.timed_out);
    assert!(process_failure.verification.is_none());
    Ok(())
}

#[test]
fn existing_single_candidate_evaluation_remains_separate_from_the_observation() -> anyhow::Result<()>
{
    let mut database = Database::open_in_memory()?;
    let run = run(
        "run-evaluated",
        Some(decision()),
        candidate(CandidateStatus::Completed, vec![check(CheckStatus::Passed)]),
    );
    database.sync_run(&run)?;
    let before = database.routing_observation_for_run(&run.id)?.unwrap();
    assert!(before.human_evaluation.is_none());

    let evaluation = EvaluationRecord {
        outcome: EvaluationOutcome::Candidate("A".into()),
        reasons: vec!["correctness".into()],
        explanation: Some("The developer selected the only candidate.".into()),
        created_at: at(5),
        blind: true,
    };
    database.save_evaluation(&run.id, &evaluation)?;

    let after = database.routing_observation_for_run(&run.id)?.unwrap();
    assert_eq!(after.id, before.id);
    assert_eq!(after.created_at, before.created_at);
    assert!(after.human_evaluation.is_none());
    assert_eq!(
        database.evaluation_id(&run.id)?.as_deref(),
        Some("evaluation-run-evaluated")
    );
    Ok(())
}
