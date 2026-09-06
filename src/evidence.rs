//! Inspection-only local counts. These are not benchmark priors or Router inputs.
use std::path::Path;

use anyhow::Result;

use crate::{
    CandidateStatus, CheckStatus, RoutingHumanOutcome, RoutingObservation, TaskFeatures,
    db::Database, source, state::State,
};

/// One exact morphology/harness group within the caller's local source filter.
/// Model/version remain in the raw observations, not grouping dimensions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalRoutingEvidence {
    pub task_features: TaskFeatures,
    pub harness: String,
    pub total_runs: u64,
    pub human_evaluated: u64,
    pub human_accepted: u64,
    pub human_rejected: u64,
    pub human_unevaluated: u64,
    pub verification_observed: u64,
    pub verification_passed: u64,
    pub verification_failed: u64,
    pub verification_timed_out: u64,
    pub verification_not_run: u64,
    pub verification_unknown: u64,
    pub process_completed: u64,
    pub process_failed: u64,
    pub process_timed_out: u64,
    pub process_cancelled: u64,
    pub process_missing_harness: u64,
}

impl LocalRoutingEvidence {
    pub(crate) fn count(
        &mut self,
        observation: &RoutingObservation,
        human: Option<&RoutingHumanOutcome>,
    ) {
        self.total_runs += 1;
        match human {
            Some(RoutingHumanOutcome::Accepted) => self.human_accepted += 1,
            Some(RoutingHumanOutcome::Rejected) => self.human_rejected += 1,
            None => self.human_unevaluated += 1,
        }
        self.human_evaluated = self.human_accepted + self.human_rejected;

        // One summary per run, following show precedence: fail, timeout, not-run,
        // then all-pass. An empty list cannot establish a verification outcome.
        match observation.verification.as_deref() {
            None | Some([]) => self.verification_unknown += 1,
            Some(checks) if checks.contains(&CheckStatus::Failed) => self.verification_failed += 1,
            Some(checks) if checks.contains(&CheckStatus::TimedOut) => {
                self.verification_timed_out += 1
            }
            Some(checks) if checks.contains(&CheckStatus::NotRun) => self.verification_not_run += 1,
            Some(_) => self.verification_passed += 1,
        }
        self.verification_observed =
            self.verification_passed + self.verification_failed + self.verification_timed_out;
        match observation.candidate_status {
            CandidateStatus::Completed => self.process_completed += 1,
            CandidateStatus::Failed => self.process_failed += 1,
            CandidateStatus::TimedOut => self.process_timed_out += 1,
            CandidateStatus::Cancelled => self.process_cancelled += 1,
            CandidateStatus::MissingHarness => self.process_missing_harness += 1,
            CandidateStatus::Preparing | CandidateStatus::Running | CandidateStatus::Verifying => {
                unreachable!("only terminal observations are aggregated")
            }
        }
    }
}

pub fn inspect_local(state: &State, source_path: &Path) -> Result<()> {
    let source_path = source::resolve_source(Some(source_path))?;
    let groups = if state.db_path().exists() {
        Database::open(state.db_path())?.local_routing_evidence(&source_path)?
    } else {
        Vec::new()
    };
    println!("Local routing evidence");
    println!("Scope: this source location only; moves and separate worktrees are not unified.");
    println!("Observed under past routing choices and optional human review; not a ranking.");
    if groups.is_empty() {
        println!("\nNo local routing observations for this source.");
    }
    for e in groups {
        println!(
            "\n{} / {} / {}\n{}",
            e.task_features.language.as_deref().unwrap_or("unknown"),
            e.task_features.task_kind.as_str(),
            e.task_features.scope.as_str(),
            e.harness
        );
        println!("  routed runs: {}", e.total_runs);
        println!("  human evaluated: {}", e.human_evaluated);
        println!("  accepted: {}", ratio(e.human_accepted, e.human_evaluated));
        println!("  rejected: {}", ratio(e.human_rejected, e.human_evaluated));
        println!("  unevaluated: {}", e.human_unevaluated);
        println!("  verification observed: {}", e.verification_observed);
        println!(
            "  passed: {}",
            ratio(e.verification_passed, e.verification_observed)
        );
        println!("  verification failed: {}", e.verification_failed);
        println!("  verification timed_out: {}", e.verification_timed_out);
        println!("  verification not_run: {}", e.verification_not_run);
        println!("  verification unknown: {}", e.verification_unknown);
        println!(
            "  process completed: {}/{}",
            e.process_completed, e.total_runs
        );
        println!("  process failed: {}", e.process_failed);
        println!("  process timed_out: {}", e.process_timed_out);
        println!("  process cancelled: {}", e.process_cancelled);
        println!("  process missing_harness: {}", e.process_missing_harness);
    }
    Ok(())
}

fn ratio(numerator: u64, denominator: u64) -> String {
    if denominator == 0 {
        "not observed (0/0)".into()
    } else {
        format!(
            "{numerator}/{denominator} ({:.1}%)",
            numerator as f64 / denominator as f64 * 100.0
        )
    }
}
