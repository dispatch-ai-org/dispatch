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
    RoutingObservation, RunRecord, VERSION,
    db::{Database, SyncRecordType, SyncSettings},
    executor::{trusted_host_executable, trusted_host_path},
    orchestrator::OperationLock,
    state::State,
};

pub const EVALUATION_SCHEMA_VERSION: u32 = 1;
pub const ROUTING_OBSERVATION_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_CLOUD_URL: &str = "https://api.rundispatch.sh";
const EVALUATION_PATH: &str = "/v1/evaluations";
const ROUTING_OBSERVATION_PATH: &str = "/v1/routing-observations";
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutingObservationV1 {
    pub schema_version: u32,
    pub observation_id: String,
    pub run_id: String,
    pub contributor_id: String,
    pub consent: ConsentMetadataV1,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub prediction: RoutingPredictionV1,
    pub actual_harness: RoutingActualHarnessV1,
    pub mechanical_outcome: RoutingMechanicalOutcomeV1,
    pub human_evaluation: Option<RoutingHumanEvaluationV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutingPredictionV1 {
    pub version: u32,
    pub task_features: RoutingTaskFeaturesV1,
    pub selected_harness: String,
    pub successes: u64,
    pub attempts: u64,
    pub specificity: u8,
    pub source: String,
    pub dataset: String,
    pub dataset_version: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutingTaskFeaturesV1 {
    pub language: Option<String>,
    pub task_kind: String,
    pub scope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutingActualHarnessV1 {
    pub harness: String,
    pub harness_version: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutingMechanicalOutcomeV1 {
    pub candidate_status: String,
    pub exit_code: Option<i32>,
    pub timed_out: Option<bool>,
    pub verification: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutingHumanEvaluationV1 {
    pub outcome: String,
    pub reasons: Vec<String>,
    pub explanation: Option<String>,
    pub evaluated_at: DateTime<Utc>,
}

/// Also stored as the immutable local event: no prediction or execution data is copied.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutingFeedbackV1 {
    pub schema_version: u32,
    pub feedback_event_id: String,
    pub observation_id: String,
    pub contributor_id: String,
    pub revision: u32,
    pub consent: ConsentMetadataV1,
    pub outcome: String,
    pub reasons: Vec<String>,
    pub explanation: Option<String>,
    pub evaluated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub synced: usize,
    pub failed: usize,
}

pub fn enable(state: &State) -> Result<()> {
    state.initialize()?;
    let _sync_lock = lock_sync(state)?;
    let mut database = Database::open(state.db_path())?;
    let settings = database.enable_sync(&Ulid::new().to_string(), Utc::now())?;
    let queued = queue_all(state, &database, &settings)?;
    let evaluations = database.evaluation_count()?;
    let routing_observations = database.routing_observations()?.len();

    println!("Dispatch contribution consent enabled locally. No data was uploaded.");
    println!(
        "The current scope includes reviewed evaluation data and routed execution observations: task morphology, routing evidence, actual harness/model identity, mechanical outcomes, and explicit routed accept/reject feedback."
    );
    println!(
        "Task text is shared only in evaluation-v1. Human explanations are uploaded verbatim when present and may contain proprietary information."
    );
    println!(
        "Source code, full diffs, logs, local paths, environment values, and credentials are not uploaded."
    );
    println!(
        "{evaluations} completed evaluation(s) and {routing_observations} routing observation(s) are available; {queued} newly queued or refreshed."
    );
    println!("Inspect the exact body with: dispatch sync preview <run-id>");
    println!("Transmit pending records only when ready with: dispatch sync");
    Ok(())
}

pub fn disable(state: &State) -> Result<()> {
    state.initialize()?;
    Database::open(state.db_path())?.disable_sync()?;
    println!("Dispatch contribution sync disabled. No uploads will be attempted.");
    Ok(())
}

pub fn status(state: &State) -> Result<()> {
    state.initialize()?;
    let database = Database::open(state.db_path())?;
    let settings = database.sync_settings()?;
    println!(
        "Evaluation sync: {}",
        if settings.enabled {
            "enabled"
        } else {
            "disabled"
        }
    );
    println!("Consent version: {}", settings.consent_version);
    println!("Scopes:");
    println!(
        "  evaluation-v1 ({})",
        if settings.enabled {
            "enabled"
        } else {
            "not enabled"
        }
    );
    println!(
        "  routing-observation-v1 ({})",
        if settings.routing_observations_enabled() {
            "enabled"
        } else {
            "not enabled"
        }
    );
    if let Some(contributor_id) = settings.contributor_id {
        println!("Contributor ID: {contributor_id}");
    }
    println!("Completed evaluations: {}", database.evaluation_count()?);
    println!(
        "Routing observations: {}",
        database.routing_observations()?.len()
    );
    for record_type in [
        SyncRecordType::EvaluationV1,
        SyncRecordType::RoutingObservationV1,
        SyncRecordType::RoutingFeedbackV1,
    ] {
        let counts = database.sync_outbox_counts_for_type(record_type)?;
        println!(
            "Outbox {}: {} pending, {} failed, {} conflict, {} synced",
            record_type.as_str(),
            counts.pending,
            counts.failed,
            counts.conflict,
            counts.synced
        );
    }
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

pub fn preview(
    state: &State,
    run_id: &str,
    requested_type: Option<&str>,
    revision: Option<u32>,
) -> Result<()> {
    let requested_type = requested_type.map(parse_record_type).transpose()?;
    let (record_type, payload) =
        preview_payload_for_revision(state, run_id, requested_type, revision)?;
    eprintln!("Record type: {}", record_type.as_str());
    println!("{payload}");
    Ok(())
}

pub fn flush(state: &State) -> Result<()> {
    state.initialize()?;
    let endpoint = configured_cloud_url()?;
    let report = flush_to(state, &endpoint)?;
    println!("Sync: {} synced, {} failed.", report.synced, report.failed);
    if report.failed > 0 {
        bail!(
            "{} upload(s) failed; inspect `dispatch sync status`",
            report.failed
        );
    }
    Ok(())
}

#[cfg(test)]
fn preview_payload(state: &State, run_id: &str) -> Result<String> {
    preview_payload_for_type(state, run_id, None).map(|(_, payload)| payload)
}

#[cfg(test)]
fn preview_payload_for_type(
    state: &State,
    run_id: &str,
    requested_type: Option<SyncRecordType>,
) -> Result<(SyncRecordType, String)> {
    preview_payload_for_revision(state, run_id, requested_type, None)
}

pub fn preview_payload_for_revision(
    state: &State,
    run_id: &str,
    requested_type: Option<SyncRecordType>,
    revision: Option<u32>,
) -> Result<(SyncRecordType, String)> {
    state.initialize()?;
    let _sync_lock = lock_sync(state)?;
    let database = Database::open(state.db_path())?;
    let settings = require_enabled(&database)?;
    let run = state.load_run(run_id)?;
    let _run_lock = OperationLock::acquire(
        &state.run_dir(&run.id).join(".operation.lock"),
        "another operation is using this run",
    )?;
    if settings.routing_observations_enabled() {
        database.reconcile_routing_feedback(Some(&run.id))?;
    }
    let observation = database.routing_observation_for_run(&run.id)?;
    let feedback = database.routing_feedback_for_run(&run.id)?;
    let mut eligible = Vec::new();
    if run.evaluation.is_some() {
        eligible.push(SyncRecordType::EvaluationV1);
    }
    if settings.routing_observations_enabled() && observation.is_some() {
        eligible.push(SyncRecordType::RoutingObservationV1);
    }
    if settings.routing_observations_enabled() && !feedback.is_empty() {
        eligible.push(SyncRecordType::RoutingFeedbackV1);
    }

    let record_type = if let Some(requested) = requested_type {
        anyhow::ensure!(
            eligible.contains(&requested),
            "run {} has no eligible {} record under the current consent scope",
            run.id,
            requested.as_str()
        );
        requested
    } else {
        match eligible.as_slice() {
            [only] => *only,
            [] if observation.is_some() && !settings.routing_observations_enabled() => bail!(
                "routing-observation sync consent is not enabled; run `dispatch sync enable` to accept the current scope"
            ),
            [] => bail!("run {} has no eligible sync record", run.id),
            _ => bail!(
                "run {} has multiple eligible sync records; use --type evaluation, --type routing-observation, or --type routing-feedback",
                run.id
            ),
        }
    };
    anyhow::ensure!(
        revision.is_none() || record_type == SyncRecordType::RoutingFeedbackV1,
        "--revision requires --type routing-feedback"
    );
    match record_type {
        SyncRecordType::EvaluationV1 => {
            queue_evaluation(&database, &settings, &run)?;
        }
        SyncRecordType::RoutingObservationV1 => {
            queue_routing_observation(
                &database,
                &settings,
                observation
                    .as_ref()
                    .context("routing observation disappeared during preview")?,
            )?;
        }
        SyncRecordType::RoutingFeedbackV1 => {
            let event = match revision {
                Some(revision) => feedback
                    .iter()
                    .find(|event| event.revision == revision)
                    .with_context(|| {
                        format!("run {} has no feedback revision {revision}", run.id)
                    })?,
                None if feedback.len() == 1 => &feedback[0],
                None => bail!("multiple feedback revisions exist; use --revision <n>"),
            };
            return Ok((
                record_type,
                database.sync_payload_for_record(&event.feedback_event_id)?,
            ));
        }
    }
    let payload = database
        .sync_payload_for_run(&run.id, record_type)?
        .with_context(|| format!("{} was not queued for sync", record_type.as_str()))?;
    Ok((record_type, payload))
}

fn queue_all(state: &State, database: &Database, settings: &SyncSettings) -> Result<usize> {
    let mut queued = 0;
    if settings.routing_observations_enabled() {
        queued += database.reconcile_routing_feedback(None)?;
    }
    for path in state.list_metadata_paths()? {
        let run: RunRecord = serde_json::from_slice(
            &fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?,
        )
        .with_context(|| format!("invalid run metadata {}", path.display()))?;
        if run.evaluation.is_some() && queue_evaluation(database, settings, &run)? {
            queued += 1;
        }
    }
    if settings.routing_observations_enabled() {
        for observation in database.routing_observations()? {
            if queue_routing_observation(database, settings, &observation)? {
                queued += 1;
            }
        }
    }
    Ok(queued)
}

fn queue_evaluation(database: &Database, settings: &SyncSettings, run: &RunRecord) -> Result<bool> {
    anyhow::ensure!(settings.enabled, "evaluation sync is disabled");
    // Attached work is observed, not selected or routed, by design (part 6.3,
    // 6.6, 14.6 of the attach plan): it is reviewed through `review.*`
    // events, never through `EvaluationRecord`, and never becomes routing or
    // evaluation evidence. This is a backstop, not the only guard: nothing in
    // this packet can set `evaluation` on an attached run in the first place.
    anyhow::ensure!(
        run.mode != crate::RunMode::Attached,
        "attached work (run {}) is never queued for evaluation upload",
        run.id
    );
    anyhow::ensure!(run.evaluation.is_some(), "run {} has no evaluation", run.id);
    let evaluation_id = database
        .evaluation_id(&run.id)?
        .with_context(|| format!("run {} has no persisted evaluation", run.id))?;
    let envelope = envelope_for(run, evaluation_id, settings)?;
    let payload = serialize_envelope(&envelope)?;
    database.enqueue_sync(
        SyncRecordType::EvaluationV1,
        &envelope.evaluation_id,
        &run.id,
        &payload,
    )
}

fn queue_routing_observation(
    database: &Database,
    settings: &SyncSettings,
    observation: &RoutingObservation,
) -> Result<bool> {
    if observation.prediction.selection_basis != crate::SelectionBasis::Evidence {
        return Ok(false);
    }
    if database
        .synced_routing_payload(&observation.run_id)?
        .is_some()
    {
        return Ok(false);
    }
    let envelope = routing_observation_envelope_for(observation, settings)?;
    let payload = serialize_routing_observation(&envelope)?;
    database.enqueue_sync(
        SyncRecordType::RoutingObservationV1,
        &envelope.observation_id,
        &envelope.run_id,
        &payload,
    )
}

fn require_enabled(database: &Database) -> Result<SyncSettings> {
    let settings = database.sync_settings()?;
    anyhow::ensure!(
        settings.enabled,
        "evaluation sync is disabled; run `dispatch sync enable` first (this also gates routing-observation sync)"
    );
    Ok(settings)
}

fn parse_record_type(value: &str) -> Result<SyncRecordType> {
    match value {
        "evaluation" => Ok(SyncRecordType::EvaluationV1),
        "routing-observation" => Ok(SyncRecordType::RoutingObservationV1),
        "routing-feedback" => Ok(SyncRecordType::RoutingFeedbackV1),
        _ => {
            bail!("sync preview type must be evaluation, routing-observation, or routing-feedback")
        }
    }
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

pub fn routing_observation_envelope_for(
    observation: &RoutingObservation,
    settings: &SyncSettings,
) -> Result<RoutingObservationV1> {
    anyhow::ensure!(
        observation.prediction.selection_basis == crate::SelectionBasis::Evidence,
        "only evidence-based routing observations are eligible for Cloud contribution"
    );
    anyhow::ensure!(
        settings.routing_observations_enabled(),
        "routing-observation sync consent is not enabled; run `dispatch sync enable` to accept the current scope"
    );
    let contributor_id = settings
        .contributor_id
        .clone()
        .context("sync contributor ID is missing")?;
    let enabled_at = settings
        .routing_observation_enabled_at
        .clone()
        .context("routing-observation consent timestamp is missing")?;
    anyhow::ensure!(
        observation.prediction.version == 1,
        "routing prediction version {} cannot be serialized as V1",
        observation.prediction.version
    );
    anyhow::ensure!(
        observation.prediction.successes <= observation.prediction.attempts,
        "routing evidence successes exceed attempts"
    );
    anyhow::ensure!(
        observation.prediction.specificity <= 3,
        "routing evidence specificity exceeds 3"
    );
    anyhow::ensure!(
        observation.candidate_status.is_terminal(),
        "routing observation {} is not terminal",
        observation.id
    );
    if let Some(verification) = &observation.verification {
        anyhow::ensure!(
            !verification.is_empty() && verification.len() <= 128,
            "routing observation verification must contain between 1 and 128 states"
        );
    }

    Ok(RoutingObservationV1 {
        schema_version: ROUTING_OBSERVATION_SCHEMA_VERSION,
        observation_id: observation.id.clone(),
        run_id: observation.run_id.clone(),
        contributor_id,
        consent: ConsentMetadataV1 {
            scope: "routing-observation-v1".into(),
            enabled_at,
        },
        created_at: observation.created_at,
        updated_at: observation.updated_at,
        prediction: RoutingPredictionV1 {
            version: observation.prediction.version,
            task_features: RoutingTaskFeaturesV1 {
                language: observation.prediction.task_features.language.clone(),
                task_kind: observation
                    .prediction
                    .task_features
                    .task_kind
                    .as_str()
                    .into(),
                scope: observation.prediction.task_features.scope.as_str().into(),
            },
            selected_harness: observation.prediction.selected_harness.clone(),
            successes: observation.prediction.successes,
            attempts: observation.prediction.attempts,
            specificity: observation.prediction.specificity,
            source: observation.prediction.source.clone(),
            dataset: observation.prediction.dataset.clone(),
            dataset_version: observation.prediction.dataset_version.clone(),
            model: observation.prediction.model.clone(),
        },
        actual_harness: RoutingActualHarnessV1 {
            // Observation creation verifies the sole candidate matches the selected
            // harness before snapshotting its version and model.
            harness: observation.prediction.selected_harness.clone(),
            harness_version: observation.harness_version.clone(),
            model: observation.model.clone(),
        },
        mechanical_outcome: RoutingMechanicalOutcomeV1 {
            candidate_status: observation.candidate_status.as_str().into(),
            exit_code: observation.exit_code,
            timed_out: Some(observation.timed_out),
            verification: observation.verification.as_ref().map(|states| {
                states
                    .iter()
                    .map(|state| match state {
                        CheckStatus::Passed => "passed",
                        CheckStatus::Failed => "failed",
                        CheckStatus::TimedOut => "timed_out",
                        CheckStatus::NotRun => "not_run",
                    })
                    .map(str::to_owned)
                    .collect()
            }),
        },
        human_evaluation: observation.human_evaluation.as_ref().map(|evaluation| {
            RoutingHumanEvaluationV1 {
                outcome: evaluation.outcome.as_str().into(),
                reasons: evaluation.reasons.clone(),
                explanation: evaluation.explanation.clone(),
                evaluated_at: evaluation.evaluated_at,
            }
        }),
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

pub fn serialize_routing_observation(observation: &RoutingObservationV1) -> Result<String> {
    serde_json::to_string_pretty(observation)
        .context("failed to serialize routing-observation envelope")
}

pub(crate) fn configured_cloud_url() -> Result<String> {
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
    let _sync_lock = lock_sync(state)?;
    let database = Database::open(state.db_path())?;
    let settings = require_enabled(&database)?;
    queue_all(state, &database, &settings)?;
    let token = database.sync_token()?.context(
        "Dispatch Cloud ingestion token not configured; run `dispatch sync token set <token>`",
    )?;
    validate_ingestion_token(&token)?;
    let base_url = validate_cloud_url(base_url)?;
    let mut report = SyncReport::default();
    for mut record in database.pending_sync_records()? {
        if record.record_type != SyncRecordType::EvaluationV1
            && !settings.routing_observations_enabled()
        {
            continue;
        }
        let _run_lock = OperationLock::acquire(
            &state.run_dir(&record.run_id).join(".operation.lock"),
            "another operation is using this run",
        )?;
        if record.record_type == SyncRecordType::RoutingObservationV1 {
            let observation = database
                .routing_observation_for_run(&record.run_id)?
                .context("routing observation is missing")?;
            queue_routing_observation(&database, &settings, &observation)?;
            record.payload_json = database.sync_payload_for_record(&record.record_id)?;
        }
        let attempted_at = Utc::now();
        let endpoint = format!("{}{}", base_url, record_endpoint(record.record_type));
        match post_record(
            &endpoint,
            record.record_type,
            &record.record_id,
            &record.payload_json,
            &token,
        ) {
            Ok(UploadResult::Synced) => {
                database.mark_sync_succeeded(&record.record_id, attempted_at)?;
                report.synced += 1;
            }
            Ok(UploadResult::Conflict(code)) => {
                database.mark_sync_conflict(
                    &record.record_id,
                    attempted_at,
                    &format!(
                        "{} {code} (HTTP 422); record retained locally without automatic retry",
                        record.record_type.as_str()
                    ),
                )?;
                report.failed += 1;
            }
            Err(error) => {
                let message = format!("{error:#}");
                database.mark_sync_failed(
                    &record.record_id,
                    attempted_at,
                    &message.chars().take(500).collect::<String>(),
                )?;
                report.failed += 1;
            }
        }
    }
    Ok(report)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UploadResult {
    Synced,
    Conflict(&'static str),
}

fn lock_sync(state: &State) -> Result<OperationLock> {
    OperationLock::acquire(
        &state.root.join(".sync.lock"),
        "another sync preparation or upload is in progress",
    )
}

#[derive(Deserialize)]
struct CloudErrorResponse {
    error: String,
}

fn record_endpoint(record_type: SyncRecordType) -> &'static str {
    match record_type {
        SyncRecordType::EvaluationV1 => EVALUATION_PATH,
        SyncRecordType::RoutingObservationV1 => ROUTING_OBSERVATION_PATH,
        SyncRecordType::RoutingFeedbackV1 => "/v1/routing-feedback",
    }
}

fn post_record(
    endpoint: &str,
    record_type: SyncRecordType,
    record_id: &str,
    payload: &str,
    token: &str,
) -> Result<UploadResult> {
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
            &format!("Idempotency-Key: {record_id}"),
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
        .context("failed to write sync payload")?;
    let status_code = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u16>()
        .unwrap_or(0);
    if output.status.success() && (200..300).contains(&status_code) {
        return Ok(UploadResult::Synced);
    }
    let response = fs::read_to_string(response_file.path()).unwrap_or_default();
    if status_code == 422
        && let Ok(error) = serde_json::from_str::<CloudErrorResponse>(&response)
    {
        match error.error.as_str() {
            "idempotency_conflict" => {
                return Ok(UploadResult::Conflict("idempotency_conflict"));
            }
            "revision_conflict" if record_type == SyncRecordType::RoutingFeedbackV1 => {
                return Ok(UploadResult::Conflict("revision_conflict"));
            }
            "invalid_parent" if record_type == SyncRecordType::RoutingFeedbackV1 => {
                return Ok(UploadResult::Conflict("invalid_parent"));
            }
            _ => {}
        }
    }
    if matches!(status_code, 401 | 403) {
        bail!(
            "HTTP authentication/authorization failure (HTTP {status_code}); check API credentials and deployment access protection"
        );
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    bail!(
        "{} upload failed (HTTP {status_code}): {}",
        record_type.as_str(),
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
        RoutingDecision, RoutingHumanEvaluation, RoutingHumanOutcome, RoutingObservation,
        RunStatus, SourceKind, TaskFeatures, TaskKind, TaskScope, db::SyncRecordType,
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
            phase3: None,
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
            mode: crate::RunMode::Legacy,
            state_revision: 0,
            outcome: crate::RunOutcome::default(),
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
            attempts: Vec::new(),
            routing: None,
            allocation: None,
            capacity: None,
            admission: None,
            coherence: None,
            evaluation: Some(EvaluationRecord {
                outcome: EvaluationOutcome::Candidate("B".into()),
                reasons: vec!["correctness".into(), "cleaner-change".into()],
                explanation: Some("  Keep this exactly.\nSecond line.  \n".into()),
                created_at: at(5),
                blind: true,
            }),
            applied_candidate: None,
            attachment: None,
        }
    }

    fn routing_observation(
        verification: Option<Vec<CheckStatus>>,
        human_evaluation: Option<RoutingHumanEvaluation>,
    ) -> RoutingObservation {
        RoutingObservation {
            id: "routing-observation-run-routed".into(),
            run_id: "run-routed".into(),
            candidate_id: "candidate-routed".into(),
            created_at: at(1),
            updated_at: at(5),
            prediction: RoutingDecision {
                version: 1,
                task_features: TaskFeatures {
                    language: Some("rust".into()),
                    task_kind: TaskKind::BugFix,
                    scope: TaskScope::MultiFile,
                },
                selected_harness: "cursor".into(),
                successes: 37,
                attempts: 50,
                specificity: 2,
                source: "harbor-framework/harbor".into(),
                dataset: "terminal-bench".into(),
                dataset_version: "2.0".into(),
                model: Some("benchmark/model".into()),
                selection_basis: crate::SelectionBasis::Evidence,
                alternatives: Vec::new(),
            },
            harness_version: Some("cursor-1.2.3".into()),
            model: Some("execution/model".into()),
            candidate_status: CandidateStatus::Completed,
            exit_code: Some(0),
            timed_out: false,
            verification,
            human_evaluation,
        }
    }

    fn current_settings() -> SyncSettings {
        SyncSettings {
            enabled: true,
            contributor_id: Some("01CONTRIBUTOR0000000000000".into()),
            enabled_at: Some(at(6).to_rfc3339()),
            consent_version: 2,
            routing_observation_enabled_at: Some(at(6).to_rfc3339()),
        }
    }

    fn routed_run(id: &str, blind_evaluation: bool) -> RunRecord {
        let mut run = evaluated_run(id);
        run.candidates.truncate(1);
        run.candidates[0].model = Some("execution/model".into());
        run.routing = Some(RoutingDecision {
            version: 1,
            task_features: TaskFeatures {
                language: Some("rust".into()),
                task_kind: TaskKind::BugFix,
                scope: TaskScope::MultiFile,
            },
            selected_harness: "cursor".into(),
            successes: 37,
            attempts: 50,
            specificity: 2,
            source: "harbor-framework/harbor".into(),
            dataset: "terminal-bench".into(),
            dataset_version: "2.0".into(),
            model: Some("benchmark/model".into()),
            selection_basis: crate::SelectionBasis::Evidence,
            alternatives: Vec::new(),
        });
        if blind_evaluation {
            run.evaluation = Some(EvaluationRecord {
                outcome: EvaluationOutcome::Candidate("A".into()),
                reasons: vec!["correctness".into()],
                explanation: None,
                created_at: at(5),
                blind: true,
            });
        } else {
            run.evaluation = None;
            run.status = RunStatus::ReadyForEvaluation;
        }
        run
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

    fn persisted_routed_run(
        blind_evaluation: bool,
    ) -> Result<(tempfile::TempDir, State, RunRecord)> {
        let temp = tempfile::tempdir()?;
        let state = State {
            root: temp.path().join("state"),
        };
        state.initialize()?;
        let run = routed_run("01ROUTINGSYNC0000000000000", blind_evaluation);
        state.save_run(&run)?;
        Database::open(state.db_path())?.sync_run(&run)?;
        Ok((temp, state, run))
    }

    fn enable_legacy_for_test(state: &State) -> Result<SyncSettings> {
        let connection = rusqlite::Connection::open(state.db_path())?;
        connection.execute(
            "UPDATE sync_settings SET enabled = 1, contributor_id = ?1, enabled_at = ?2, \
             consent_version = 1, routing_observation_enabled_at = NULL WHERE id = 1",
            rusqlite::params!["01CONTRIBUTOR0000000000000", at(6).to_rfc3339()],
        )?;
        Database::open(state.db_path())?.sync_settings()
    }

    fn enable_for_test(state: &State) -> Result<SyncSettings> {
        let mut database = Database::open(state.db_path())?;
        database.enable_sync("01CONTRIBUTOR0000000000000", at(6))
    }

    fn set_token_for_test(state: &State, token: &str) -> Result<()> {
        Database::open(state.db_path())?.set_sync_token(token)
    }

    #[test]
    fn production_cloud_endpoint_is_the_default() -> Result<()> {
        assert_eq!(DEFAULT_CLOUD_URL, "https://api.rundispatch.sh");
        assert_eq!(validate_cloud_url(DEFAULT_CLOUD_URL)?, DEFAULT_CLOUD_URL);
        Ok(())
    }

    fn feedback(outcome: RoutingHumanOutcome, second: u32) -> RoutingHumanEvaluation {
        RoutingHumanEvaluation {
            outcome,
            reasons: vec!["correctness".into(), "tests".into()],
            explanation: Some("  Reviewed locally.\nKeep this verbatim.  \n".into()),
            evaluated_at: at(second),
        }
    }

    fn sync_parent_for_test(state: &State, run_id: &str) -> Result<String> {
        enable_for_test(state)?;
        let (_, payload) =
            preview_payload_for_type(state, run_id, Some(SyncRecordType::RoutingObservationV1))?;
        Database::open(state.db_path())?
            .mark_sync_succeeded(&format!("routing-observation-{run_id}"), at(7))?;
        Ok(payload)
    }

    #[test]
    fn feedback_revisions_are_durable_immutable_and_only_follow_synced_parents() -> Result<()> {
        let (_temp, state, run) = persisted_routed_run(false)?;
        enable_for_test(&state)?;
        let mut database = Database::open(state.db_path())?;
        let accepted = feedback(RoutingHumanOutcome::Accepted, 8);
        database.save_routing_human_evaluation(&run.id, &accepted)?;
        assert!(database.routing_feedback_for_run(&run.id)?.is_empty());
        let parent = sync_parent_for_test(&state, &run.id)?;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&parent)?["human_evaluation"]["outcome"],
            "accepted"
        );
        database
            .save_routing_human_evaluation(&run.id, &feedback(RoutingHumanOutcome::Accepted, 9))?;
        assert!(database.routing_feedback_for_run(&run.id)?.is_empty());

        for (index, outcome) in [
            RoutingHumanOutcome::Rejected,
            RoutingHumanOutcome::Accepted,
            RoutingHumanOutcome::Rejected,
        ]
        .into_iter()
        .enumerate()
        {
            let human = feedback(outcome, 10 + index as u32);
            database.save_routing_human_evaluation(&run.id, &human)?;
            database.save_routing_human_evaluation(&run.id, &human)?;
            let events = database.routing_feedback_for_run(&run.id)?;
            assert_eq!(events.len(), index + 1);
            assert_eq!(events[index].revision, index as u32 + 1);
            assert_eq!(
                events[index].feedback_event_id,
                format!("routing-feedback-{}-{}", run.id, index + 1)
            );
            assert_eq!(events[index].reasons, human.reasons);
            assert_eq!(events[index].explanation, human.explanation);
            assert_eq!(events[index].evaluated_at, human.evaluated_at);
        }
        let events = database.routing_feedback_for_run(&run.id)?;
        assert_eq!(
            events
                .iter()
                .map(|e| e.outcome.as_str())
                .collect::<Vec<_>>(),
            ["reject", "accept", "reject"]
        );
        assert_eq!(events[0].evaluated_at, at(10));
        assert_eq!(database.pending_sync_records()?.len(), 3);
        drop(database);
        assert_eq!(
            Database::open(state.db_path())?.routing_feedback_for_run(&run.id)?,
            events
        );
        assert_eq!(
            preview_payload_for_type(&state, &run.id, Some(SyncRecordType::RoutingObservationV1))?
                .1,
            parent
        );
        assert!(
            preview_payload_for_revision(
                &state,
                &run.id,
                Some(SyncRecordType::RoutingFeedbackV1),
                None
            )
            .unwrap_err()
            .to_string()
            .contains("--revision")
        );
        for revision in 1..=3 {
            let (_, payload) = preview_payload_for_revision(
                &state,
                &run.id,
                Some(SyncRecordType::RoutingFeedbackV1),
                Some(revision),
            )?;
            let event: RoutingFeedbackV1 = serde_json::from_str(&payload)?;
            assert_eq!(event, events[revision as usize - 1]);
        }
        assert!(
            preview_payload_for_revision(
                &state,
                &run.id,
                Some(SyncRecordType::RoutingFeedbackV1),
                Some(4)
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn feedback_wire_schema_is_narrow_and_preserves_parent_consent_identity() -> Result<()> {
        let (_temp, state, run) = persisted_routed_run(false)?;
        let parent = sync_parent_for_test(&state, &run.id)?;
        let parent: RoutingObservationV1 = serde_json::from_str(&parent)?;
        let mut database = Database::open(state.db_path())?;
        database
            .save_routing_human_evaluation(&run.id, &feedback(RoutingHumanOutcome::Accepted, 8))?;
        let event = database.routing_feedback_for_run(&run.id)?.remove(0);
        assert_eq!(event.schema_version, 1);
        assert_eq!(event.revision, 1);
        assert_eq!(event.observation_id, parent.observation_id);
        assert_eq!(event.contributor_id, parent.contributor_id);
        assert_eq!(event.consent, parent.consent);
        assert_eq!(event.consent.scope, "routing-observation-v1");
        assert_eq!(event.outcome, "accept");
        let value = serde_json::to_value(&event)?;
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../schemas/routing-feedback-v1.json"))?;
        assert_eq!(schema["properties"]["schema_version"]["const"], 1);
        for key in value.as_object().unwrap().keys() {
            assert!(
                schema["properties"].get(key).is_some(),
                "unexpected wire field {key}"
            );
        }
        for key in schema["required"].as_array().unwrap() {
            assert!(value.get(key.as_str().unwrap()).is_some());
        }
        for excluded in [
            "task",
            "task_features",
            "prediction",
            "actual_harness",
            "mechanical_outcome",
            "verification",
            "source",
            "logs",
            "paths",
            "run_id",
        ] {
            assert!(value.get(excluded).is_none());
        }
        Ok(())
    }

    #[test]
    fn historical_feedback_reconciliation_recovers_only_current_known_state() -> Result<()> {
        for (parent_human, current_human, expected) in [
            (None, feedback(RoutingHumanOutcome::Accepted, 9), 1),
            (
                Some(feedback(RoutingHumanOutcome::Accepted, 8)),
                feedback(RoutingHumanOutcome::Rejected, 9),
                1,
            ),
            (
                Some(feedback(RoutingHumanOutcome::Accepted, 8)),
                feedback(RoutingHumanOutcome::Accepted, 9),
                0,
            ),
        ] {
            let (_temp, state, run) = persisted_routed_run(false)?;
            let mut database = Database::open(state.db_path())?;
            if let Some(human) = parent_human {
                database.save_routing_human_evaluation(&run.id, &human)?;
            }
            let parent = sync_parent_for_test(&state, &run.id)?;
            // Simulate the previous binary's post-sync replacement, with no event history.
            let mut observation = database.routing_observation_for_run(&run.id)?.unwrap();
            observation.human_evaluation = Some(current_human.clone());
            observation.updated_at = current_human.evaluated_at;
            rusqlite::Connection::open(state.db_path())?.execute(
                "UPDATE routing_observations SET observation_json = ?1 WHERE run_id = ?2",
                rusqlite::params![serde_json::to_string(&observation)?, run.id],
            )?;
            assert_eq!(database.reconcile_routing_feedback(None)?, expected);
            assert_eq!(database.reconcile_routing_feedback(None)?, 0);
            let events = database.routing_feedback_for_run(&run.id)?;
            assert_eq!(events.len(), expected);
            if let Some(event) = events.first() {
                assert_eq!(event.revision, 1);
                assert_eq!(event.evaluated_at, current_human.evaluated_at);
            }
            assert_eq!(
                database
                    .sync_payload_for_run(&run.id, SyncRecordType::RoutingObservationV1)?
                    .unwrap(),
                parent
            );
        }
        Ok(())
    }

    #[test]
    fn unsynced_parent_states_never_create_feedback_events() -> Result<()> {
        for status in ["not_queued", "pending", "failed", "conflict"] {
            let (_temp, state, run) = persisted_routed_run(false)?;
            enable_for_test(&state)?;
            let mut database = Database::open(state.db_path())?;
            if status != "not_queued" {
                preview_payload(&state, &run.id)?;
                rusqlite::Connection::open(state.db_path())?
                    .execute("UPDATE sync_outbox SET status = ?1", [status])?;
            }
            database.save_routing_human_evaluation(
                &run.id,
                &feedback(RoutingHumanOutcome::Accepted, 8),
            )?;
            assert_eq!(database.reconcile_routing_feedback(None)?, 0);
            assert!(database.routing_feedback_for_run(&run.id)?.is_empty());
            if status != "conflict" {
                let parent: RoutingObservationV1 =
                    serde_json::from_str(&preview_payload(&state, &run.id)?)?;
                assert_eq!(parent.human_evaluation.unwrap().outcome, "accepted");
            }
        }
        Ok(())
    }

    #[test]
    fn feedback_upload_uses_existing_outbox_and_preserves_retries_and_conflicts() -> Result<()> {
        for (status, error) in [
            (201, ""),
            (200, ""),
            (500, ""),
            (422, "idempotency_conflict"),
            (422, "revision_conflict"),
            (422, "invalid_parent"),
        ] {
            let (_temp, state, run) = persisted_routed_run(false)?;
            let parent = sync_parent_for_test(&state, &run.id)?;
            let mut database = Database::open(state.db_path())?;
            database.save_routing_human_evaluation(
                &run.id,
                &feedback(RoutingHumanOutcome::Accepted, 8),
            )?;
            set_token_for_test(&state, "feedback-test-token")?;
            let (_, preview) = preview_payload_for_revision(
                &state,
                &run.id,
                Some(SyncRecordType::RoutingFeedbackV1),
                None,
            )?;
            let (url, received, server) =
                mock_server_with_body(status, &format!(r#"{{"error":"{error}"}}"#))?;
            let report = flush_to(&state, &url)?;
            let request = received.recv()?;
            server.join().unwrap()?;
            let (headers, body) = split_request(&request)?;
            assert!(headers.starts_with("POST /v1/routing-feedback "));
            assert!(headers.contains(&format!("Idempotency-Key: routing-feedback-{}-1", run.id)));
            assert_eq!(body, preview.as_bytes());
            assert_eq!(report.synced, usize::from(status < 300));
            assert_eq!(
                database
                    .sync_payload_for_run(&run.id, SyncRecordType::RoutingObservationV1)?
                    .unwrap(),
                parent
            );
            assert_eq!(database.routing_feedback_for_run(&run.id)?.len(), 1);
            if status == 500 {
                assert_eq!(database.pending_sync_records()?[0].payload_json, preview);
                let (url, received, server) = mock_server(201)?;
                assert_eq!(flush_to(&state, &url)?.synced, 1);
                assert_eq!(split_request(&received.recv()?)?.1, preview.as_bytes());
                server.join().unwrap()?;
            } else if status == 422 {
                assert_eq!(database.sync_outbox_counts()?.conflict, 1);
                let stored: String = rusqlite::Connection::open(state.db_path())?.query_row(
                    "SELECT last_error FROM sync_outbox WHERE record_type = 'routing-feedback-v1'",
                    [],
                    |row| row.get(0),
                )?;
                assert!(stored.contains(error));
            }
            assert!(database.pending_sync_records()?.is_empty());
            assert_eq!(
                flush_to(&state, "https://must-not-contact.invalid")?.synced,
                0
            );
        }
        Ok(())
    }

    #[test]
    fn queued_feedback_requires_current_consent_and_token_at_transmission() -> Result<()> {
        let (_temp, state, run) = persisted_routed_run(false)?;
        sync_parent_for_test(&state, &run.id)?;
        let mut database = Database::open(state.db_path())?;
        database
            .save_routing_human_evaluation(&run.id, &feedback(RoutingHumanOutcome::Accepted, 8))?;
        assert!(
            flush_to(&state, "https://must-not-contact.invalid")
                .unwrap_err()
                .to_string()
                .contains("token not configured")
        );
        set_token_for_test(&state, "test-token")?;
        database.disable_sync()?;
        assert!(flush_to(&state, "https://must-not-contact.invalid").is_err());
        enable_legacy_for_test(&state)?;
        assert_eq!(
            flush_to(&state, "https://must-not-contact.invalid")?,
            SyncReport::default()
        );
        assert_eq!(database.pending_sync_records()?.len(), 1);
        Ok(())
    }

    #[test]
    fn concurrent_feedback_changes_allocate_unique_contiguous_revisions() -> Result<()> {
        let (_temp, state, run) = persisted_routed_run(false)?;
        sync_parent_for_test(&state, &run.id)?;
        let threads: Vec<_> = (0..6)
            .map(|index| {
                let path = state.db_path();
                let run_id = run.id.clone();
                thread::spawn(move || -> Result<()> {
                    let mut human = feedback(RoutingHumanOutcome::Accepted, 8);
                    human.explanation = Some(format!("Review {index}"));
                    Database::open(path)?.save_routing_human_evaluation(&run_id, &human)?;
                    Ok(())
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap()?;
        }
        let events = Database::open(state.db_path())?.routing_feedback_for_run(&run.id)?;
        assert_eq!(
            events.iter().map(|e| e.revision).collect::<Vec<_>>(),
            [1, 2, 3, 4, 5, 6]
        );
        Ok(())
    }

    #[test]
    fn feedback_uploads_in_numeric_revision_order_without_reposting_parent() -> Result<()> {
        let (_temp, state, run) = persisted_routed_run(false)?;
        let parent = sync_parent_for_test(&state, &run.id)?;
        let mut database = Database::open(state.db_path())?;
        for revision in 1..=12 {
            let mut human = feedback(RoutingHumanOutcome::Accepted, 8);
            human.explanation = Some(format!("Revision {revision}"));
            database.save_routing_human_evaluation(&run.id, &human)?;
        }
        set_token_for_test(&state, "ordering-test-token")?;
        let (url, requests, server) = mock_server_responses(&vec![(201, ""); 12])?;
        assert_eq!(flush_to(&state, &url)?.synced, 12);
        for revision in 1..=12 {
            let request = requests.recv()?;
            let (headers, body) = split_request(&request)?;
            assert!(headers.starts_with("POST /v1/routing-feedback "));
            assert_eq!(
                serde_json::from_slice::<RoutingFeedbackV1>(body)?.revision,
                revision
            );
            assert_eq!(
                body,
                preview_payload_for_revision(
                    &state,
                    &run.id,
                    Some(SyncRecordType::RoutingFeedbackV1),
                    Some(revision)
                )?
                .1
                .as_bytes()
            );
        }
        server.join().unwrap()?;
        assert_eq!(
            database
                .sync_payload_for_run(&run.id, SyncRecordType::RoutingObservationV1)?
                .unwrap(),
            parent
        );
        Ok(())
    }

    #[test]
    fn immutable_feedback_event_survives_outbox_reconstruction() -> Result<()> {
        let (_temp, state, run) = persisted_routed_run(false)?;
        sync_parent_for_test(&state, &run.id)?;
        let mut database = Database::open(state.db_path())?;
        database
            .save_routing_human_evaluation(&run.id, &feedback(RoutingHumanOutcome::Accepted, 8))?;
        let event = database.routing_feedback_for_run(&run.id)?.remove(0);
        let payload = database.sync_payload_for_record(&event.feedback_event_id)?;
        assert!(
            database
                .enqueue_sync(
                    SyncRecordType::RoutingFeedbackV1,
                    &event.feedback_event_id,
                    &run.id,
                    "changed payload"
                )
                .is_err()
        );
        rusqlite::Connection::open(state.db_path())?.execute(
            "DELETE FROM sync_outbox WHERE record_id = ?1",
            [&event.feedback_event_id],
        )?;
        assert_eq!(
            database.routing_feedback_for_run(&run.id)?.as_slice(),
            std::slice::from_ref(&event)
        );
        assert_eq!(database.reconcile_routing_feedback(Some(&run.id))?, 0);
        assert_eq!(
            database.sync_payload_for_record(&event.feedback_event_id)?,
            payload
        );
        assert_eq!(database.routing_feedback_for_run(&run.id)?, [event]);
        Ok(())
    }

    #[test]
    fn sync_reuses_locks_to_prevent_pending_payload_and_evaluation_races() -> Result<()> {
        let (_temp, state, run) = persisted_routed_run(false)?;
        enable_for_test(&state)?;
        set_token_for_test(&state, "lock-test-token")?;
        let lock = OperationLock::acquire(&state.run_dir(&run.id).join(".operation.lock"), "busy")?;
        assert!(
            preview_payload(&state, &run.id)
                .unwrap_err()
                .to_string()
                .contains("another operation")
        );
        assert!(
            flush_to(&state, "https://must-not-contact.invalid")
                .unwrap_err()
                .to_string()
                .contains("another operation")
        );
        drop(lock);
        let _lock = lock_sync(&state)?;
        assert!(
            preview_payload(&state, &run.id)
                .unwrap_err()
                .to_string()
                .contains("another sync")
        );
        assert!(
            flush_to(&state, "https://must-not-contact.invalid")
                .unwrap_err()
                .to_string()
                .contains("another sync")
        );
        Ok(())
    }

    #[test]
    fn routing_observation_v1_preserves_independent_signals_and_privacy() -> Result<()> {
        let human = RoutingHumanEvaluation {
            outcome: RoutingHumanOutcome::Rejected,
            reasons: vec!["correctness".into(), "tests".into()],
            explanation: Some("The result needs more work.\n  Keep this spacing.".into()),
            evaluated_at: at(5),
        };
        let observation = routing_observation(Some(vec![CheckStatus::Passed]), Some(human.clone()));
        let envelope = routing_observation_envelope_for(&observation, &current_settings())?;
        let payload = serialize_routing_observation(&envelope)?;
        let value: serde_json::Value = serde_json::from_str(&payload)?;

        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["observation_id"], observation.id);
        assert_eq!(value["consent"]["scope"], "routing-observation-v1");
        assert_eq!(value["prediction"]["version"], 1);
        assert_eq!(value["prediction"]["task_features"]["language"], "rust");
        assert_eq!(value["prediction"]["selected_harness"], "cursor");
        assert_eq!(value["prediction"]["successes"], 37);
        assert_eq!(value["prediction"]["attempts"], 50);
        assert_eq!(value["prediction"]["specificity"], 2);
        assert_eq!(value["prediction"]["model"], "benchmark/model");
        assert_eq!(value["actual_harness"]["harness"], "cursor");
        assert_eq!(value["actual_harness"]["model"], "execution/model");
        assert_eq!(value["mechanical_outcome"]["verification"][0], "passed");
        assert_eq!(value["human_evaluation"]["outcome"], "rejected");
        assert_eq!(value["human_evaluation"]["reasons"][1], "tests");
        assert_eq!(
            value["human_evaluation"]["explanation"],
            human.explanation.unwrap()
        );
        let decoded: RoutingObservationV1 = serde_json::from_str(&payload)?;
        assert_eq!(decoded, envelope);

        for excluded in [
            "Fix the retry race",
            "/Users/alice",
            "private system prompt",
            "private-head",
            "diff.patch",
            "changed_files",
            "stdout_path",
            "verification_command",
            "workspace_path",
        ] {
            assert!(!payload.contains(excluded), "payload leaked {excluded}");
        }

        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../schemas/routing-observation-v1.json"))?;
        assert_eq!(schema["properties"]["schema_version"]["const"], 1);
        assert_eq!(
            schema["$defs"]["consent"]["properties"]["scope"]["const"],
            "routing-observation-v1"
        );
        assert_eq!(
            schema["$defs"]["prediction"]["properties"]["version"]["const"],
            1
        );
        Ok(())
    }

    #[test]
    fn routing_observation_v1_preserves_failure_unknown_and_not_run_states() -> Result<()> {
        let accepted = RoutingHumanEvaluation {
            outcome: RoutingHumanOutcome::Accepted,
            reasons: vec![],
            explanation: None,
            evaluated_at: at(5),
        };
        let failed = routing_observation(Some(vec![CheckStatus::Failed]), Some(accepted.clone()));
        let failed = routing_observation_envelope_for(&failed, &current_settings())?;
        assert_eq!(
            failed.mechanical_outcome.verification,
            Some(vec!["failed".into()])
        );
        assert_eq!(failed.human_evaluation.unwrap().outcome, "accepted");

        let unknown = routing_observation(None, None);
        let unknown = routing_observation_envelope_for(&unknown, &current_settings())?;
        assert!(unknown.mechanical_outcome.verification.is_none());
        assert!(unknown.human_evaluation.is_none());

        let not_run = routing_observation(Some(vec![CheckStatus::NotRun]), Some(accepted));
        let not_run = routing_observation_envelope_for(&not_run, &current_settings())?;
        assert_eq!(
            not_run.mechanical_outcome.verification,
            Some(vec!["not_run".into()])
        );
        assert_eq!(not_run.human_evaluation.unwrap().outcome, "accepted");
        Ok(())
    }

    #[test]
    fn routing_observations_require_explicit_upgraded_consent() -> Result<()> {
        let (_temp, state, run) = persisted_routed_run(false)?;
        let mut database = Database::open(state.db_path())?;
        assert!(!database.sync_settings()?.enabled);
        assert!(database.pending_sync_records()?.is_empty());

        database.save_routing_human_evaluation(
            &run.id,
            &RoutingHumanEvaluation {
                outcome: RoutingHumanOutcome::Accepted,
                reasons: vec![],
                explanation: None,
                evaluated_at: Utc::now(),
            },
        )?;
        database.set_sync_token("token-without-consent")?;
        drop(database);
        assert!(flush_to(&state, "https://not-contacted.invalid").is_err());
        assert!(
            Database::open(state.db_path())?
                .pending_sync_records()?
                .is_empty()
        );

        let legacy = enable_legacy_for_test(&state)?;
        let database = Database::open(state.db_path())?;
        assert_eq!(legacy.consent_version, 1);
        assert_eq!(queue_all(&state, &database, &legacy)?, 0);
        assert!(database.pending_sync_records()?.is_empty());
        database.clear_sync_token()?;
        drop(database);

        let mut database = Database::open(state.db_path())?;
        let current = database.enable_sync("replacement", Utc::now())?;
        assert_eq!(current.consent_version, 2);
        assert!(current.routing_observation_enabled_at.is_some());
        assert_eq!(queue_all(&state, &database, &current)?, 1);
        assert_eq!(queue_all(&state, &database, &current)?, 0);
        let records = database.pending_sync_records()?;
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].record_id,
            format!("routing-observation-{}", run.id)
        );
        assert_eq!(records[0].record_type, SyncRecordType::RoutingObservationV1);
        drop(database);
        let error = flush_to(&state, "http://127.0.0.1:9")
            .unwrap_err()
            .to_string();
        assert!(error.contains("ingestion token not configured"));
        assert_eq!(
            Database::open(state.db_path())?
                .sync_outbox_counts()?
                .pending,
            1
        );
        Ok(())
    }

    #[test]
    fn routing_preview_is_explicit_when_a_run_has_two_record_types() -> Result<()> {
        let (_temp, state, run) = persisted_routed_run(true)?;
        let settings = enable_for_test(&state)?;
        let database = Database::open(state.db_path())?;
        assert_eq!(queue_all(&state, &database, &settings)?, 2);

        let error = preview_payload(&state, &run.id).unwrap_err().to_string();
        assert!(error.contains("--type"));
        let (evaluation_type, evaluation) =
            preview_payload_for_type(&state, &run.id, Some(SyncRecordType::EvaluationV1))?;
        assert_eq!(evaluation_type, SyncRecordType::EvaluationV1);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&evaluation)?["schema_version"],
            1
        );
        let (routing_type, routing) =
            preview_payload_for_type(&state, &run.id, Some(SyncRecordType::RoutingObservationV1))?;
        assert_eq!(routing_type, SyncRecordType::RoutingObservationV1);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&routing)?["observation_id"],
            format!("routing-observation-{}", run.id)
        );
        Ok(())
    }

    #[test]
    fn pending_routing_payload_refreshes_before_upload_but_synced_payload_is_immutable()
    -> Result<()> {
        let (_temp, state, run) = persisted_routed_run(false)?;
        let settings = enable_for_test(&state)?;
        let database = Database::open(state.db_path())?;
        assert_eq!(queue_all(&state, &database, &settings)?, 1);
        assert!(
            serde_json::from_str::<serde_json::Value>(
                &database.pending_sync_records()?[0].payload_json
            )?["human_evaluation"]
                .is_null()
        );
        drop(database);

        let explanation = "Accepted after review.\nKeep this verbatim.  \n";
        let mut database = Database::open(state.db_path())?;
        database.save_routing_human_evaluation(
            &run.id,
            &RoutingHumanEvaluation {
                outcome: RoutingHumanOutcome::Accepted,
                reasons: vec!["correctness".into(), "tests".into()],
                explanation: Some(explanation.into()),
                evaluated_at: Utc::now(),
            },
        )?;
        database.set_sync_token("routing-preview-token")?;
        drop(database);

        let (_, preview) =
            preview_payload_for_type(&state, &run.id, Some(SyncRecordType::RoutingObservationV1))?;
        let preview_value: serde_json::Value = serde_json::from_str(&preview)?;
        assert_eq!(preview_value["human_evaluation"]["outcome"], "accepted");
        assert_eq!(
            preview_value["human_evaluation"]["explanation"],
            explanation
        );

        let (url, received, server) = mock_server(201)?;
        assert_eq!(
            flush_to(&state, &url)?,
            SyncReport {
                synced: 1,
                failed: 0
            }
        );
        let request = received.recv()?;
        server.join().unwrap()?;
        let (headers, body) = split_request(&request)?;
        assert!(headers.starts_with("POST /v1/routing-observations "));
        assert!(headers.contains(&format!("Idempotency-Key: routing-observation-{}", run.id)));
        assert_eq!(body, preview.as_bytes());

        let mut database = Database::open(state.db_path())?;
        database.save_routing_human_evaluation(
            &run.id,
            &RoutingHumanEvaluation {
                outcome: RoutingHumanOutcome::Rejected,
                reasons: vec!["correctness".into()],
                explanation: None,
                evaluated_at: Utc::now(),
            },
        )?;
        drop(database);
        assert_eq!(
            preview_payload_for_type(&state, &run.id, Some(SyncRecordType::RoutingObservationV1))?
                .1,
            preview
        );
        let events = Database::open(state.db_path())?.routing_feedback_for_run(&run.id)?;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].outcome, "reject");
        assert_eq!(
            Database::open(state.db_path())?
                .sync_outbox_counts()?
                .synced,
            1
        );
        Ok(())
    }

    #[test]
    fn routing_upload_handles_replay_retry_and_idempotency_conflict() -> Result<()> {
        for status in [200, 500, 422] {
            let (_temp, state, run) = persisted_routed_run(false)?;
            enable_for_test(&state)?;
            set_token_for_test(&state, "routing-status-token")?;
            let before = Database::open(state.db_path())?
                .routing_observation_for_run(&run.id)?
                .unwrap();
            let (url, received, server) = match status {
                200 => mock_server_with_body(
                    status,
                    &format!(
                        r#"{{"status":"already_exists","observation_id":"routing-observation-{}"}}"#,
                        run.id
                    ),
                )?,
                422 => mock_server_with_body(
                    status,
                    r#"{"error":"idempotency_conflict","message":"different payload"}"#,
                )?,
                _ => mock_server(status)?,
            };
            let report = flush_to(&state, &url)?;
            received.recv()?;
            server.join().unwrap()?;
            let counts = Database::open(state.db_path())?.sync_outbox_counts()?;
            match status {
                200 => {
                    assert_eq!(report.synced, 1);
                    assert_eq!(counts.synced, 1);
                    assert_eq!(
                        flush_to(&state, "https://repeat-must-not-contact.invalid")?,
                        SyncReport {
                            synced: 0,
                            failed: 0
                        }
                    );
                    let connection = rusqlite::Connection::open(state.db_path())?;
                    let count: i64 = connection.query_row(
                        "SELECT COUNT(*) FROM sync_outbox WHERE record_id = ?1",
                        [format!("routing-observation-{}", run.id)],
                        |row| row.get(0),
                    )?;
                    assert_eq!(count, 1);
                }
                500 => {
                    assert_eq!(report.failed, 1);
                    assert_eq!(counts.failed, 1);
                    assert_eq!(
                        Database::open(state.db_path())?
                            .pending_sync_records()?
                            .len(),
                        1
                    );
                }
                422 => {
                    assert_eq!(report.failed, 1);
                    assert_eq!(counts.conflict, 1);
                    assert!(
                        Database::open(state.db_path())?
                            .pending_sync_records()?
                            .is_empty()
                    );
                    let connection = rusqlite::Connection::open(state.db_path())?;
                    let (record_status, error): (String, String) = connection.query_row(
                        "SELECT status, last_error FROM sync_outbox WHERE record_id = ?1",
                        [format!("routing-observation-{}", run.id)],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )?;
                    assert_eq!(record_status, "conflict");
                    assert!(error.contains("idempotency_conflict"));
                    assert_eq!(
                        Database::open(state.db_path())?
                            .routing_observation_for_run(&run.id)?
                            .unwrap(),
                        before
                    );
                    assert_eq!(
                        flush_to(&state, "https://conflict-must-not-retry.invalid")?,
                        SyncReport {
                            synced: 0,
                            failed: 0
                        }
                    );
                }
                _ => unreachable!(),
            }
        }
        Ok(())
    }

    /// S1 (attach part 6.3/6.6/14.6): attached work is observed, not
    /// selected or routed, and never becomes evaluation or routing evidence.
    /// Even if something upstream ever set `evaluation` on an attached-mode
    /// run, `queue_evaluation` refuses to enqueue it for upload.
    #[test]
    fn attached_runs_are_never_queued_for_evaluation_upload() -> Result<()> {
        let mut run = evaluated_run("attached-run");
        run.mode = crate::RunMode::Attached;
        let settings = SyncSettings {
            enabled: true,
            contributor_id: Some("contributor".into()),
            enabled_at: Some(at(6).to_rfc3339()),
            consent_version: 2,
            routing_observation_enabled_at: Some(at(6).to_rfc3339()),
        };
        let database = Database::open_in_memory()?;
        let error = queue_evaluation(&database, &settings, &run)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("is never queued for evaluation upload"),
            "{error}"
        );

        // A legacy-mode run with the same evaluation is unaffected by the
        // guard (it still requires a persisted evaluation, which this
        // in-memory database was never given): the mode guard is not simply
        // rejecting every run.
        run.mode = crate::RunMode::Legacy;
        let error = queue_evaluation(&database, &settings, &run)
            .unwrap_err()
            .to_string();
        assert!(
            !error.contains("is never queued for evaluation upload"),
            "{error}"
        );
        assert!(error.contains("no persisted evaluation"), "{error}");
        Ok(())
    }

    #[test]
    fn envelope_preserves_semantics_unknowns_and_privacy_boundary() -> Result<()> {
        let run = evaluated_run("run-envelope");
        let settings = SyncSettings {
            enabled: true,
            contributor_id: Some("contributor".into()),
            enabled_at: Some(at(6).to_rfc3339()),
            consent_version: 2,
            routing_observation_enabled_at: Some(at(6).to_rfc3339()),
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
            consent_version: 2,
            routing_observation_enabled_at: Some(at(6).to_rfc3339()),
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
        assert_eq!(queued[0].record_id, format!("evaluation-{}", run.id));
        assert_eq!(queued[0].record_type, SyncRecordType::EvaluationV1);
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
    fn enable_records_consent_and_queues_without_uploading() -> Result<()> {
        let (_temp, state, run) = persisted_run()?;
        set_token_for_test(&state, "configured-before-consent")?;

        enable(&state)?;

        let database = Database::open(state.db_path())?;
        let settings = database.sync_settings()?;
        assert!(settings.enabled);
        assert!(settings.contributor_id.is_some());
        assert!(settings.enabled_at.is_some());
        assert_eq!(database.sync_outbox_counts()?.pending, 1);
        let connection = rusqlite::Connection::open(state.db_path())?;
        let (status, last_attempt, synced_at): (String, Option<String>, Option<String>) =
            connection.query_row(
                "SELECT status, last_attempt, synced_at FROM sync_outbox WHERE run_id = ?1",
                [&run.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
        assert_eq!(status, "pending");
        assert!(last_attempt.is_none());
        assert!(synced_at.is_none());
        Ok(())
    }

    #[test]
    fn preview_requires_consent() -> Result<()> {
        let (_temp, state, run) = persisted_run()?;
        let error = preview_payload(&state, &run.id).unwrap_err().to_string();
        assert!(error.contains("dispatch sync enable"));
        assert!(
            Database::open(state.db_path())?
                .pending_sync_records()?
                .is_empty()
        );
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
            "SELECT status, last_error FROM sync_outbox WHERE record_id = ?1",
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
        assert!(error.contains("authentication/authorization failure (HTTP 401)"));
        assert!(!error.contains("token was rejected"));
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
        mock_server_with_body(status, "")
    }

    fn mock_server_with_body(status: u16, response_body: &str) -> Result<MockServer> {
        mock_server_responses(&[(status, response_body)])
    }

    fn mock_server_responses(responses: &[(u16, &str)]) -> Result<MockServer> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let (sender, receiver) = mpsc::channel();
        let responses: Vec<_> = responses
            .iter()
            .map(|(status, body)| (*status, body.to_string()))
            .collect();
        let handle = thread::spawn(move || -> Result<()> {
            for (status, response_body) in responses {
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
                    422 => "Unprocessable Entity",
                    401 => "Unauthorized",
                    _ => "Server Error",
                };
                write!(
                    stream,
                    "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response_body.len(),
                    response_body
                )?;
            }
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
