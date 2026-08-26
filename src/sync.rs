use std::{
    env, fs,
    io::Write,
    process::{Command, Stdio},
    thread,
};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::{
    CandidateRecord, CandidateStatus, CheckPhase, CheckResult, CheckStatus, EvaluationOutcome,
    RunRecord, VERSION,
    db::{Database, SyncSettings},
    executor::{trusted_host_executable, trusted_host_path},
    state::State,
};

pub const EVALUATION_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_CLOUD_URL: &str = "https://api.dispatch.dev";
const EVALUATION_PATH: &str = "/v1/evaluations";
const MAX_TOKEN_LENGTH: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvaluationEnvelopeV1 {
    pub schema_version: u32,
    pub evaluation_id: String,
    pub contributor_id: String,
    pub consent: ConsentMetadataV1,
    pub run: SyncRunV1,
    pub task: SyncTaskV1,
    pub project: SyncProjectV1,
    pub baseline_verification: VerificationSummaryV1,
    pub candidates: Vec<SyncCandidateV1>,
    pub human_evaluation: HumanEvaluationV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConsentMetadataV1 {
    pub scope: String,
    pub enabled_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SyncRunV1 {
    pub run_id: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub dispatch_version: String,
    pub os: String,
    pub architecture: String,
    pub execution_backend: String,
    pub timeout_secs: u64,
    pub cpus: f64,
    pub memory: String,
    pub resource_limits_enforced: bool,
    pub unsafe_local: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SyncTaskV1 {
    pub text: String,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SyncProjectV1 {
    pub source_type: String,
    pub languages: Option<Vec<String>>,
    pub file_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SyncCandidateV1 {
    pub candidate_id: String,
    pub label: String,
    pub harness_id: String,
    pub harness_version: Option<String>,
    pub model: Option<String>,
    pub status: String,
    pub runtime_ms: u64,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub process_failure: bool,
    pub task_tokens: Option<u64>,
    pub token_semantics: Option<String>,
    pub cost_usd: Option<f64>,
    pub files_changed: u64,
    pub lines_added: u64,
    pub lines_removed: u64,
    pub verification: VerificationSummaryV1,
    pub human_outcome: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerificationSummaryV1 {
    pub state: String,
    pub checks: Vec<VerificationResultV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerificationResultV1 {
    pub name: String,
    pub phase: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HumanEvaluationV1 {
    pub outcome: String,
    pub selected_candidate: Option<String>,
    pub reasons: Vec<String>,
    pub explanation: Option<String>,
    pub blind: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub synced: usize,
    pub failed: usize,
}

pub fn enable(state: &State) -> Result<()> {
    state.initialize()?;
    let mut database = Database::open(state.db_path())?;
    let settings = database.enable_sync(&Ulid::new().to_string(), Utc::now())?;
    let queued = queue_all(state, &database, &settings)?;
    let available = database.evaluation_count()?;

    println!("Evaluation sync enabled.");
    println!(
        "Share evaluation records with Dispatch to improve harness statistics and future routing."
    );
    println!(
        "Shared fields include task descriptions, harness/version/model metadata, execution and verification results, and your evaluation and reasoning."
    );
    println!(
        "Source code, full diffs, logs, local paths, environment values, and credentials are not uploaded."
    );
    println!("{available} completed evaluation(s) are available; {queued} newly queued.");
    println!("Review one with: dispatch sync preview <run-id>");
    Ok(())
}

pub fn disable(state: &State) -> Result<()> {
    state.initialize()?;
    Database::open(state.db_path())?.disable_sync()?;
    println!("Evaluation sync disabled. No evaluation uploads will be attempted.");
    Ok(())
}

pub fn status(state: &State) -> Result<()> {
    state.initialize()?;
    let database = Database::open(state.db_path())?;
    let settings = database.sync_settings()?;
    let counts = database.sync_outbox_counts()?;
    println!(
        "Evaluation sync: {}",
        if settings.enabled {
            "enabled"
        } else {
            "disabled"
        }
    );
    if let Some(contributor_id) = settings.contributor_id {
        println!("Contributor ID: {contributor_id}");
    }
    println!("Completed evaluations: {}", database.evaluation_count()?);
    println!(
        "Outbox: {} pending, {} failed, {} synced",
        counts.pending, counts.failed, counts.synced
    );
    println!("Cloud URL: {}", configured_cloud_url()?);
    Ok(())
}

pub fn token_set(state: &State, token: &str) -> Result<()> {
    validate_ingestion_token(token)?;
    state.initialize()?;
    Database::open(state.db_path())?.set_sync_token(token)?;
    println!("Dispatch Cloud ingestion token configured.");
    Ok(())
}

pub fn token_status(state: &State) -> Result<()> {
    state.initialize()?;
    let configured = Database::open(state.db_path())?.sync_token()?.is_some();
    println!(
        "Dispatch Cloud ingestion token: {}",
        if configured {
            "configured"
        } else {
            "not configured"
        }
    );
    Ok(())
}

pub fn token_clear(state: &State) -> Result<()> {
    state.initialize()?;
    Database::open(state.db_path())?.clear_sync_token()?;
    println!("Dispatch Cloud ingestion token cleared.");
    Ok(())
}

pub fn preview(state: &State, run_id: &str) -> Result<()> {
    println!("{}", preview_payload(state, run_id)?);
    Ok(())
}

pub fn flush(state: &State) -> Result<()> {
    state.initialize()?;
    let endpoint = configured_cloud_url()?;
    let report = flush_to(state, &endpoint)?;
    println!(
        "Evaluation sync: {} synced, {} failed.",
        report.synced, report.failed
    );
    if report.failed > 0 {
        bail!(
            "{} evaluation upload(s) failed and remain in the outbox",
            report.failed
        );
    }
    Ok(())
}

pub fn preview_payload(state: &State, run_id: &str) -> Result<String> {
    state.initialize()?;
    let database = Database::open(state.db_path())?;
    let settings = require_enabled(&database)?;
    let run = state.load_run(run_id)?;
    queue_run(&database, &settings, &run)?;
    database
        .sync_payload_for_run(&run.id)?
        .context("evaluation was not queued for sync")
}

fn queue_all(state: &State, database: &Database, settings: &SyncSettings) -> Result<usize> {
    let mut queued = 0;
    for path in state.list_metadata_paths()? {
        let run: RunRecord = serde_json::from_slice(
            &fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?,
        )
        .with_context(|| format!("invalid run metadata {}", path.display()))?;
        if run.evaluation.is_some() && queue_run(database, settings, &run)? {
            queued += 1;
        }
    }
    Ok(queued)
}

fn queue_run(database: &Database, settings: &SyncSettings, run: &RunRecord) -> Result<bool> {
    anyhow::ensure!(settings.enabled, "evaluation sync is disabled");
    anyhow::ensure!(run.evaluation.is_some(), "run {} has no evaluation", run.id);
    let evaluation_id = database
        .evaluation_id(&run.id)?
        .with_context(|| format!("run {} has no persisted evaluation", run.id))?;
    let envelope = envelope_for(run, evaluation_id, settings)?;
    let payload = serialize_envelope(&envelope)?;
    database.enqueue_sync(&envelope.evaluation_id, &run.id, &payload)
}

fn require_enabled(database: &Database) -> Result<SyncSettings> {
    let settings = database.sync_settings()?;
    anyhow::ensure!(
        settings.enabled,
        "evaluation sync is disabled; run `dispatch sync enable` first"
    );
    Ok(settings)
}

pub fn envelope_for(
    run: &RunRecord,
    evaluation_id: String,
    settings: &SyncSettings,
) -> Result<EvaluationEnvelopeV1> {
    let evaluation = run
        .evaluation
        .as_ref()
        .with_context(|| format!("run {} has no evaluation", run.id))?;
    let contributor_id = settings
        .contributor_id
        .clone()
        .context("sync contributor ID is missing")?;
    let enabled_at = settings
        .enabled_at
        .clone()
        .context("sync consent timestamp is missing")?;
    let (outcome, selected_candidate) = match &evaluation.outcome {
        EvaluationOutcome::Candidate(label) => ("candidate", Some(label.clone())),
        EvaluationOutcome::Tie => ("tie", None),
        EvaluationOutcome::Neither => ("neither", None),
    };

    Ok(EvaluationEnvelopeV1 {
        schema_version: EVALUATION_SCHEMA_VERSION,
        evaluation_id,
        contributor_id,
        consent: ConsentMetadataV1 {
            scope: "evaluation-v1".into(),
            enabled_at,
        },
        run: SyncRunV1 {
            run_id: run.id.clone(),
            status: run.status.as_str().into(),
            created_at: run.created_at,
            completed_at: run.completed_at,
            dispatch_version: run.environment.dispatch_version.clone(),
            os: run.environment.os.clone(),
            architecture: run.environment.architecture.clone(),
            execution_backend: run.environment.execution_backend.clone(),
            timeout_secs: run.environment.timeout_secs,
            cpus: run.environment.cpus,
            memory: run.environment.memory.clone(),
            resource_limits_enforced: run.environment.resource_limits_enforced,
            unsafe_local: run.environment.unsafe_local,
        },
        task: SyncTaskV1 {
            text: run.task.clone(),
            source: None,
        },
        project: SyncProjectV1 {
            source_type: run.source_kind.as_str().into(),
            languages: None,
            file_count: None,
        },
        baseline_verification: verification(&run.baseline_checks, false),
        candidates: run
            .candidates
            .iter()
            .map(|candidate| candidate_envelope(candidate, &evaluation.outcome))
            .collect(),
        human_evaluation: HumanEvaluationV1 {
            outcome: outcome.into(),
            selected_candidate,
            reasons: evaluation.reasons.clone(),
            explanation: evaluation.explanation.clone(),
            blind: evaluation.blind,
            created_at: evaluation.created_at,
        },
    })
}

fn candidate_envelope(candidate: &CandidateRecord, outcome: &EvaluationOutcome) -> SyncCandidateV1 {
    let human_outcome = match outcome {
        EvaluationOutcome::Candidate(label) if label == &candidate.label => "selected",
        EvaluationOutcome::Candidate(_) => "not_selected",
        EvaluationOutcome::Tie => "tie",
        EvaluationOutcome::Neither => "neither",
    };
    let process_failure = matches!(
        candidate.status,
        CandidateStatus::Failed | CandidateStatus::MissingHarness
    ) || candidate.exit_code.is_some_and(|code| code != 0)
        && !candidate.timed_out;
    SyncCandidateV1 {
        candidate_id: candidate.id.clone(),
        label: candidate.label.clone(),
        harness_id: candidate.harness_id.clone(),
        harness_version: candidate.harness_version.clone(),
        model: candidate.model.clone(),
        status: candidate.status.as_str().into(),
        runtime_ms: candidate.duration_ms,
        exit_code: candidate.exit_code,
        timed_out: candidate.timed_out,
        process_failure,
        task_tokens: candidate.tokens,
        token_semantics: candidate.token_semantics.clone(),
        cost_usd: candidate.cost_usd,
        files_changed: candidate.diff_stats.files_changed,
        lines_added: candidate.diff_stats.lines_added,
        lines_removed: candidate.diff_stats.lines_removed,
        verification: verification(
            &candidate.checks,
            candidate.status != CandidateStatus::Completed,
        ),
        human_outcome: human_outcome.into(),
    }
}

fn verification(checks: &[CheckResult], not_run: bool) -> VerificationSummaryV1 {
    let state = if checks.is_empty() {
        if not_run { "not_run" } else { "not_configured" }
    } else if checks
        .iter()
        .all(|check| check.status == CheckStatus::Passed)
    {
        "passed"
    } else {
        "failed"
    };
    VerificationSummaryV1 {
        state: state.into(),
        checks: checks
            .iter()
            .map(|check| VerificationResultV1 {
                name: check.name.clone(),
                phase: match check.phase {
                    CheckPhase::Baseline => "baseline",
                    CheckPhase::Verify => "verify",
                }
                .into(),
                status: match check.status {
                    CheckStatus::Passed => "passed",
                    CheckStatus::Failed => "failed",
                    CheckStatus::TimedOut => "timed_out",
                    CheckStatus::NotRun => "not_run",
                }
                .into(),
                exit_code: check.exit_code,
                duration_ms: check.duration_ms,
            })
            .collect(),
    }
}

pub fn serialize_envelope(envelope: &EvaluationEnvelopeV1) -> Result<String> {
    serde_json::to_string_pretty(envelope).context("failed to serialize evaluation envelope")
}

fn configured_cloud_url() -> Result<String> {
    let value = env::var("DISPATCH_CLOUD_URL").unwrap_or_else(|_| DEFAULT_CLOUD_URL.into());
    validate_cloud_url(&value)
}

fn validate_cloud_url(value: &str) -> Result<String> {
    let value = value.trim_end_matches('/');
    let local_http = value.starts_with("http://127.0.0.1:")
        || value.starts_with("http://localhost:")
        || value.starts_with("http://[::1]:");
    anyhow::ensure!(
        value.starts_with("https://") || local_http,
        "Dispatch Cloud URL must use HTTPS (plain HTTP is allowed only for loopback testing)"
    );
    anyhow::ensure!(
        !value.contains('@'),
        "Dispatch Cloud URL must not contain credentials"
    );
    anyhow::ensure!(
        !value.chars().any(char::is_whitespace),
        "Dispatch Cloud URL must not contain whitespace"
    );
    Ok(value.into())
}

fn validate_ingestion_token(token: &str) -> Result<()> {
    anyhow::ensure!(
        !token.is_empty()
            && token.len() <= MAX_TOKEN_LENGTH
            && token.bytes().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'+' | b'/' | b'=')
            }),
        "invalid Dispatch Cloud ingestion token"
    );
    Ok(())
}

pub(crate) fn flush_to(state: &State, base_url: &str) -> Result<SyncReport> {
    state.initialize()?;
    let database = Database::open(state.db_path())?;
    let settings = require_enabled(&database)?;
    queue_all(state, &database, &settings)?;
    let token = database.sync_token()?.context(
        "Dispatch Cloud ingestion token not configured; run `dispatch sync token set <token>`",
    )?;
    validate_ingestion_token(&token)?;
    let endpoint = format!("{}{}", validate_cloud_url(base_url)?, EVALUATION_PATH);
    let mut report = SyncReport::default();
    for record in database.pending_sync_records()? {
        let attempted_at = Utc::now();
        match post_evaluation(
            &endpoint,
            &record.evaluation_id,
            &record.payload_json,
            &token,
        ) {
            Ok(()) => {
                database.mark_sync_succeeded(&record.evaluation_id, attempted_at)?;
                report.synced += 1;
            }
            Err(error) => {
                let message = format!("{error:#}");
                database.mark_sync_failed(
                    &record.evaluation_id,
                    attempted_at,
                    &message.chars().take(500).collect::<String>(),
                )?;
                report.failed += 1;
            }
        }
    }
    Ok(report)
}

fn post_evaluation(endpoint: &str, evaluation_id: &str, payload: &str, token: &str) -> Result<()> {
    let curl = trusted_host_executable("curl")?;
    let response_file =
        tempfile::NamedTempFile::new().context("failed to create HTTP response file")?;
    let mut authorization_file =
        tempfile::NamedTempFile::new().context("failed to create HTTP authorization file")?;
    writeln!(authorization_file, "Authorization: Bearer {token}")
        .context("failed to write HTTP authorization")?;
    let authorization_path = authorization_file
        .path()
        .to_str()
        .context("HTTP authorization path is not UTF-8")?;
    let mut command = Command::new(curl);
    command
        .env_clear()
        .env("PATH", trusted_host_path())
        .args([
            "--silent",
            "--show-error",
            "--connect-timeout",
            "5",
            "--max-time",
            "10",
            "--max-filesize",
            "1048576",
            "--request",
            "POST",
            "--header",
            "Content-Type: application/json",
            "--header",
            &format!("@{authorization_path}"),
            "--header",
            &format!("Idempotency-Key: {evaluation_id}"),
            "--header",
            &format!("User-Agent: dispatch/{VERSION}"),
            "--data-binary",
            "@-",
            "--output",
            response_file
                .path()
                .to_str()
                .context("HTTP response path is not UTF-8")?,
            "--write-out",
            "%{http_code}",
            endpoint,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().context("failed to start curl")?;
    let mut stdin = child.stdin.take().context("failed to open curl stdin")?;
    let body = payload.as_bytes().to_vec();
    let writer = thread::spawn(move || stdin.write_all(&body));
    let output = child
        .wait_with_output()
        .context("failed to wait for curl")?;
    writer
        .join()
        .map_err(|_| anyhow::anyhow!("curl request writer panicked"))?
        .context("failed to write evaluation payload")?;
    let status_code = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u16>()
        .unwrap_or(0);
    if output.status.success() && (200..300).contains(&status_code) {
        return Ok(());
    }
    if matches!(status_code, 401 | 403) {
        bail!("Dispatch Cloud ingestion token was rejected (HTTP {status_code})");
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    bail!(
        "evaluation upload failed (HTTP {status_code}): {}",
        stderr.trim()
    )
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        path::PathBuf,
        sync::mpsc,
    };

    use chrono::TimeZone;

    use super::*;
    use crate::{
        CandidateStatus, CheckPhase, CheckStatus, DiffStats, EnvironmentRecord, EvaluationRecord,
        RunStatus, SourceKind,
    };

    fn at(second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 22, 12, 0, second).unwrap()
    }

    fn check(phase: CheckPhase, status: CheckStatus) -> CheckResult {
        CheckResult {
            name: match phase {
                CheckPhase::Baseline => "baseline 1",
                CheckPhase::Verify => "verify 1",
            }
            .into(),
            phase,
            command: "/Users/alice/private/repo/test --token secret".into(),
            status,
            exit_code: Some(0),
            duration_ms: 25,
            stdout_path: PathBuf::from("/Users/alice/private/stdout"),
            stderr_path: PathBuf::from("/Users/alice/private/stderr"),
        }
    }

    fn candidate(id: &str, label: &str, harness: &str, tokens: Option<u64>) -> CandidateRecord {
        CandidateRecord {
            id: id.into(),
            label: label.into(),
            harness_id: harness.into(),
            harness_version: (label == "A").then(|| "1.2.3".into()),
            model: None,
            status: CandidateStatus::Completed,
            workspace_path: PathBuf::from("/Users/alice/private/workspace"),
            prompt_path: PathBuf::from("/Users/alice/private/prompt"),
            stdout_path: PathBuf::from("/Users/alice/private/stdout"),
            stderr_path: PathBuf::from("/Users/alice/private/stderr"),
            diff_path: PathBuf::from("/Users/alice/private/diff.patch"),
            duration_ms: 1_250,
            exit_code: Some(0),
            timed_out: false,
            tokens,
            token_semantics: tokens.map(|_| "uncached-input+output+cache-creation".into()),
            cost_usd: None,
            error: Some("credential AWS_SECRET_ACCESS_KEY=secret".into()),
            diff_stats: DiffStats {
                files_changed: 2,
                lines_added: 12,
                lines_removed: 3,
                changed_files: vec!["src/private.rs".into()],
                untracked_files: vec![],
            },
            checks: vec![check(CheckPhase::Verify, CheckStatus::Passed)],
        }
    }

    fn evaluated_run(id: &str) -> RunRecord {
        RunRecord {
            id: id.into(),
            task: "Fix the retry race.".into(),
            exact_prompt: "private system prompt".into(),
            source_path: PathBuf::from("/Users/alice/private/repo"),
            source_kind: SourceKind::Directory,
            source_git_head: Some("private-head".into()),
            source_fingerprint: "private-fingerprint".into(),
            baseline_path: PathBuf::from("/Users/alice/private/baseline"),
            baseline_commit: "private-commit".into(),
            status: RunStatus::Evaluated,
            created_at: at(1),
            completed_at: Some(at(4)),
            environment: EnvironmentRecord {
                dispatch_version: "0.1.0".into(),
                os: "macos".into(),
                architecture: "aarch64".into(),
                execution_backend: "local".into(),
                timeout_secs: 300,
                cpus: 2.0,
                memory: "4g".into(),
                max_parallel: 2,
                docker_image: None,
                resource_limits_enforced: false,
                unsafe_local: true,
                forwarded_env: vec!["AWS_SECRET_ACCESS_KEY".into()],
            },
            baseline_checks: vec![check(CheckPhase::Baseline, CheckStatus::Failed)],
            candidates: vec![
                candidate("candidate-a", "A", "cursor", Some(20_892)),
                candidate("candidate-b", "B", "codex", None),
            ],
            evaluation: Some(EvaluationRecord {
                outcome: EvaluationOutcome::Candidate("B".into()),
                reasons: vec!["correctness".into(), "cleaner-change".into()],
                explanation: Some("  Keep this exactly.\nSecond line.  \n".into()),
                created_at: at(5),
                blind: true,
            }),
            applied_candidate: None,
        }
    }

    fn persisted_run() -> Result<(tempfile::TempDir, State, RunRecord)> {
        let temp = tempfile::tempdir()?;
        let state = State {
            root: temp.path().join("state"),
        };
        state.initialize()?;
        let run = evaluated_run("01TESTSYNC0000000000000000");
        state.save_run(&run)?;
        let mut database = Database::open(state.db_path())?;
        database.sync_run(&run)?;
        Ok((temp, state, run))
    }

    fn enable_for_test(state: &State) -> Result<SyncSettings> {
        let mut database = Database::open(state.db_path())?;
        database.enable_sync("01CONTRIBUTOR0000000000000", at(6))
    }

    fn set_token_for_test(state: &State, token: &str) -> Result<()> {
        Database::open(state.db_path())?.set_sync_token(token)
    }

    #[test]
    fn envelope_preserves_semantics_unknowns_and_privacy_boundary() -> Result<()> {
        let run = evaluated_run("run-envelope");
        let settings = SyncSettings {
            enabled: true,
            contributor_id: Some("contributor".into()),
            enabled_at: Some(at(6).to_rfc3339()),
        };
        let envelope = envelope_for(&run, "evaluation-run-envelope".into(), &settings)?;
        let payload = serialize_envelope(&envelope)?;
        let value: serde_json::Value = serde_json::from_str(&payload)?;

        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["candidates"][0]["task_tokens"], 20_892);
        assert_eq!(
            value["candidates"][0]["token_semantics"],
            "uncached-input+output+cache-creation"
        );
        assert!(value["candidates"][1]["task_tokens"].is_null());
        assert!(value["candidates"][1]["model"].is_null());
        assert!(value["candidates"][1]["cost_usd"].is_null());
        assert_eq!(
            value["human_evaluation"]["explanation"],
            "  Keep this exactly.\nSecond line.  \n"
        );
        for excluded in [
            "/Users/alice",
            "AWS_SECRET_ACCESS_KEY",
            "private-head",
            "private-fingerprint",
            "diff.patch",
            "workspace_path",
            "forwarded_env",
            "exact_prompt",
        ] {
            assert!(!payload.contains(excluded), "payload leaked {excluded}");
        }

        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../schemas/evaluation-v1.json"))?;
        assert_eq!(schema["properties"]["schema_version"]["const"], 1);
        Ok(())
    }

    #[test]
    fn envelope_distinguishes_verification_not_run_from_not_configured() -> Result<()> {
        let mut run = evaluated_run("run-verification-state");
        run.candidates[0].status = CandidateStatus::TimedOut;
        run.candidates[0].timed_out = true;
        run.candidates[0].checks.clear();
        run.candidates[1].checks.clear();
        let settings = SyncSettings {
            enabled: true,
            contributor_id: Some("contributor".into()),
            enabled_at: Some(at(6).to_rfc3339()),
        };
        let envelope = envelope_for(&run, "evaluation-run-verification-state".into(), &settings)?;
        assert_eq!(envelope.candidates[0].verification.state, "not_run");
        assert_eq!(envelope.candidates[1].verification.state, "not_configured");
        Ok(())
    }

    #[test]
    fn historical_evaluation_queues_only_after_opt_in_with_stable_id() -> Result<()> {
        let (_temp, state, run) = persisted_run()?;
        let database = Database::open(state.db_path())?;
        assert!(!database.sync_settings()?.enabled);
        assert!(database.pending_sync_records()?.is_empty());
        drop(database);

        let first = enable_for_test(&state)?;
        let database = Database::open(state.db_path())?;
        assert_eq!(queue_all(&state, &database, &first)?, 1);
        let queued = database.pending_sync_records()?;
        assert_eq!(queued[0].evaluation_id, format!("evaluation-{}", run.id));
        assert_eq!(queue_all(&state, &database, &first)?, 0);
        drop(database);

        let mut database = Database::open(state.db_path())?;
        database.disable_sync()?;
        let second = database.enable_sync("replacement", at(7))?;
        assert_eq!(first.contributor_id, second.contributor_id);
        assert_eq!(first.enabled_at, second.enabled_at);
        Ok(())
    }

    #[test]
    fn token_storage_is_separate_from_consent_and_preview() -> Result<()> {
        let (_temp, state, run) = persisted_run()?;
        let token = "dispatch-preview-secret";
        set_token_for_test(&state, token)?;
        let database = Database::open(state.db_path())?;
        assert!(!database.sync_settings()?.enabled);
        assert_eq!(database.sync_token()?.as_deref(), Some(token));
        drop(database);

        enable_for_test(&state)?;
        let preview = preview_payload(&state, &run.id)?;
        assert!(!preview.contains(token));
        let database = Database::open(state.db_path())?;
        database.disable_sync()?;
        assert_eq!(database.sync_token()?.as_deref(), Some(token));
        database.clear_sync_token()?;
        assert!(database.sync_token()?.is_none());
        Ok(())
    }

    #[test]
    fn enabled_sync_without_token_fails_safely_and_stays_pending() -> Result<()> {
        let (_temp, state, run) = persisted_run()?;
        enable_for_test(&state)?;
        let error = flush_to(&state, "http://127.0.0.1:9").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Dispatch Cloud ingestion token not configured")
        );
        let database = Database::open(state.db_path())?;
        assert_eq!(database.sync_outbox_counts()?.pending, 1);
        assert_eq!(database.sync_outbox_counts()?.failed, 0);
        assert_eq!(
            state.load_run(&run.id)?.evaluation.unwrap().explanation,
            run.evaluation.unwrap().explanation,
            "missing auth must not alter the local evaluation"
        );
        Ok(())
    }

    #[test]
    fn disabled_sync_with_token_does_not_queue_or_upload() -> Result<()> {
        let (_temp, state, _run) = persisted_run()?;
        set_token_for_test(&state, "configured-but-no-consent")?;
        let error = flush_to(&state, "https://not-contacted.invalid").unwrap_err();
        assert!(error.to_string().contains("evaluation sync is disabled"));
        let database = Database::open(state.db_path())?;
        assert!(database.pending_sync_records()?.is_empty());
        assert_eq!(
            database.sync_token()?.as_deref(),
            Some("configured-but-no-consent")
        );
        Ok(())
    }

    #[test]
    fn preview_is_the_transmitted_payload_and_success_marks_synced() -> Result<()> {
        let (_temp, state, run) = persisted_run()?;
        enable_for_test(&state)?;
        let token = "accepted-preview-token";
        set_token_for_test(&state, token)?;
        let preview = preview_payload(&state, &run.id)?;
        let (url, received, server) = mock_server(201)?;
        let report = flush_to(&state, &url)?;
        assert_eq!(
            report,
            SyncReport {
                synced: 1,
                failed: 0
            }
        );
        let request = received.recv()?;
        server.join().unwrap()?;
        let (headers, body) = split_request(&request)?;
        assert!(headers.contains(&format!("Authorization: Bearer {token}")));
        assert!(headers.contains(&format!("Idempotency-Key: evaluation-{}", run.id)));
        assert!(
            !body
                .windows(token.len())
                .any(|window| window == token.as_bytes())
        );
        assert_eq!(body, preview.as_bytes());
        assert_eq!(
            Database::open(state.db_path())?
                .sync_outbox_counts()?
                .synced,
            1
        );
        Ok(())
    }

    #[test]
    fn failed_upload_retries_without_changing_local_evaluation() -> Result<()> {
        let (_temp, state, run) = persisted_run()?;
        enable_for_test(&state)?;
        set_token_for_test(&state, "retry-token")?;
        let preview = preview_payload(&state, &run.id)?;
        let (failed_url, failed_request, failed_server) = mock_server(500)?;
        assert_eq!(
            flush_to(&state, &failed_url)?,
            SyncReport {
                synced: 0,
                failed: 1
            }
        );
        failed_request.recv()?;
        failed_server.join().unwrap()?;
        assert_eq!(
            state.load_run(&run.id)?.evaluation.unwrap().explanation,
            run.evaluation.unwrap().explanation
        );

        let (success_url, received, success_server) = mock_server(200)?;
        assert_eq!(
            flush_to(&state, &success_url)?,
            SyncReport {
                synced: 1,
                failed: 0
            }
        );
        let request = received.recv()?;
        success_server.join().unwrap()?;
        assert_eq!(split_request(&request)?.1, preview.as_bytes());
        Ok(())
    }

    #[test]
    fn rejected_token_leaves_outbox_retryable_without_leaking_secret() -> Result<()> {
        let (_temp, state, run) = persisted_run()?;
        let token = "revoked-token-secret";
        enable_for_test(&state)?;
        set_token_for_test(&state, token)?;
        let (url, received, server) = mock_server(401)?;
        assert_eq!(
            flush_to(&state, &url)?,
            SyncReport {
                synced: 0,
                failed: 1
            }
        );
        received.recv()?;
        server.join().unwrap()?;
        let connection = rusqlite::Connection::open(state.db_path())?;
        let (status, error): (String, String) = connection.query_row(
            "SELECT status, last_error FROM sync_outbox WHERE evaluation_id = ?1",
            [format!("evaluation-{}", run.id)],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(status, "failed");
        assert_eq!(
            Database::open(state.db_path())?
                .pending_sync_records()?
                .len(),
            1
        );
        assert!(error.contains("token was rejected (HTTP 401)"));
        assert!(!error.contains(token));
        assert_eq!(
            state.load_run(&run.id)?.evaluation.unwrap().explanation,
            run.evaluation.unwrap().explanation
        );
        Ok(())
    }

    type MockServer = (
        String,
        mpsc::Receiver<Vec<u8>>,
        thread::JoinHandle<Result<()>>,
    );

    fn mock_server(status: u16) -> Result<MockServer> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let (sender, receiver) = mpsc::channel();
        let handle = thread::spawn(move || -> Result<()> {
            let (mut stream, _) = listener.accept()?;
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            let mut expected = None;
            loop {
                let read = stream.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if expected.is_none()
                    && let Some(split) = request.windows(4).position(|part| part == b"\r\n\r\n")
                {
                    let headers = String::from_utf8_lossy(&request[..split]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .and_then(|value| value.parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    expected = Some(split + 4 + length);
                }
                if expected.is_some_and(|length| request.len() >= length) {
                    break;
                }
            }
            sender.send(request)?;
            let reason = match status {
                200 => "OK",
                201 => "Created",
                401 => "Unauthorized",
                _ => "Server Error",
            };
            write!(
                stream,
                "HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )?;
            Ok(())
        });
        Ok((format!("http://{address}"), receiver, handle))
    }

    fn split_request(request: &[u8]) -> Result<(&str, &[u8])> {
        let split = request
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
            .context("mock request had no header terminator")?;
        Ok((
            std::str::from_utf8(&request[..split])?,
            &request[split + 4..],
        ))
    }
}
