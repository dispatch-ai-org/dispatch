use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    BugFix,
    Feature,
    Refactor,
    Tests,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskScope {
    Localized,
    MultiFile,
    Broad,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
    Failed,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
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
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub environment: EnvironmentRecord,
    #[serde(default)]
    pub baseline_checks: Vec<CheckResult>,
    #[serde(default)]
    pub candidates: Vec<CandidateRecord>,
    #[serde(default)]
    pub routing: Option<RoutingDecision>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationRecord {
    pub outcome: EvaluationOutcome,
    #[serde(default)]
    pub reasons: Vec<String>,
    pub explanation: Option<String>,
    pub created_at: DateTime<Utc>,
    pub blind: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRecord {
    pub run_id: String,
    pub candidate_label: Option<String>,
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
    pub payload: serde_json::Value,
}
