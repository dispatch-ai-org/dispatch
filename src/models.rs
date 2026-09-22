use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    BugFix,
    Feature,
    Refactor,
    Tests,
    #[default]
    Unknown,
}

impl TaskKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BugFix => "bug_fix",
            Self::Feature => "feature",
            Self::Refactor => "refactor",
            Self::Tests => "tests",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskScope {
    Localized,
    MultiFile,
    Broad,
    #[default]
    Unknown,
}

impl TaskScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Localized => "localized",
            Self::MultiFile => "multi_file",
            Self::Broad => "broad",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskFeatures {
    pub language: Option<String>,
    pub task_kind: TaskKind,
    pub scope: TaskScope,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutingDecision {
    pub version: u32,
    pub task_features: TaskFeatures,
    pub selected_harness: String,
    pub successes: u64,
    pub attempts: u64,
    pub specificity: u8,
    pub source: String,
    pub dataset: String,
    pub dataset_version: String,
    pub model: Option<String>,
    #[serde(default)]
    pub selection_basis: SelectionBasis,
    #[serde(default)]
    pub alternatives: Vec<RoutingAlternative>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SelectionBasis {
    #[default]
    Evidence,
    Default,
    Override,
}

impl SelectionBasis {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Evidence => "Evidence-based",
            Self::Default => "Dispatch default",
            Self::Override => "Agent override",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutingAlternative {
    pub harness: String,
    pub successes: u64,
    pub attempts: u64,
    pub specificity: Option<u8>,
    pub source: Option<String>,
    pub dataset: Option<String>,
    pub dataset_version: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoutingHumanOutcome {
    Accepted,
    Rejected,
}

impl RoutingHumanOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutingHumanEvaluation {
    pub outcome: RoutingHumanOutcome,
    #[serde(default)]
    pub reasons: Vec<String>,
    pub explanation: Option<String>,
    pub evaluated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutingObservation {
    pub id: String,
    pub run_id: String,
    pub candidate_id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub prediction: RoutingDecision,
    pub harness_version: Option<String>,
    pub model: Option<String>,
    pub candidate_status: CandidateStatus,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub verification: Option<Vec<CheckStatus>>,
    pub human_evaluation: Option<RoutingHumanEvaluation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BenchmarkPrior {
    pub source: String,
    pub dataset: String,
    pub dataset_version: String,
    pub harness: String,
    pub model: Option<String>,
    pub language: Option<String>,
    pub task_kind: TaskKind,
    pub scope: TaskScope,
    pub successes: u64,
    pub attempts: u64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Git,
    GitWorktree,
    Directory,
}

impl SourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Git => "git",
            Self::GitWorktree => "git_worktree",
            Self::Directory => "directory",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Preparing,
    Running,
    ReadyForEvaluation,
    Evaluated,
    Applied,
    Interrupted,
    Deferred,
    Failed,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    #[default]
    Legacy,
    Routed,
    Allocation,
    Comparison,
}

impl RunMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::Routed => "routed",
            Self::Allocation => "allocation",
            Self::Comparison => "comparison",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    Preparing,
    Working,
    Waiting,
    #[default]
    Finished,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkResult {
    #[default]
    Pending,
    Ready,
    Failed,
    Cancelled,
    Interrupted,
    Deferred,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerificationState {
    #[default]
    NotConfigured,
    NotRun,
    Passed,
    Failed,
    Inconclusive,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    NotRequested,
    #[default]
    Pending,
    Accepted,
    Rejected,
    Deferred,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationState {
    #[default]
    NotApplied,
    Applied,
    BlockedBySourceDrift,
    Failed,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunPhase {
    Preparing,
    Planning,
    Integrating,
    Executing,
    Verifying,
    Reviewing,
    Applying,
    #[default]
    Finished,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WaitingOn {
    #[default]
    None,
    Human,
    Capacity,
    Dependency,
    Authorization,
    Admission,
    Reconciliation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunOutcome {
    pub version: u32,
    pub lifecycle: LifecycleState,
    pub work_result: WorkResult,
    pub verification: VerificationState,
    pub review: ReviewState,
    pub application: ApplicationState,
    pub phase: RunPhase,
    pub waiting_on: WaitingOn,
    /// Who applied the result: a human decision or the auto-apply policy.
    /// `None` when nothing was applied, or for records written before the
    /// actor was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_by: Option<AppliedBy>,
}

/// The authority under which a candidate was applied to the source. It is
/// recorded on the outcome so that an application by policy can never be
/// mistaken for human acceptance.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppliedBy {
    Human,
    AutoApply,
}

impl Default for RunOutcome {
    fn default() -> Self {
        Self {
            version: 1,
            lifecycle: LifecycleState::Finished,
            work_result: WorkResult::Pending,
            verification: VerificationState::NotConfigured,
            review: ReviewState::Pending,
            application: ApplicationState::NotApplied,
            phase: RunPhase::Finished,
            waiting_on: WaitingOn::None,
            applied_by: None,
        }
    }
}

impl RunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::Running => "running",
            Self::ReadyForEvaluation => "ready_for_evaluation",
            Self::Evaluated => "evaluated",
            Self::Applied => "applied",
            Self::Interrupted => "interrupted",
            Self::Deferred => "deferred",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CandidateStatus {
    Preparing,
    Running,
    Verifying,
    Completed,
    Failed,
    TimedOut,
    Cancelled,
    MissingHarness,
}

impl CandidateStatus {
    pub(crate) fn is_terminal(&self) -> bool {
        !matches!(self, Self::Preparing | Self::Running | Self::Verifying)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::Running => "running",
            Self::Verifying => "verifying",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
            Self::Cancelled => "cancelled",
            Self::MissingHarness => "missing_harness",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckPhase {
    Baseline,
    Verify,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Passed,
    Failed,
    TimedOut,
    NotRun,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub name: String,
    pub phase: CheckPhase,
    pub command: String,
    pub status: CheckStatus,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub stdout_path: PathBuf,
    pub stderr_path: PathBuf,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiffStats {
    pub files_changed: u64,
    pub lines_added: u64,
    pub lines_removed: u64,
    #[serde(default)]
    pub changed_files: Vec<String>,
    #[serde(default)]
    pub untracked_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateRecord {
    pub id: String,
    pub label: String,
    pub harness_id: String,
    pub harness_version: Option<String>,
    pub model: Option<String>,
    pub status: CandidateStatus,
    pub workspace_path: PathBuf,
    pub prompt_path: PathBuf,
    pub stdout_path: PathBuf,
    pub stderr_path: PathBuf,
    pub diff_path: PathBuf,
    pub duration_ms: u64,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    /// Uncached input plus output, with cache creation included when reported separately.
    pub tokens: Option<u64>,
    #[serde(default)]
    pub token_semantics: Option<String>,
    pub cost_usd: Option<f64>,
    pub error: Option<String>,
    pub diff_stats: DiffStats,
    #[serde(default)]
    pub checks: Vec<CheckResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentRecord {
    pub dispatch_version: String,
    pub os: String,
    pub architecture: String,
    pub execution_backend: String,
    pub timeout_secs: u64,
    pub cpus: f64,
    pub memory: String,
    pub max_parallel: usize,
    #[serde(default)]
    pub docker_image: Option<String>,
    #[serde(default)]
    pub resource_limits_enforced: bool,
    #[serde(default)]
    pub unsafe_local: bool,
    #[serde(default)]
    pub forwarded_env: Vec<String>,
}

/// Latest work-coherence state of a run against the moving source tree.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoherenceRecord {
    #[serde(default = "coherence_version")]
    pub version: u32,
    #[serde(default)]
    pub refreshed_from: Option<String>,
    #[serde(default)]
    pub facts: Vec<MustHold>,
    #[serde(default)]
    pub validity: Option<Validity>,
    #[serde(default)]
    pub first_invalid_at: Option<DateTime<Utc>>,
}

fn coherence_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MustHold {
    pub id: String,
    pub kind: FactKind,
    pub path: String,
    #[serde(default)]
    pub subject: String,
    pub origin: FactOrigin,
    #[serde(default)]
    pub sig_fp: String,
    #[serde(default)]
    pub full_fp: Option<String>,
    #[serde(default)]
    pub display: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FactKind {
    Signature,
    File,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FactOrigin {
    Modified,
    Referenced,
    FileFallback,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Continue,
    Refresh,
    Stop,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisLevel {
    Symbols,
    FilesOnly,
    Integration,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReasonCode {
    FactBroken,
    FactMissing,
    SameSymbolEdited,
    PatchConflict,
    AlreadyApplied,
    IntegrationCheckFailed,
    AnalysisUncertain,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Reason {
    pub code: ReasonCode,
    #[serde(default)]
    pub fact_id: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Validity {
    pub decision: Decision,
    pub evaluated_at: DateTime<Utc>,
    #[serde(default)]
    pub world_digest: String,
    #[serde(default)]
    pub world_changed: bool,
    #[serde(default)]
    pub changed_files: u32,
    /// Capped at 20.
    #[serde(default)]
    pub reasons: Vec<Reason>,
    pub analysis: AnalysisLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoherenceSummary {
    pub decision: Decision,
    /// Capped at 5.
    #[serde(default)]
    pub reasons: Vec<Reason>,
    #[serde(default)]
    pub changed_files: u32,
    pub analysis: AnalysisLevel,
}

/// The JSON/JSONL projection of an automatic application attempt
/// (`orchestrator::apply::ApplyOutcome`), carried on `RunResult.auto_apply`
/// only when `run --auto-apply`/`refresh --auto-apply` attempted one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutoApplySummary {
    /// "applied" | "blocked" | "skipped" | "failed".
    pub outcome: String,
    /// The skip/block reason, or the error text for a failed application.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The validity the decision was made against, present only when one
    /// exists (an unmoved world produces none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coherence: Option<CoherenceSummary>,
    #[serde(default)]
    pub files_changed: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    #[serde(default)]
    pub phase3: Option<GoalExecution>,
    pub id: String,
    pub task: String,
    pub exact_prompt: String,
    pub source_path: PathBuf,
    pub source_kind: SourceKind,
    pub source_git_head: Option<String>,
    pub source_fingerprint: String,
    pub baseline_path: PathBuf,
    pub baseline_commit: String,
    pub status: RunStatus,
    #[serde(default)]
    pub mode: RunMode,
    #[serde(default)]
    pub state_revision: u64,
    #[serde(default)]
    pub outcome: RunOutcome,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub environment: EnvironmentRecord,
    #[serde(default)]
    pub baseline_checks: Vec<CheckResult>,
    #[serde(default)]
    pub candidates: Vec<CandidateRecord>,
    #[serde(default)]
    pub attempts: Vec<AttemptRecord>,
    #[serde(default)]
    pub routing: Option<RoutingDecision>,
    #[serde(default)]
    pub allocation: Option<AllocationDecision>,
    #[serde(default)]
    pub capacity: Option<CapacityObservation>,
    #[serde(default)]
    pub admission: Option<AdmissionSummary>,
    #[serde(default)]
    pub coherence: Option<CoherenceRecord>,
    pub evaluation: Option<EvaluationRecord>,
    pub applied_candidate: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "candidate", rename_all = "snake_case")]
pub enum EvaluationOutcome {
    Candidate(String),
    Tie,
    Neither,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvaluationRecord {
    pub outcome: EvaluationOutcome,
    #[serde(default)]
    pub reasons: Vec<String>,
    pub explanation: Option<String>,
    pub created_at: DateTime<Utc>,
    pub blind: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EventRecord {
    #[serde(default = "event_protocol_version")]
    pub protocol_version: u32,
    pub run_id: String,
    #[serde(default)]
    pub sequence: u64,
    #[serde(default)]
    pub attempt_id: Option<String>,
    #[serde(default)]
    pub generation: u32,
    #[serde(default = "default_event_actor")]
    pub actor: String,
    pub candidate_label: Option<String>,
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
    pub payload: serde_json::Value,
}

fn event_protocol_version() -> u32 {
    1
}

fn default_event_actor() -> String {
    "legacy".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttemptRecord {
    #[serde(default)]
    pub detail: AttemptDetail,
    pub id: String,
    pub run_id: String,
    pub candidate_id: String,
    pub role: String,
    pub ordinal: u32,
    pub generation: u32,
    pub harness_id: String,
    pub harness_version: Option<String>,
    pub requested_model: Option<String>,
    pub resolved_model: Option<String>,
    pub observed_model: Option<String>,
    pub requested_effort: Option<String>,
    pub resolved_effort: Option<String>,
    pub observed_effort: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub outcome: String,
    pub raw_telemetry_path: PathBuf,
    #[serde(default)]
    pub resource: Option<ResourceChoice>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResourceTier {
    Light,
    Standard,
    Strong,
}

impl ResourceTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Standard => "standard",
            Self::Strong => "strong",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceChoice {
    pub provider: String,
    pub funding_source: String,
    pub harness: String,
    pub requested_model: String,
    pub resolved_model: String,
    pub effort: Option<String>,
    pub service_mode: String,
    pub runtime: String,
    pub pool: String,
    pub tier: ResourceTier,
    pub no_overage_verified: bool,
    pub internal_composition: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelCapability {
    pub model: String,
    #[serde(default)]
    pub efforts: Vec<String>,
    pub included: bool,
    pub no_overage_verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilitySnapshot {
    pub version: u32,
    pub source: String,
    pub harness: String,
    pub observed_at: DateTime<Utc>,
    pub models: Vec<ModelCapability>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AllocationAlternative {
    pub choice: ResourceChoice,
    pub eligible: bool,
    pub exclusion: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AllocationDecision {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_evidence: Option<crate::private_evidence::DecisionEvidence>,
    pub version: u32,
    pub policy_version: String,
    pub task_features: TaskFeatures,
    pub selected: ResourceChoice,
    pub reason: String,
    pub capability: CapabilitySnapshot,
    pub alternatives: Vec<AllocationAlternative>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "knowledge", rename_all = "snake_case")]
pub enum CapacityValue<T> {
    Reported {
        value: T,
    },
    Estimated {
        lower: T,
        upper: T,
        method: String,
        samples: u32,
    },
    Unknown {
        reason: String,
    },
}

impl<T> CapacityValue<T> {
    pub fn unknown(reason: impl Into<String>) -> Self {
        Self::Unknown {
            reason: reason.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapacityMapping {
    Mapped,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CapacityConstraint {
    pub kind: String,
    pub unit: String,
    pub provider_bucket_id: Option<String>,
    pub window_id: Option<String>,
    pub reported_used_percent: Option<f64>,
    pub remaining: CapacityValue<f64>,
    pub reset_at: CapacityValue<DateTime<Utc>>,
    pub window_duration_secs: CapacityValue<u64>,
    pub scope: CapacityValue<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScarcityState {
    Available,
    Constrained,
    Reserve,
    Exhausted,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CapacityObservation {
    pub id: String,
    pub pool_id: String,
    pub source: String,
    pub source_version: String,
    pub sampled_at: DateTime<Utc>,
    pub valid_until: DateTime<Utc>,
    pub mapping: CapacityMapping,
    pub auth_mode: CapacityValue<String>,
    #[serde(default = "unknown_funding_identity")]
    pub funding_identity: CapacityValue<String>,
    pub plan_type: CapacityValue<String>,
    pub credits_available: CapacityValue<bool>,
    pub service_tier: CapacityValue<String>,
    pub constraints: Vec<CapacityConstraint>,
    pub scarcity: ScarcityState,
    pub attributable_attempt_id: Option<String>,
    pub attribution: String,
    pub raw_observation_ref: Option<PathBuf>,
}

fn unknown_funding_identity() -> CapacityValue<String> {
    CapacityValue::unknown("funding identity not recorded")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionState {
    Queued,
    Admitted,
    Reconciliation,
    Released,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdmissionSummary {
    pub request_id: String,
    pub pool_id: String,
    #[serde(default)]
    pub attempt_id: String,
    #[serde(default)]
    pub route_snapshot_json: String,
    #[serde(default)]
    pub configuration_revision: String,
    #[serde(default)]
    pub canonical_pool_identity: String,
    #[serde(default)]
    pub authorization_id: String,
    #[serde(default)]
    pub authorization_revision: u64,
    pub owner_session: String,
    pub generation: u64,
    pub priority: i32,
    pub state: AdmissionState,
    pub fence: Option<u64>,
    pub enqueued_at: DateTime<Utc>,
    pub released_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoalFeedbackRevision {
    pub id: String,
    pub run_id: String,
    pub revision: u32,
    pub outcome: RoutingHumanOutcome,
    #[serde(default)]
    pub reasons: Vec<String>,
    pub explanation: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    pub phase3: Option<GoalExecution>,
    pub elapsed_ms: i64,
    pub schema_version: u32,
    pub run_id: String,
    pub mode: RunMode,
    pub state_revision: u64,
    pub outcome: RunOutcome,
    pub exit_code: i32,
    pub attempts: Vec<AttemptRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allocation: Option<AllocationDecision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capacity: Option<CapacityObservation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admission: Option<AdmissionSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coherence: Option<CoherenceSummary>,
    /// Present only when `--auto-apply` attempted an automatic application.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_apply: Option<AutoApplySummary>,
}

/// Local execution policy and delivery lineage, never part of v1 sync envelopes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalExecution {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planning: Option<crate::planning::Planning>,
    pub max_invocations: u32,
    pub deadline_at: DateTime<Utc>,
    pub no_retry: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_harness: Option<String>,
    pub fixed_model: Option<String>,
    pub fixed_effort: Option<String>,
    pub priority: i32,
    pub owner_uid: u32,
    #[serde(default)]
    pub supervisor: Option<crate::admission::ProcessIdentity>,
    pub final_attempt_id: Option<String>,
    pub contributing_attempts: Vec<String>,
    pub provenance: String,
    pub failure: Option<FailureKind>,
    pub questions: Vec<Clarification>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AttemptDetail {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consumed_artifacts: Vec<String>,
    #[serde(default)]
    pub usage_categories: std::collections::BTreeMap<String, u64>,
    pub parent_attempt_id: Option<String>,
    pub reason: Option<String>,
    pub input_baseline: Option<PathBuf>,
    pub decision: Option<AllocationDecision>,
    pub admission: Option<AdmissionSummary>,
    pub capacity: Option<CapacityObservation>,
    pub result: Option<CandidateRecord>,
    pub failure: Option<FailureKind>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    TargetVerification,
    VerificationInfrastructure,
    VerificationUnknown,
    HarnessProcess,
    BaselineInfrastructure,
    Cancelled,
    Deadline,
    CapacityAdmission,
    Authorization,
    UnsupportedCheckpoint,
    InternalState,
    SourceDrift,
    InvocationLimit,
    /// The mid-run coherence watcher stopped the agent because the source
    /// moved underneath its work.
    StaleWork,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CheckpointReport {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub version: u32,
    pub question: String,
    #[serde(default)]
    pub choices: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuestionState {
    Pending,
    Answered,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Clarification {
    pub id: String,
    pub run_id: String,
    pub attempt_id: String,
    pub generation: u32,
    pub revision: u64,
    pub report: CheckpointReport,
    pub state: QuestionState,
    pub answer: Option<String>,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub actor_uid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
}
