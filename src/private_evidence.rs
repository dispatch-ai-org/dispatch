//! Local descriptive evidence. No provider calls, labels inferred from prose, or scores.
//! Existing run/attempt/feedback rows remain authoritative; annotations attest provenance.
use crate::{
    AllocationDecision, Config, FailureKind, ResourceChoice, ResourceTier, RunRecord, TaskFeatures,
    TaskKind, TaskScope, commands::digest, config::ResourceConfig, db::Database, state::State,
};
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, Write},
    path::Path,
};

pub const BASE: &str = "allocation-portfolio-v3";
pub const RULE: &str = "private-quality-trial-v1";
const MAX_GOALS: usize = 1000;
const WINDOW_DAYS: i64 = 90;
const MIN_REVIEWED: u64 = 20;
const MIN_QUALITY_REJECTIONS: u64 = 5;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EvidenceConfig {
    pub shadow: bool,
    /// Explicit owner-selected routine task/check mappings; no natural-language labeling.
    pub routine_mappings: Vec<Mapping>,
}
impl EvidenceConfig {
    pub fn is_disabled(&self) -> bool {
        !self.shadow && self.routine_mappings.is_empty()
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Mapping {
    pub id: String,
    pub features: TaskFeatures,
    pub verify: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextKey {
    pub source_key: String,
    pub features: TaskFeatures,
    pub verification: String,
    pub mapping: Option<String>,
    pub base_policy: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Behavior {
    pub key: String,
    pub executable_known: bool,
    pub provider: String,
    pub harness: String,
    pub requested_model: String,
    pub resolved_model: String,
    pub effort: Option<String>,
    pub runtime: String,
    pub service_mode: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DecisionEvidence {
    pub version: u32,
    pub context: ContextKey,
    pub behaviors: Vec<Behavior>,
    pub selected_behavior: String,
    pub cutoff: DateTime<Utc>,
    pub snapshot_hash: String,
    pub annotation_revision: u64,
    pub base_choice: ResourceChoice,
    pub proposed_choice: Option<ResourceChoice>,
    pub proposal_id: Option<String>,
    pub policy_revision: u64,
    pub mode: String,
    pub reason: String,
    pub counts: Option<Counts>,
    pub cohorts: Option<BTreeMap<String, Counts>>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Counts {
    pub goals: u64,
    pub attempts: u64,
    pub launched: u64,
    pub launch_unknown: u64,
    pub prelaunch: u64,
    pub policy_chains: u64,
    pub continuations: u64,
    pub first_attempt_checks: BTreeMap<String, u64>,
    pub recovery_triggers: BTreeMap<String, u64>,
    pub eligible_executions: u64,
    pub reviewed: u64,
    pub accepted: u64,
    pub rejected: u64,
    pub quality_rejected: u64,
    pub accepted_verified: u64,
    pub accepted_unverified: u64,
    pub accepted_apply_blocked: u64,
    pub missing_review: u64,
    pub deferred: u64,
    pub verification: BTreeMap<String, u64>,
    pub failures: BTreeMap<String, u64>,
    pub goal_failures: BTreeMap<String, u64>,
    pub exclusions: BTreeMap<String, u64>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Distribution {
    pub count: u64,
    pub total: u64,
    pub min: Option<u64>,
    pub max: Option<u64>,
}
impl Distribution {
    fn add(&mut self, value: u64) {
        self.count += 1;
        self.total = self.total.saturating_add(value);
        self.min = Some(self.min.map_or(value, |v| v.min(value)));
        self.max = Some(self.max.map_or(value, |v| v.max(value)));
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Economics {
    pub harness_ms: Distribution,
    pub verification_ms: Distribution,
    pub end_to_end_ms: Distribution,
    pub admission_wait_ms: Distribution,
    pub reported_repair_minutes: Distribution,
    /// Keys include provider, harness and the original token semantics.
    pub usage: BTreeMap<String, Distribution>,
    pub allowance_attribution: String,
    pub cash_cost: Option<u64>,
    pub harness_ms_per_accepted: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub version: u32,
    pub cutoff: DateTime<Utc>,
    pub since: DateTime<Utc>,
    pub annotation_revision: u64,
    pub context: ContextKey,
    pub truncated: bool,
    pub counts: Counts,
    pub origins: BTreeMap<String, Counts>,
    pub submissions: BTreeMap<String, u64>,
    pub review_provenance: BTreeMap<String, u64>,
    pub execution_contexts: BTreeMap<String, u64>,
    pub resources: BTreeMap<String, Counts>,
    pub economics: BTreeMap<String, Economics>,
    pub references: Vec<Reference>,
    pub snapshot_hash: String,
    pub limitations: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Reference {
    pub run_id: String,
    pub cohort: String,
    pub feedback_id: String,
    pub annotation_sequence: u64,
    pub delivery: String,
    pub facts: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Annotation {
    pub origin: String,
    pub review: String,
    pub delivery: String,
    pub actor: String,
    pub repair_minutes: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub version: u32,
    pub rule: String,
    pub context: ContextKey,
    pub cutoff: DateTime<Utc>,
    pub source_behavior: String,
    pub target_behavior: String,
    pub target: ResourceChoice,
    pub counts: Counts,
    pub references: Vec<Reference>,
    pub snapshot_hash: String,
    pub parameters: Value,
    pub limitations: Vec<String>,
    pub fallback: String,
}
fn bump(map: &mut BTreeMap<String, u64>, key: impl Into<String>) {
    *map.entry(key.into()).or_default() += 1;
}
fn word(value: &impl Serialize) -> String {
    serde_json::to_value(value)
        .unwrap_or(Value::Null)
        .as_str()
        .unwrap_or("unknown")
        .into()
}
pub fn source_key(source: &Path) -> Result<String> {
    digest(&source.canonicalize()?)
}
fn behavior(choice: &ResourceChoice, config: &Config) -> Result<Behavior> {
    let harness = config.harnesses.get(&choice.harness);
    let executable = harness
        .executable
        .clone()
        .unwrap_or_else(|| choice.harness.clone().into());
    let executable = which::which(&executable).unwrap_or(executable);
    // No invocation or credential access. Version is additionally retained in outcome groups.
    let executable_hash = (|| -> Result<String> {
        use sha2::{Digest, Sha256};
        use std::io::Read;
        let mut file = std::fs::File::open(executable)?;
        ensure!(file.metadata()?.is_file(), "not a regular executable");
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 65536];
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
        }
        Ok(hex::encode(hash.finalize()))
    })()
    .ok();
    let mut value = Behavior {
        key: String::new(),
        executable_known: executable_hash.is_some(),
        provider: choice.provider.clone(),
        harness: choice.harness.clone(),
        requested_model: choice.requested_model.clone(),
        resolved_model: choice.resolved_model.clone(),
        effort: choice.effort.clone(),
        runtime: choice.runtime.clone(),
        service_mode: choice.service_mode.clone(),
    };
    value.key = digest(
        &json!({"behavior":value,"executable":executable_hash,"harness":harness,
        "execution":config.execution,"adapter_contract":"phase6-title-disabled-v1"}),
    )?;
    Ok(value)
}
fn context(source: &Path, features: &TaskFeatures, config: &Config) -> Result<ContextKey> {
    let matches: Vec<_> = config
        .private_evidence
        .routine_mappings
        .iter()
        .filter(|m| {
            m.features == *features
                && m.verify == config.checks.verify
                && !m.verify.is_empty()
                && !m.id.is_empty()
                && features.language.is_some()
                && features.task_kind != TaskKind::Unknown
                && matches!(features.scope, TaskScope::Localized | TaskScope::MultiFile)
        })
        .collect();
    Ok(ContextKey {
        source_key: source_key(source)?,
        features: features.clone(),
        verification: digest(&config.checks)?,
        mapping: if matches.len() == 1 {
            Some(digest(matches[0])?)
        } else {
            None
        },
        base_policy: BASE.into(),
    })
}
fn fixture_domain(c: &Connection) -> Result<bool> {
    Ok(c.query_row(
        "SELECT EXISTS(SELECT 1 FROM private_fixture_domain)",
        [],
        |r| r.get(0),
    )?)
}
fn latest_annotation(
    c: &Connection,
    id: &str,
    cutoff: DateTime<Utc>,
) -> Result<Option<(u64, Option<String>, Annotation)>> {
    let row: Option<(u64,Option<String>,String)> = c.query_row(
        "SELECT sequence,feedback_id,payload_json FROM private_annotations WHERE run_id=?1 AND created_at<=?2 ORDER BY sequence DESC LIMIT 1",
        params![id,cutoff.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    row.map(|(seq, f, s)| Ok((seq, f, serde_json::from_str(&s)?)))
        .transpose()
}
pub fn delivery(run: &RunRecord) -> Result<String> {
    digest(
        &json!({"final":run.phase3.as_ref().and_then(|p|p.final_attempt_id.as_ref()),
        "contributors":run.phase3.as_ref().map(|p|&p.contributing_attempts),
        "baseline":run.baseline_commit,"candidates":run.candidates.iter().map(|c| (&c.id,&c.diff_stats,&c.checks)).collect::<Vec<_>>() }),
    )
}
fn quality_reason(reasons: &[String]) -> bool {
    reasons
        .iter()
        .any(|r| matches!(r.as_str(), "correctness" | "completeness" | "rework"))
        && !reasons
            .iter()
            .any(|r| matches!(r.as_str(), "changed-requirements" | "source-drift"))
}
fn outcome_facts(run: &RunRecord) -> Result<String> {
    digest(&json!({"attempts":run.attempts,"allocation":run.allocation,
        "verification":run.outcome.verification,"baseline_checks":run.baseline_checks,
        "phase3":run.phase3,"source":run.source_path}))
}
fn feedback(
    c: &Connection,
    run: &str,
    cutoff: DateTime<Utc>,
) -> Result<Option<(String, String, Vec<String>)>> {
    let row: Option<(String,String,String)> = c.query_row(
        "SELECT id,outcome,reasons_json FROM goal_feedback_revisions WHERE run_id=?1 AND created_at<=?2 ORDER BY revision DESC LIMIT 1",
        params![run,cutoff.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    row.map(|(id, o, s)| Ok((id, o, serde_json::from_str(&s)?)))
        .transpose()
}
/// Caller holds one read transaction, also used by proposal generation and active-policy checks.
fn summarize(
    c: &Connection,
    source: &Path,
    ctx: &ContextKey,
    cutoff: DateTime<Utc>,
) -> Result<Summary> {
    let since = cutoff - Duration::days(WINDOW_DAYS);
    let mut out = Summary { version:1, cutoff,since,context:ctx.clone(),truncated:false,annotation_revision: c.query_row(
        "SELECT COALESCE(MAX(sequence),0) FROM private_annotations WHERE created_at<=?1",[cutoff.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)],|r|r.get(0))?,
        submissions:BTreeMap::new(),review_provenance:BTreeMap::new(),execution_contexts:BTreeMap::new(),counts:Counts::default(),origins:BTreeMap::new(),resources:BTreeMap::new(),economics:BTreeMap::new(),references:vec![],snapshot_hash:String::new(),
        limitations:vec!["Descriptive selected populations; no causal comparison or success probability.".into(),
        "Observed Codex model/effort may be unknown; profile outcomes are not confirmed underlying-model measurements.".into(),
        "No cross-project pooling. Quality cohort requires single-attempt delivery and explicit review provenance.".into()] };
    // Source filter is relational and indexed; never scan transcripts or unrelated project projections.
    let domain = fixture_domain(c)?;
    let mut q=c.prepare("SELECT run_projection_json FROM runs WHERE source_id IN (SELECT id FROM sources WHERE path=?1) AND created_at>=?2 AND created_at<=?3 ORDER BY created_at DESC,id DESC LIMIT ?4")?;
    let rows = q.query_map(
        params![
            source.canonicalize()?.to_string_lossy(),
            since.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
            cutoff.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
            MAX_GOALS + 1
        ],
        |r| r.get::<_, Option<String>>(0),
    )?;
    for (index, row) in rows.enumerate() {
        if index == MAX_GOALS {
            out.truncated = true;
            break;
        }
        let Some(raw) = row? else {
            bump(&mut out.counts.exclusions, "missing_projection");
            continue;
        };
        let run: RunRecord = serde_json::from_str(&raw)?;
        let annotation = latest_annotation(c, &run.id, cutoff)?;
        let machine: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM control_runs WHERE run_id=?1)",
            [&run.id],
            |r| r.get(0),
        )?;
        bump(
            &mut out.submissions,
            if machine {
                "scoped_machine"
            } else {
                "local_or_unknown"
            },
        );
        bump(
            &mut out.execution_contexts,
            format!("{}:{}", word(&run.mode), run.environment.execution_backend),
        );
        bump(
            &mut out.review_provenance,
            annotation
                .as_ref()
                .map(|a| a.2.review.as_str())
                .unwrap_or("unknown"),
        );
        let origin = annotation
            .as_ref()
            .map(|a| a.2.origin.as_str())
            .unwrap_or("unknown");
        let origin_counts = out.origins.entry(origin.into()).or_default();
        origin_counts.goals += 1;
        out.counts.goals += 1;
        let economics = out.economics.entry(origin.into()).or_default();
        let planned = crate::planning::planning(&run).is_some();
        if planned {
            // Committed check events include baseline, child-input comparisons and
            // root integration checks, without counting the final projection twice.
            let mut checks = c.prepare("SELECT payload_json FROM events WHERE run_id=?1 AND event_type='check.finished' AND timestamp<=?2 ORDER BY sequence")?;
            for row in checks.query_map(
                params![
                    run.id,
                    cutoff.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
                ],
                |r| r.get::<_, String>(0),
            )? {
                let value: Value = serde_json::from_str(&row?)?;
                if let Some(ms) = value.get("duration_ms").and_then(Value::as_u64) {
                    economics.verification_ms.add(ms);
                }
            }
        }
        economics.allowance_attribution =
            "unknown_or_mixed_pool_activity; provider units are not interchangeable".into();
        if let Some(completed) = run.completed_at {
            economics
                .end_to_end_ms
                .add((completed - run.created_at).num_milliseconds().max(0) as u64);
        }

        bump(
            &mut out.counts.verification,
            word(&run.outcome.verification),
        );
        bump(
            &mut origin_counts.verification,
            word(&run.outcome.verification),
        );
        let mut operational = false;
        if let Some(f) = run.phase3.as_ref().and_then(|p| p.failure) {
            bump(&mut out.counts.goal_failures, word(&f));
            bump(&mut origin_counts.goal_failures, word(&f));
        }
        let before_launch = (
            out.counts.launched,
            out.counts.launch_unknown,
            out.counts.prelaunch,
        );
        for attempt in &run.attempts {
            out.counts.attempts += 1;
            origin_counts.attempts += 1;
            let result = attempt.detail.result.as_ref();
            // Historical successful/terminated processes establish launch; a prepared attempt alone does not.
            let launch: Option<String> = c.query_row("SELECT launch_knowledge FROM admission_requests WHERE attempt_id=?1 ORDER BY enqueued_at DESC LIMIT 1",[&attempt.id],|r|r.get(0)).optional()?;
            let launched = matches!(
                launch.as_deref(),
                Some("child_recorded" | "cleanup_confirmed")
            ) || result.is_some_and(|r| r.exit_code.is_some() || r.timed_out);
            let unlaunched = matches!(
                launch.as_deref(),
                Some("launch_intent_committed" | "launch_not_started")
            );
            let waiting: Option<(String,String)> = c.query_row("SELECT enqueued_at,admitted_at FROM admission_requests WHERE attempt_id=?1 AND admitted_at IS NOT NULL ORDER BY enqueued_at DESC LIMIT 1",[&attempt.id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
            if let Some((start, end)) = waiting {
                economics.admission_wait_ms.add(
                    (DateTime::parse_from_rfc3339(&end)? - DateTime::parse_from_rfc3339(&start)?)
                        .num_milliseconds()
                        .max(0) as u64,
                );
            }
            if launched {
                out.counts.launched += 1;
                origin_counts.launched += 1;
            } else if unlaunched {
                out.counts.prelaunch += 1;
                origin_counts.prelaunch += 1;
            } else {
                out.counts.launch_unknown += 1;
                origin_counts.launch_unknown += 1;
            }
            if let Some(result) = result {
                if launched || result.duration_ms > 0 {
                    economics.harness_ms.add(result.duration_ms);
                }
                if !planned {
                    for check in &result.checks {
                        economics.verification_ms.add(check.duration_ms);
                    }
                }
                for (category, value) in &attempt.detail.usage_categories {
                    economics
                        .usage
                        .entry(format!(
                            "{}/{}/terminal_category:{category}",
                            attempt
                                .resource
                                .as_ref()
                                .map(|r| r.provider.as_str())
                                .unwrap_or("unknown"),
                            attempt.harness_id
                        ))
                        .or_default()
                        .add(*value);
                }
                if let Some(tokens) = result.tokens {
                    let provider = attempt
                        .resource
                        .as_ref()
                        .map(|r| r.provider.as_str())
                        .unwrap_or("unknown");
                    economics
                        .usage
                        .entry(format!(
                            "{provider}/{}:{}",
                            attempt.harness_id,
                            result.token_semantics.as_deref().unwrap_or("unknown")
                        ))
                        .or_default()
                        .add(tokens);
                }
            }
            if let Some(f) = attempt.detail.failure {
                bump(&mut out.counts.failures, word(&f));
                bump(&mut origin_counts.failures, word(&f));
                operational |= f != FailureKind::TargetVerification;
            }
        }
        if run.attempts.is_empty() {
            out.counts.prelaunch += 1;
            origin_counts.prelaunch += 1;
        }
        let first_checks = run
            .attempts
            .first()
            .and_then(|a| a.detail.result.as_ref())
            .map(|r| {
                if r.checks.is_empty() {
                    "not_run_or_not_configured"
                } else if r
                    .checks
                    .iter()
                    .all(|c| c.status == crate::CheckStatus::Passed)
                {
                    "passed"
                } else if r
                    .checks
                    .iter()
                    .any(|c| c.status == crate::CheckStatus::Failed)
                {
                    "failed"
                } else {
                    "inconclusive"
                }
            })
            .unwrap_or("not_run");
        bump(&mut out.counts.first_attempt_checks, first_checks);
        bump(&mut origin_counts.first_attempt_checks, first_checks);
        for attempt in run.attempts.iter().skip(1) {
            // Initial children are part of the planned allocation policy, not retries.
            if attempt.role == "task" {
                continue;
            }
            out.counts.continuations += 1;
            origin_counts.continuations += 1;
            let trigger = if attempt.detail.reason.as_deref() == Some("target_verification_failure")
            {
                "target_verification_failure"
            } else {
                "clarification_or_other_continuation"
            };
            bump(&mut out.counts.recovery_triggers, trigger);
            bump(&mut origin_counts.recovery_triggers, trigger);
        }
        let chain = run.attempts.len() != 1
            || run
                .phase3
                .as_ref()
                .is_some_and(|p| p.contributing_attempts.len() > 1 || !p.questions.is_empty())
            || run
                .attempts
                .iter()
                .any(|a| a.detail.parent_attempt_id.is_some() || a.role != "executor");
        if chain {
            out.counts.policy_chains += 1;
            origin_counts.policy_chains += 1;
        }
        let latest = feedback(c, &run.id, cutoff)?;
        let valid = match (&annotation, &latest) {
            (Some((_, Some(attested), a)), Some((id, _, _))) => {
                attested == id
                    && a.delivery == delivery(&run)?
                    && (a.review == "human" && a.origin == "ordinary"
                        || domain && a.review == "synthetic" && a.origin == "synthetic")
            }
            _ => false,
        };
        if !valid {
            out.counts.missing_review += 1;
            origin_counts.missing_review += 1;
        }
        if word(&run.outcome.review) == "deferred" {
            out.counts.deferred += 1;
            origin_counts.deferred += 1;
        }
        if valid {
            if let Some(minutes) = annotation.as_ref().and_then(|a| a.2.repair_minutes) {
                economics.reported_repair_minutes.add(minutes);
            }
            let (_, outcome, reasons) = latest.as_ref().unwrap();
            out.counts.reviewed += 1;
            origin_counts.reviewed += 1;
            if outcome == "accepted" {
                out.counts.accepted += 1;
                origin_counts.accepted += 1;
                if run.outcome.verification == crate::VerificationState::Passed {
                    out.counts.accepted_verified += 1;
                    origin_counts.accepted_verified += 1;
                } else {
                    out.counts.accepted_unverified += 1;
                    origin_counts.accepted_unverified += 1;
                }
                if run.outcome.application == crate::ApplicationState::BlockedBySourceDrift {
                    out.counts.accepted_apply_blocked += 1;
                    origin_counts.accepted_apply_blocked += 1;
                }
            } else {
                out.counts.rejected += 1;
                origin_counts.rejected += 1;
                if quality_reason(reasons) {
                    out.counts.quality_rejected += 1;
                    origin_counts.quality_rejected += 1;
                }
            }
        }
        let mut excluded = Vec::new();
        if !(origin == "ordinary" && !domain || origin == "synthetic" && domain) {
            excluded.push(format!("origin_{origin}"));
        }
        if run
            .attempts
            .first()
            .is_none_or(|a| a.completed_at.is_none() || a.detail.result.is_none())
        {
            excluded.push("unfinished_execution".into());
        }
        if chain {
            excluded.push("combined_or_continued_delivery".into());
        }
        if operational {
            excluded.push("operational_failure".into());
        }
        let decision = run
            .allocation
            .as_ref()
            .and_then(|a| a.private_evidence.as_ref());
        if decision.is_none_or(|d| d.context != *ctx) {
            excluded.push("incompatible_or_unknown_task_check_context".into());
        }
        if ctx.mapping.is_none() {
            excluded.push("no_explicit_routine_check_mapping".into());
        }
        let attempt = run.attempts.first();
        if attempt.is_none_or(|a| a.harness_version.is_none() || a.resolved_model.is_none()) {
            excluded.push("incomplete_resource_attribution".into());
        }
        if attempt.is_some_and(|a| {
            a.observed_model
                .as_ref()
                .is_some_and(|m| Some(m) != a.resolved_model.as_ref())
                || a.observed_effort
                    .as_ref()
                    .is_some_and(|e| Some(e) != a.resolved_effort.as_ref())
        }) {
            excluded.push("observed_substitution".into());
        }
        if !excluded.is_empty() {
            for reason in excluded {
                bump(&mut origin_counts.exclusions, reason.clone());
                bump(&mut out.counts.exclusions, reason);
            }
            continue;
        }
        let d = decision.unwrap();
        let a = attempt.unwrap();
        // Distinct harness versions and observed-identity knowledge remain distinct cohorts.
        let key = format!(
            "{}|{}|{}|{}|{}",
            d.selected_behavior,
            a.harness_version.as_deref().unwrap(),
            a.observed_model.as_deref().unwrap_or("unknown"),
            a.observed_effort.as_deref().unwrap_or("unknown"),
            run.allocation.as_ref().unwrap().policy_version
        );
        let cohort = out.resources.entry(key.clone()).or_default();
        cohort.goals += 1;
        cohort.attempts += 1;
        cohort.launched += out.counts.launched - before_launch.0;
        cohort.launch_unknown += out.counts.launch_unknown - before_launch.1;
        cohort.prelaunch += out.counts.prelaunch - before_launch.2;
        cohort.deferred += u64::from(run.outcome.review == crate::ReviewState::Deferred);
        bump(&mut cohort.first_attempt_checks, first_checks);
        if let Some(f) = a.detail.failure {
            bump(&mut cohort.failures, word(&f));
        }
        if let Some(f) = run.phase3.as_ref().and_then(|p| p.failure) {
            bump(&mut cohort.goal_failures, word(&f));
        }
        origin_counts.eligible_executions += 1;
        cohort.eligible_executions += 1;
        out.counts.eligible_executions += 1;
        bump(&mut cohort.verification, word(&run.outcome.verification));
        out.references.push(Reference {
            run_id: run.id.clone(),
            cohort: key.clone(),
            feedback_id: latest.as_ref().map(|f| f.0.clone()).unwrap_or_default(),
            annotation_sequence: annotation.as_ref().map(|a| a.0).unwrap_or(0),
            delivery: delivery(&run)?,
            facts: outcome_facts(&run)?,
        });
        if !valid {
            cohort.missing_review += 1;
            continue;
        }
        let (_, outcome, reasons) = latest.unwrap();
        cohort.reviewed += 1;
        if outcome == "accepted" {
            cohort.accepted += 1;
            if word(&run.outcome.verification) == "passed" {
                cohort.accepted_verified += 1;
            } else {
                cohort.accepted_unverified += 1;
            }
            if word(&run.outcome.application) == "blocked_by_source_drift" {
                cohort.accepted_apply_blocked += 1;
            }
        } else {
            cohort.rejected += 1;
            if quality_reason(&reasons) {
                cohort.quality_rejected += 1;
            }
        }
    }
    for (origin, e) in &mut out.economics {
        let accepted = out.origins.get(origin).map(|c| c.accepted).unwrap_or(0);
        e.harness_ms_per_accepted = (accepted > 0
            && e.harness_ms.count == out.origins[origin].launched
            && out.origins[origin].launch_unknown == 0)
            .then(|| e.harness_ms.total / accepted);
    }
    out.snapshot_hash = digest(&out)?;
    Ok(out)
}
fn rule_parameters() -> Value {
    json!({"window_days":WINDOW_DAYS,"minimum_reviewed":MIN_REVIEWED,
        "minimum_review_coverage_percent":80,"minimum_quality_rejections":MIN_QUALITY_REJECTIONS,
        "minimum_quality_rejection_percent":25,"minimum_alternative_verified":1,"maximum_window_goals":MAX_GOALS})
}
fn proposal(
    summary: &Summary,
    decision: &AllocationDecision,
    behaviors: &[Behavior],
) -> Result<(Option<Proposal>, String)> {
    if summary.truncated || summary.context.mapping.is_none() {
        return Ok((None, "incomplete_window_or_unmapped_task".into()));
    }
    // Explicit multi-file mappings support descriptive evidence, not rule-v1 trials.
    if summary.context.features.scope != TaskScope::Localized {
        return Ok((None, "task_scope_outside_trial_rule".into()));
    }
    if decision.selected.tier != ResourceTier::Light {
        return Ok((None, "base_choice_is_not_light".into()));
    }
    let base = behaviors
        .iter()
        .find(|b| {
            b.resolved_model == decision.selected.resolved_model
                && b.harness == decision.selected.harness
                && b.effort == decision.selected.effort
        })
        .context("missing selected behavior")?;
    if !base.executable_known || base.runtime != "local" {
        return Ok((None, "incomplete_runtime_identity".into()));
    }
    let cohorts: Vec<_> = summary
        .resources
        .iter()
        .filter(|(k, _)| k.starts_with(&format!("{}|", base.key)))
        .collect();
    if cohorts.len() != 1 {
        return Ok((None, "insufficient_or_conflicting_resource_cohorts".into()));
    }
    let (cohort_key, counts) = cohorts[0];
    if counts.reviewed < MIN_REVIEWED || counts.reviewed * 100 < counts.goals * 80 {
        return Ok((
            None,
            "insufficient_reviewed_goals_or_review_coverage".into(),
        ));
    }
    if counts.quality_rejected < MIN_QUALITY_REJECTIONS
        || counts.quality_rejected * 100 < counts.reviewed * 25
    {
        return Ok((None, "quality_rejection_trigger_not_met".into()));
    }
    for (alt, b) in decision.alternatives.iter().zip(behaviors) {
        if !alt.eligible || !b.executable_known || alt.choice.tier == ResourceTier::Light {
            continue;
        }
        let supported: Vec<_> = summary
            .resources
            .iter()
            .filter(|(k, _)| k.starts_with(&format!("{}|", b.key)))
            .collect();
        if supported.len() != 1
            || supported[0]
                .1
                .verification
                .get("passed")
                .copied()
                .unwrap_or(0)
                == 0
        {
            continue;
        }
        let references = summary
            .references
            .iter()
            .filter(|r| r.cohort == *cohort_key || r.cohort == *supported[0].0)
            .cloned()
            .collect();
        let p=Proposal{version:1,rule:RULE.into(),context:summary.context.clone(),cutoff:summary.cutoff,
            source_behavior:base.key.clone(),target_behavior:b.key.clone(),target:alt.choice.clone(),counts:counts.clone(),references,
            snapshot_hash:summary.snapshot_hash.clone(),parameters:rule_parameters(),
            limitations:vec!["Trial proposal; alternative superiority is unproven. No numerical uplift.".into(),
                "Only explicitly classified correctness/completeness/rework rejections trigger this rule.".into(),
                "Unknown observed identity supports resolved-profile trials only, never pure-model claims.".into()],
            fallback:"Use unchanged deterministic selection among currently suitable authorized choices; otherwise existing defer/block.".into()};
        return Ok((Some(p), "repeated_quality_rejections_trial_only".into()));
    }
    Ok((None, "no_compatible_verified_alternative_support".into()))
}
fn active(c: &Connection, key: &str) -> Result<(u64, Option<String>)> {
    Ok(c.query_row("SELECT revision,proposal_id FROM private_policy_transitions WHERE source_key=?1 ORDER BY revision DESC LIMIT 1",[key],|r|Ok((r.get(0)?,r.get(1)?))).optional()?.unwrap_or((0,None)))
}
pub fn policy_revision(state: &State, source: &Path) -> Result<u64> {
    let db = Database::open(state.db_path())?;
    Ok(active(db.connection(), &source_key(source)?)?.0)
}
fn load_proposal(c: &Connection, id: &str) -> Result<(Proposal, String)> {
    let (raw, status): (String, String) = c.query_row(
        "SELECT payload_json,status FROM private_proposals WHERE id=?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let p: Proposal = serde_json::from_str(&raw)?;
    ensure!(
        digest(&p)? == id
            && p.version == 1
            && p.rule == RULE
            && p.context.base_policy == BASE
            && p.parameters == rule_parameters(),
        "invalid proposal content/version"
    );
    Ok((p, status))
}
fn valid_references(c: &Connection, p: &Proposal, now: DateTime<Utc>) -> Result<bool> {
    if p.cutoff > now || p.cutoff < now - Duration::days(WINDOW_DAYS) {
        return Ok(false);
    }
    for r in &p.references {
        let raw: Option<String> = c
            .query_row(
                "SELECT run_projection_json FROM runs WHERE id=?1",
                [&r.run_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        let Some(raw) = raw else { return Ok(false) };
        let run: RunRecord = serde_json::from_str(&raw)?;
        let a = latest_annotation(c, &r.run_id, now)?;
        let f = feedback(c, &r.run_id, now)?;
        if a.as_ref().map(|a| a.0) != Some(r.annotation_sequence)
            || f.as_ref().map(|f| f.0.as_str()).unwrap_or("") != r.feedback_id
            || delivery(&run)? != r.delivery
            || outcome_facts(&run)? != r.facts
            || run.created_at < now - Duration::days(WINDOW_DAYS)
            || run
                .allocation
                .as_ref()
                .and_then(|d| d.private_evidence.as_ref())
                .is_none_or(|d| d.context != p.context)
        {
            return Ok(false);
        }
    }
    Ok(!p.references.is_empty())
}
fn store_proposal(c: &Connection, p: &Proposal) -> Result<String> {
    let id = digest(p)?;
    c.execute("INSERT OR IGNORE INTO private_proposals(id,source_key,payload_json,status) VALUES (?1,?2,?3,'proposed')",params![id,p.context.source_key,serde_json::to_string(p)?])?;
    Ok(id)
}
/// One analytics read snapshot after the existing preflight/capacity filtering. No admission held.
pub(crate) fn select(
    state: &State,
    source: &Path,
    config: &Config,
    mut decision: AllocationDecision,
    explicit: bool,
) -> Result<AllocationDecision> {
    let db = Database::open(state.db_path())?;
    let c = db.connection();
    let tx = c.unchecked_transaction()?;
    let now = Utc::now();
    let ctx = context(source, &decision.task_features, config)?;
    let (revision, active_id) = active(&tx, &ctx.source_key)?;
    ensure!(
        crate::commands::private_policy_revision().is_none_or(|bound| bound == revision),
        "authorization_required: private policy changed at selection; issue a new grant"
    );
    let behaviors = decision
        .alternatives
        .iter()
        .map(|a| behavior(&a.choice, config))
        .collect::<Result<Vec<_>>>()?;
    let summary = summarize(&tx, source, &ctx, now);
    let base_decision = decision.clone();
    let base_choice = decision.selected.clone();
    let mut reason = if summary.is_err() {
        "analytics_unavailable_base_fallback"
    } else {
        "observation_only"
    }
    .to_owned();
    let mut mode = if config.private_evidence.shadow {
        "shadow"
    } else {
        "default"
    }
    .to_owned();
    let mut stale = None;
    if let Some(id) = active_id {
        let (p, status) = load_proposal(&tx, &id)?;
        if status == "active" && !valid_references(&tx, &p, now)? {
            stale = Some(id);
            reason = "active_evidence_stale_base_fallback".into();
        } else if status == "active"
            && p.context == ctx
            && summary.is_ok()
            && !explicit
            && behavior(&decision.selected, config)?.key == p.source_behavior
        {
            if let Some((alt, _)) = decision.alternatives.iter().zip(&behaviors).find(|(a, b)| {
                a.eligible && b.key == p.target_behavior && a.choice.tier != ResourceTier::Light
            }) {
                decision.selected = alt.choice.clone();
                decision.reason =
                    "Using an explicitly approved private-evidence trial preference.".into();
                mode = "promoted".into();
            } else {
                reason = "approved_target_unavailable_base_fallback".into();
            }
        }
    }
    let (proposed, screen_reason) = match &summary {
        Ok(s) if config.private_evidence.shadow => proposal(s, &base_decision, &behaviors)?,
        _ => (
            None,
            if summary.is_err() {
                "analytics_unavailable_base_fallback".into()
            } else {
                reason.clone()
            },
        ),
    };
    if mode != "promoted" && reason == "observation_only" {
        reason = screen_reason;
    }
    let selected_behavior = behavior(&decision.selected, config)?.key;
    let meta = DecisionEvidence {
        version: 1,
        context: ctx,
        behaviors,
        selected_behavior,
        cutoff: now,
        snapshot_hash: summary
            .as_ref()
            .map(|s| s.snapshot_hash.clone())
            .unwrap_or_default(),
        annotation_revision: summary.as_ref().map(|s| s.annotation_revision).unwrap_or(0),
        base_choice,
        proposed_choice: proposed.as_ref().map(|p| p.target.clone()),
        proposal_id: proposed.as_ref().map(digest).transpose()?,
        policy_revision: revision,
        mode,
        reason,
        cohorts: summary.as_ref().map(|s| s.resources.clone()).ok(),
        counts: summary.as_ref().map(|s| s.counts.clone()).ok(),
    };
    tx.commit()?;
    if let Some(id) = stale {
        c.execute(
            "UPDATE private_proposals SET status='stale' WHERE id=?1 AND status='active'",
            [id],
        )?;
    }
    if let Some(p) = proposed {
        store_proposal(c, &p)?;
    }
    if revision > 0 {
        decision.policy_version = format!("{BASE}/private-{revision}");
    }
    decision.private_evidence = Some(meta);
    Ok(decision)
}

pub fn inspect(state: &State, run_id: &str) -> Result<Value> {
    crate::commands::ensure_machine_review_denied()?;
    let db = Database::open(state.db_path())?;
    let tx = db.connection().unchecked_transaction()?;
    let run = db
        .committed_run_projection(run_id)?
        .context("unknown committed run")?;
    let decision = run.allocation.as_ref().context("not an allocation goal")?;
    let ctx = match &decision.private_evidence {
        Some(d) => d.context.clone(),
        None => ContextKey {
            source_key: source_key(&run.source_path)?,
            features: decision.task_features.clone(),
            verification: "unknown".into(),
            mapping: None,
            base_policy: decision.policy_version.clone(),
        },
    };
    let summary = summarize(&tx, &run.source_path, &ctx, Utc::now())?;
    tx.commit()?;
    Ok(
        json!({"domain":if fixture_domain(db.connection())? {"synthetic_fixture"}else{"ordinary_private"},"at_decision_time":decision.private_evidence,"current":summary}),
    )
}
pub fn propose(state: &State, run_id: &str) -> Result<Value> {
    crate::commands::ensure_machine_review_denied()?;
    let db = Database::open(state.db_path())?;
    let run = db
        .committed_run_projection(run_id)?
        .context("unknown committed run")?;
    let mut decision = run.allocation.context("allocation required")?;
    let old = decision
        .private_evidence
        .take()
        .context("no compatible pre-execution evidence context")?;
    let (config, _) = Config::discover(&run.source_path, None)?;
    ensure!(
        context(&run.source_path, &decision.task_features, &config)? == old.context,
        "task/check configuration changed"
    );
    let resources = ResourceConfig::load(&state.root)?;
    for alt in &mut decision.alternatives {
        alt.eligible &= resources.profiles.iter().any(|p| {
            p.model == alt.choice.resolved_model
                && p.harness == alt.choice.harness
                && p.effort == alt.choice.effort
                && p.eligibility().is_ok()
        });
    }
    decision.selected = old.base_choice;
    let behaviors = decision
        .alternatives
        .iter()
        .map(|a| behavior(&a.choice, &config))
        .collect::<Result<Vec<_>>>()?;
    let tx = db.connection().unchecked_transaction()?;
    let summary = summarize(&tx, &run.source_path, &old.context, Utc::now())?;
    let (p, reason) = proposal(&summary, &decision, &behaviors)?;
    tx.commit()?;
    let id = p
        .as_ref()
        .map(|p| store_proposal(db.connection(), p))
        .transpose()?;
    Ok(json!({"proposal_id":id,"proposal":p,"reason":reason,"counts":summary.counts}))
}
/// Explicit attestation, not a TTY heuristic. Same-UID arbitrary DB writers remain trusted owners.
fn owner_authority(state: &State) -> Result<String> {
    crate::commands::ensure_machine_review_denied()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            std::fs::metadata(&state.root)?.uid() == crate::orchestrator::phase3::local_uid(),
            "state owner required"
        );
    }
    Ok(format!(
        "local-owner:{}",
        crate::orchestrator::phase3::local_uid()
    ))
}
fn owner_attestation(state: &State, statement: &str) -> Result<String> {
    owner_authority(state)?;
    let mut tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .context(
            "owner attestation requires the controlling terminal; machine grants cannot attest",
        )?;
    let challenge = ulid::Ulid::new().to_string();
    writeln!(
        tty,
        "{statement}\nType {challenge} to attest this exact statement:"
    )?;
    tty.flush()?;
    let mut answer = String::new();
    std::io::BufReader::new(&tty).read_line(&mut answer)?;
    ensure!(answer.trim() == challenge, "owner attestation declined");
    owner_authority(state)
}
pub fn annotate(
    state: &State,
    id: &str,
    origin: &str,
    review: &str,
    repair: Option<u64>,
) -> Result<Value> {
    let request = prepare_annotation(state, id, origin, review, repair)?;
    owner_attestation(
        state,
        &format!(
            "Annotate goal {id}, delivery {}, feedback {:?}: origin={origin}, review={review}, repair_minutes={repair:?}. Human means an actual person's judgment, never scripted approval. This creates no outcome label.",
            request.annotation.delivery, request.feedback_id
        ),
    )?;
    commit_annotation(state, &request)?;
    Ok(json!(request.annotation))
}

/// Fixed facts displayed before confirmation. Not a machine command or deserializable authority.
#[derive(Clone)]
pub(crate) struct AnnotationRequest {
    run: RunRecord,
    feedback_id: Option<String>,
    annotation_sequence: Option<u64>,
    annotation: Annotation,
}
fn prepare_annotation(
    state: &State,
    id: &str,
    origin: &str,
    review: &str,
    repair: Option<u64>,
) -> Result<AnnotationRequest> {
    let actor = owner_authority(state)?;
    ensure!(
        matches!(
            origin,
            "ordinary" | "synthetic" | "scripted_smoke" | "unknown"
        ) && matches!(
            review,
            "human" | "scripted" | "machine" | "unknown" | "synthetic"
        ),
        "invalid provenance annotation"
    );
    ensure!(
        repair.is_none_or(|m| m <= 10080),
        "repair effort exceeds one week"
    );
    let db = Database::open(state.db_path())?;
    let tx = db.connection().unchecked_transaction()?;
    let run: RunRecord = serde_json::from_str(&tx.query_row(
        "SELECT run_projection_json FROM runs WHERE id=?1",
        [id],
        |r| r.get::<_, String>(0),
    )?)?;
    let f = feedback(&tx, id, Utc::now())?;
    ensure!(
        review != "human"
            || origin == "ordinary"
                && f.as_ref().is_some_and(|(_, outcome, _)| {
                    matches!(
                        (outcome.as_str(), &run.outcome.review),
                        ("accepted", crate::ReviewState::Accepted)
                            | ("rejected", crate::ReviewState::Rejected)
                    )
                }),
        "human attestation requires ordinary work and an existing acceptance/rejection revision"
    );
    let old = latest_annotation(&tx, id, Utc::now())?;
    let annotation = Annotation {
        origin: origin.into(),
        review: review.into(),
        delivery: delivery(&run)?,
        actor,
        repair_minutes: repair,
    };
    validate_provenance(old.as_ref().map(|(_, _, a)| a), &annotation)?;
    Ok(AnnotationRequest {
        run,
        feedback_id: f.map(|f| f.0),
        annotation_sequence: old.map(|a| a.0),
        annotation,
    })
}
fn validate_provenance(old: Option<&Annotation>, new: &Annotation) -> Result<()> {
    ensure!(
        old.is_none_or(
            |old| !matches!(old.origin.as_str(), "synthetic" | "scripted_smoke")
                || new.origin == old.origin
        ),
        "experiment provenance cannot become ordinary work"
    );
    Ok(())
}
pub(crate) fn prepare_review_annotation(
    state: &State,
    displayed: &RunRecord,
) -> Result<AnnotationRequest> {
    let request = prepare_annotation(state, &displayed.id, "ordinary", "human", None)?;
    ensure!(
        request.run.state_revision == displayed.state_revision
            && request.annotation.delivery == delivery(displayed)?,
        "review changed; inspect the current result before attesting"
    );
    Ok(request)
}
/// Called only after explicit presenter confirmation or the advanced CLI's terminal challenge.
pub(crate) fn commit_annotation(state: &State, request: &AnnotationRequest) -> Result<()> {
    let actor = owner_authority(state)?;
    ensure!(
        actor == request.annotation.actor,
        "owner changed during attestation"
    );
    let db = Database::open(state.db_path())?;
    let tx = rusqlite::Transaction::new_unchecked(
        db.connection(),
        rusqlite::TransactionBehavior::Immediate,
    )?;
    let current: RunRecord = serde_json::from_str(&tx.query_row(
        "SELECT run_projection_json FROM runs WHERE id=?1",
        [&request.run.id],
        |r| r.get::<_, String>(0),
    )?)?;
    ensure!(
        current.state_revision == request.run.state_revision
            && delivery(&current)? == request.annotation.delivery
            && feedback(&tx, &current.id, Utc::now())?.map(|f| f.0) == request.feedback_id,
        "review changed during attestation; no annotation recorded"
    );
    let old = latest_annotation(&tx, &current.id, Utc::now())?;
    validate_provenance(old.as_ref().map(|(_, _, a)| a), &request.annotation)?;
    if old.as_ref().is_some_and(|(_, feedback, a)| {
        feedback == &request.feedback_id && a == &request.annotation
    }) {
        return Ok(()); // Idempotent confirmation: no new revision and no extra vote.
    }
    ensure!(
        old.map(|a| a.0) == request.annotation_sequence,
        "provenance changed during attestation; confirm again"
    );
    tx.execute("INSERT INTO private_annotations(run_id,feedback_id,payload_json,created_at) VALUES (?1,?2,?3,?4)",params![current.id,request.feedback_id,serde_json::to_string(&request.annotation)?,Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)])?;
    tx.commit()?;
    Ok(())
}
pub fn policy_inspect(state: &State, id: &str) -> Result<Value> {
    crate::commands::ensure_machine_review_denied()?;
    let db = Database::open(state.db_path())?;
    let (p, status) = load_proposal(db.connection(), id)?;
    Ok(
        json!({"id":id,"status":status,"currently_valid":valid_references(db.connection(),&p,Utc::now())?,"proposal":p,
        "active":active(db.connection(),&p.context.source_key)?}),
    )
}
pub fn transition(
    state: &State,
    source: &Path,
    proposal_id: Option<&str>,
    expected: u64,
) -> Result<Value> {
    let db = Database::open_control(state.db_path())?;
    let content = proposal_id
        .map(|id| policy_inspect(state, id))
        .transpose()?;
    let actor = owner_attestation(
        state,
        &format!(
            "Change future private policy for {} at revision {expected}. Proposal: {}. Rollback restores {BASE}; no historical goal or permission is rewritten.",
            source.display(),
            serde_json::to_string_pretty(&content)?
        ),
    )?;
    transition_on(
        db.connection(),
        source,
        proposal_id,
        expected,
        &actor,
        &ResourceConfig::load(&state.root)?,
        &Config::discover(source, None)?.0,
    )
}
fn router_base(
    resources: &ResourceConfig,
    features: &TaskFeatures,
    config: &Config,
) -> Result<AllocationDecision> {
    crate::router::select_resource(
        resources,
        features,
        &config.execution.backend,
        None,
        None,
        None,
        None,
    )
}
fn transition_on(
    c: &Connection,
    source: &Path,
    id: Option<&str>,
    expected: u64,
    actor: &str,
    resources: &ResourceConfig,
    config: &Config,
) -> Result<Value> {
    crate::commands::ensure_machine_review_denied()?;
    ensure!(
        actor.starts_with("local-owner:"),
        "explicit local owner required"
    );
    let key = source_key(source)?;
    let tx = rusqlite::Transaction::new_unchecked(c, rusqlite::TransactionBehavior::Immediate)?;
    let (revision, previous) = active(&tx, &key)?;
    ensure!(revision == expected, "stale policy revision");
    if let Some(id) = id {
        let (p, status) = load_proposal(&tx, id)?;
        ensure!(
            status == "proposed"
                && p.context.source_key == key
                && p.context == context(source, &p.context.features, config)?
                && valid_references(&tx, &p, Utc::now())?,
            "stale, superseded, or incompatible proposal"
        );
        ensure!(
            resources.profiles.iter().any(|r| r.eligibility().is_ok()
                && r.tier != ResourceTier::Light
                && crate::router::select_resource(
                    resources,
                    &p.context.features,
                    &config.execution.backend,
                    Some(&r.model),
                    r.effort.as_deref(),
                    None,
                    None
                )
                .ok()
                .is_some_and(
                    |d| behavior(&d.selected, config).is_ok_and(|b| b.key == p.target_behavior)
                )),
            "target configuration unavailable"
        );
        // Recompute screening from authoritative current facts; never trust counts in a file.
        let current_base = router_base(resources, &p.context.features, config)?;
        ensure!(
            behavior(&current_base.selected, config)?.key == p.source_behavior,
            "source profile changed; proposal no longer applies"
        );
        let summary = summarize(&tx, source, &p.context, Utc::now())?;
        let source_counts: Vec<_> = summary
            .resources
            .iter()
            .filter(|(k, _)| k.starts_with(&format!("{}|", p.source_behavior)))
            .collect();
        ensure!(
            !summary.truncated && source_counts.len() == 1 && source_counts[0].1 == &p.counts,
            "screening evidence changed; inspect a new proposal"
        );
        tx.execute(
            "UPDATE private_proposals SET status='active' WHERE id=?1 AND status='proposed'",
            [id],
        )?;
    }
    if let Some(previous) = previous {
        tx.execute(
            "UPDATE private_proposals SET status=?2 WHERE id=?1 AND status IN ('active','stale')",
            params![
                previous,
                if id.is_some() {
                    "superseded"
                } else {
                    "revoked"
                }
            ],
        )?;
    }
    tx.execute("INSERT INTO private_policy_transitions(source_key,revision,previous_revision,proposal_id,actor,created_at) VALUES (?1,?2,?3,?4,?5,?6)",params![key,revision+1,revision,id,actor,Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)])?;
    tx.commit()?;
    Ok(json!({"revision":revision+1,"previous_revision":revision,"proposal_id":id,"fallback":BASE}))
}

pub fn explain(state: &State, id: &str) -> Result<()> {
    let value = inspect(state, id)?;
    let current = &value["current"];
    let then = &value["at_decision_time"];
    println!(
        "Private evidence: decision mode {}; reason {}.",
        then["mode"].as_str().unwrap_or("historical_unknown"),
        then["reason"]
            .as_str()
            .unwrap_or("no_pre_execution_snapshot")
    );
    println!(
        "  Current project window: {} goals; {} reviewed; {} accepted; {} rejected; {} missing reviews.",
        current["counts"]["goals"],
        current["counts"]["reviewed"],
        current["counts"]["accepted"],
        current["counts"]["rejected"],
        current["counts"]["missing_review"]
    );
    println!(
        "  {} compatible resource cohorts; exclusions: {}. Observations are not success probabilities.",
        current["resources"].as_object().map_or(0, |m| m.len()),
        current["counts"]["exclusions"]
    );
    println!(
        "  Detail: dispatch evidence private {id} (decision-time and current evidence are separate)."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RoutingHumanOutcome, router};
    fn resources() -> ResourceConfig {
        serde_json::from_value(json!({"allocation_enabled":true,"profiles":[
            {"provider":"openai","funding_source":"fixture","harness":"codex","model":"light-fixed","effort":"low","service_mode":"standard","runtime":"local","pool":"fixture","tier":"light","included":true,"no_overage_verified":true},
            {"provider":"openai","funding_source":"fixture","harness":"codex","model":"standard-fixed","effort":"low","service_mode":"standard","runtime":"local","pool":"fixture","tier":"standard","included":true,"no_overage_verified":true}
        ]})).unwrap()
    }
    fn features() -> TaskFeatures {
        TaskFeatures {
            language: Some("rust".into()),
            task_kind: TaskKind::Tests,
            scope: TaskScope::Localized,
        }
    }
    fn config() -> Config {
        let mut c = Config::default();
        c.harnesses.codex.executable = Some("/bin/sh".into());
        c.checks.verify = vec!["test -f result.txt".into()];
        c.private_evidence.routine_mappings.push(Mapping {
            id: "explicit-routine-fixture".into(),
            features: features(),
            verify: c.checks.verify.clone(),
        });
        c
    }
    fn screened() -> (Summary, AllocationDecision, Vec<Behavior>) {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open_in_memory().unwrap();
        let c = config();
        let ctx = context(dir.path(), &features(), &c).unwrap();
        let mut summary = summarize(db.connection(), dir.path(), &ctx, Utc::now()).unwrap();
        let d = router::select_resource(&resources(), &features(), "local", None, None, None, None)
            .unwrap();
        let b = d
            .alternatives
            .iter()
            .map(|a| behavior(&a.choice, &c).unwrap())
            .collect::<Vec<_>>();
        summary.resources.insert(
            format!("{}|v1|unknown|unknown|{BASE}", b[0].key),
            Counts {
                goals: 25,
                reviewed: 20,
                accepted: 15,
                rejected: 5,
                quality_rejected: 5,
                ..Counts::default()
            },
        );
        summary.resources.insert(
            format!("{}|v1|unknown|unknown|{BASE}", b[1].key),
            Counts {
                goals: 1,
                verification: BTreeMap::from([("passed".into(), 1)]),
                ..Counts::default()
            },
        );
        (summary, d, b)
    }
    #[test]
    fn explicit_multifile_mapping_is_descriptive_only_and_checks_stay_exact() {
        let dir = tempfile::tempdir().unwrap();
        let f = TaskFeatures {
            language: Some("c".into()),
            task_kind: TaskKind::Feature,
            scope: TaskScope::MultiFile,
        };
        let mut c = Config::default();
        c.checks.verify = vec!["sh ./verify.sh".into()];
        assert!(context(dir.path(), &f, &c).unwrap().mapping.is_none());
        c.private_evidence.routine_mappings.push(Mapping {
            id: "raylib-feature-collisions".into(),
            features: f.clone(),
            verify: c.checks.verify.clone(),
        });
        let ctx = context(dir.path(), &f, &c).unwrap();
        assert!(ctx.mapping.is_some());
        // Even enough synthetic light-profile reviews cannot broaden rule v1.
        let (mut summary, mut decision, behaviors) = screened();
        summary.context = ctx;
        decision.task_features = f.clone();
        let (p, reason) = proposal(&summary, &decision, &behaviors).unwrap();
        assert!(p.is_none());
        assert_eq!(reason, "task_scope_outside_trial_rule");
        for invalid in [
            TaskFeatures {
                language: None,
                ..f.clone()
            },
            TaskFeatures {
                task_kind: TaskKind::Unknown,
                ..f.clone()
            },
            TaskFeatures {
                scope: TaskScope::Unknown,
                ..f.clone()
            },
            TaskFeatures {
                scope: TaskScope::Broad,
                ..f.clone()
            },
        ] {
            c.private_evidence.routine_mappings[0].features = invalid.clone();
            assert!(context(dir.path(), &invalid, &c).unwrap().mapping.is_none());
        }
        c.private_evidence.routine_mappings[0].features = f.clone();
        c.checks.verify = vec!["true".into()];
        assert!(context(dir.path(), &f, &c).unwrap().mapping.is_none());
        c.checks.verify = vec![];
        c.private_evidence.routine_mappings[0].verify.clear();
        assert!(context(dir.path(), &f, &c).unwrap().mapping.is_none());
        c.checks.verify = vec!["sh ./verify.sh".into()];
        c.private_evidence.routine_mappings[0].verify = c.checks.verify.clone();
        c.private_evidence
            .routine_mappings
            .push(c.private_evidence.routine_mappings[0].clone());
        assert!(context(dir.path(), &f, &c).unwrap().mapping.is_none());
    }

    #[test]
    fn c_test_and_feature_mappings_do_not_infer_unknown_scope() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("tests")).unwrap();
        std::fs::write(dir.path().join("main.c"), "// synthetic C source").unwrap();
        std::fs::write(
            dir.path().join("tests/collision_test.c"),
            "// synthetic test",
        )
        .unwrap();
        let mut config = Config::default();
        config.checks.verify = vec!["sh ./verify.sh".into()];
        for (id, kind, scope) in [
            ("feature", TaskKind::Feature, TaskScope::MultiFile),
            ("tests", TaskKind::Tests, TaskScope::Localized),
        ] {
            config.private_evidence.routine_mappings.push(Mapping {
                id: id.into(),
                features: TaskFeatures {
                    language: Some("c".into()),
                    task_kind: kind,
                    scope,
                },
                verify: config.checks.verify.clone(),
            });
        }
        let classify = |task| crate::classifier::classify_task(dir.path(), task).unwrap();
        let tests = classify("Add tests in tests/collision_test.c");
        let feature = classify("Add another platform in main.c and tests/collision_test.c");
        let unspecified = classify(
            "Add yet another platform. make them equidistant from one another and the border of the sim.",
        );
        let test_mapping = context(dir.path(), &tests, &config).unwrap().mapping;
        let feature_mapping = context(dir.path(), &feature, &config).unwrap().mapping;
        assert!(test_mapping.is_some() && feature_mapping.is_some());
        assert_ne!(test_mapping, feature_mapping);
        assert_eq!(unspecified.task_kind, TaskKind::Feature);
        assert_eq!(unspecified.scope, TaskScope::Unknown);
        assert!(
            context(dir.path(), &unspecified, &config)
                .unwrap()
                .mapping
                .is_none()
        );
        config.checks.verify.push("extra check".into());
        assert!(
            context(dir.path(), &tests, &config)
                .unwrap()
                .mapping
                .is_none()
        );
        assert!(
            context(dir.path(), &feature, &config)
                .unwrap()
                .mapping
                .is_none()
        );
    }

    #[test]
    fn default_config_preserves_historical_grant_digest_and_malformed_proposals_fail() {
        let config = serde_json::to_value(Config::default()).unwrap();
        assert!(config.get("private_evidence").is_none());
        let (summary, decision, behaviors) = screened();
        let mut p = proposal(&summary, &decision, &behaviors)
            .unwrap()
            .0
            .unwrap();
        p.parameters["minimum_reviewed"] = json!(1);
        let db = Database::open_in_memory().unwrap();
        let id = store_proposal(db.connection(), &p).unwrap();
        assert!(load_proposal(db.connection(), &id).is_err());
    }

    #[test]
    fn screening_parameters_are_not_calibration_or_activation() {
        let (s, d, b) = screened();
        let (p, why) = proposal(&s, &d, &b).unwrap();
        let p = p.unwrap();
        assert_eq!(why, "repeated_quality_rejections_trial_only");
        assert_eq!(p.parameters["minimum_reviewed"], 20);
        assert_eq!(d.selected.tier, ResourceTier::Light);
        assert_eq!(p.target.tier, ResourceTier::Standard);
        assert!(p.limitations.iter().any(|s| s.contains("unproven")));
        let db = Database::open_in_memory().unwrap();
        assert_eq!(active(db.connection(), "fixture").unwrap(), (0, None));
    }
    #[test]
    fn sparse_missing_conflicting_and_unmapped_cohorts_abstain() {
        let (s, d, b) = screened();
        for mode in 0..7 {
            let mut x = s.clone();
            let key = x
                .resources
                .keys()
                .find(|k| k.starts_with(&b[0].key))
                .unwrap()
                .clone();
            match mode {
                0 => x.resources.get_mut(&key).unwrap().reviewed = 19,
                1 => x.resources.get_mut(&key).unwrap().goals = 26,
                2 => x.resources.get_mut(&key).unwrap().quality_rejected = 4,
                3 => {
                    x.resources
                        .insert(format!("{}|v2", b[0].key), Counts::default());
                }
                4 => x.context.mapping = None,
                5 => x.truncated = true,
                _ => x.resources.retain(|k, _| !k.starts_with(&b[1].key)),
            }
            assert!(proposal(&x, &d, &b).unwrap().0.is_none(), "mode {mode}");
        }
    }
    #[test]
    fn constraints_unknown_identity_and_behavior_changes_are_conservative() {
        let (s, mut d, mut b) = screened();
        d.alternatives[1].eligible = false;
        assert!(proposal(&s, &d, &b).unwrap().0.is_none());
        d.alternatives[1].eligible = true;
        b[1].executable_known = false;
        assert!(proposal(&s, &d, &b).unwrap().0.is_none());
        let base = behavior(&d.selected, &config()).unwrap();
        let mut changed = d.selected.clone();
        changed.funding_source = "renewed-label".into();
        changed.pool = "renamed-pool".into();
        assert_eq!(base, behavior(&changed, &config()).unwrap());
        changed.effort = Some("high".into());
        assert_ne!(base.key, behavior(&changed, &config()).unwrap().key);
        let mut c = config();
        c.harnesses
            .codex
            .extra_args
            .push("--different-tools".into());
        assert_ne!(base.key, behavior(&d.selected, &c).unwrap().key);
    }
    #[test]
    fn terminal_cache_categories_do_not_add_breakdowns_or_estimate_cash() {
        let events = vec![
            json!({"type":"assistant","usage":{"input_tokens":999}}),
            json!({"type":"result","usage":{"input_tokens":14,"output_tokens":727,"cache_read_input_tokens":97140,"cache_creation_input_tokens":2419},"total_cost_usd":0.036402,"modelUsage":{"fixed":{"inputTokens":999999}}}),
        ];
        let usage = crate::harness::claude::usage_categories(&events);
        assert_eq!(usage.values().sum::<u64>(), 100300);
        assert_eq!(usage.len(), 4);
        assert!(
            crate::harness::parse_jsonl_telemetry("")
                .usage_categories
                .is_empty()
        );
        let missing = crate::harness::claude::usage_categories(&[
            json!({"type":"result","usage":{"output_tokens":1}}),
        ]);
        assert_eq!(missing.len(), 1);
        assert!(!missing.contains_key("input_tokens"));
        let mut duplicate = events.clone();
        duplicate.push(events[1].clone());
        assert!(crate::harness::claude::usage_categories(&duplicate).is_empty());
        assert_eq!(Economics::default().cash_cost, None);
        assert_eq!(Economics::default().harness_ms_per_accepted, None);
    }
    fn empty_run(source: &Path) -> RunRecord {
        serde_json::from_value(json!({"id":"synthetic-snapshot","task":"synthetic","exact_prompt":"synthetic","source_path":source,"source_kind":"directory","source_git_head":null,"source_fingerprint":"fixture","baseline_path":source,"baseline_commit":"fixture","status":"ready_for_evaluation","created_at":Utc::now()-Duration::seconds(1),"completed_at":Utc::now(),"environment":{"dispatch_version":"fixture","os":"fixture","architecture":"fixture","execution_backend":"local","timeout_secs":30,"cpus":1.0,"memory":"1g","max_parallel":1},"evaluation":null,"applied_candidate":null})).unwrap()
    }
    #[test]
    fn one_sqlite_snapshot_keeps_feedback_revision_coherent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let mut writer = Database::open(&path).unwrap();
        let run = empty_run(dir.path());
        writer.sync_run(&run).unwrap();
        let first = writer
            .save_goal_feedback(
                &run.id,
                RoutingHumanOutcome::Rejected,
                vec!["correctness".into()],
                None,
            )
            .unwrap();
        let reader = Database::open(&path).unwrap();
        let tx = reader.connection().unchecked_transaction().unwrap();
        let cutoff = Utc::now() + Duration::seconds(1);
        assert_eq!(feedback(&tx, &run.id, cutoff).unwrap().unwrap().0, first.id);
        let second = writer
            .save_goal_feedback(&run.id, RoutingHumanOutcome::Accepted, vec![], None)
            .unwrap();
        assert_eq!(feedback(&tx, &run.id, cutoff).unwrap().unwrap().0, first.id);
        tx.commit().unwrap();
        assert_eq!(
            feedback(reader.connection(), &run.id, cutoff)
                .unwrap()
                .unwrap()
                .0,
            second.id
        );
        assert_eq!(
            feedback(reader.connection(), &run.id, first.created_at)
                .unwrap()
                .unwrap()
                .0,
            first.id
        );
        assert_eq!(
            writer
                .latest_goal_feedback(&run.id)
                .unwrap()
                .unwrap()
                .revision,
            2
        );
    }
    #[test]
    fn fixture_domain_cannot_be_enabled_after_history_or_removed() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Database::open_in_memory().unwrap();
        db.sync_run(&empty_run(dir.path())).unwrap();
        assert!(
            db.connection()
                .execute("INSERT INTO private_fixture_domain VALUES (1)", [])
                .is_err()
        );
        let fresh = Database::open_in_memory().unwrap();
        fresh
            .connection()
            .execute("INSERT INTO private_fixture_domain VALUES (1)", [])
            .unwrap();
        assert!(
            fresh
                .connection()
                .execute("DELETE FROM private_fixture_domain", [])
                .is_err()
        );
        assert!(
            fresh
                .connection()
                .execute("UPDATE private_fixture_domain SET singleton=1", [])
                .is_err()
        );
    }
}
