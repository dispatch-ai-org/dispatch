//! Applying a finished candidate to the source: the human path (`accept`,
//! the review view, `dispatch apply`) and the policy path (`auto_apply`).
//!
//! Both go through `apply_locked`, which holds the per-source lock, asks
//! `coherence::gate` whether the work is still valid against the source as it
//! is now, and applies through the digest or fingerprint fence. What differs
//! is the authority: a human decision records acceptance, a policy never does.

use super::*;
use crate::{
    AppliedBy,
    coherence::{AcceptGate, CoherenceBlocked},
    source::ApplyReport,
};

/// Who is applying. This is threaded into the persisted outcome and events so
/// that an application by policy is never recorded as human acceptance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyAuthority {
    /// A person accepted the result (or typed `dispatch apply`).
    Human,
    /// The auto-apply policy of the owning session or invocation.
    AutoApply,
}

impl ApplyAuthority {
    pub fn applied_by(self) -> AppliedBy {
        match self {
            Self::Human => AppliedBy::Human,
            Self::AutoApply => AppliedBy::AutoApply,
        }
    }
}

/// What an automatic application attempt did. Every variant has already been
/// persisted (events and outcome) by the time it is returned; the caller only
/// presents it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// The source was patched; `validity` is present when the world had moved.
    Applied {
        report: ApplyReport,
        validity: Option<Validity>,
    },
    /// The gate, or the authorization predicate on top of it, refused. The
    /// source is unchanged and the run remains reviewable.
    Blocked {
        reason: String,
        validity: Option<Validity>,
    },
    /// The run was not eligible; nothing external ran.
    Skipped { reason: String },
    /// The apply itself failed after authorization (for example Git).
    Failed { error: String },
}

/// Store the latest validity on the run, remembering when work first became
/// invalid so that later evidence can show how long it ran on a false premise.
pub(super) fn remember_validity(run: &mut RunRecord, validity: &Validity) {
    let mut record = run.coherence.take().unwrap_or(CoherenceRecord {
        version: 1,
        refreshed_from: None,
        facts: Vec::new(),
        validity: None,
        first_invalid_at: None,
    });
    if validity.decision != Decision::Continue && record.first_invalid_at.is_none() {
        record.first_invalid_at = Some(validity.evaluated_at);
    }
    record.validity = Some(validity.clone());
    run.coherence = Some(record);
}

pub fn apply(state: &State, run_id: &str, candidate_label: &str) -> Result<()> {
    let resolved_run_id = state.resolve_run_id(run_id)?;
    let _run_lock = OperationLock::acquire(
        &state.run_dir(&resolved_run_id).join(".operation.lock"),
        "another compare/apply operation is already using this run",
    )?;
    let run = state.load_run(&resolved_run_id)?;
    apply_locked(state, run, candidate_label, false, ApplyAuthority::Human)
}

/// Apply `candidate_label` of `run` onto the source under the per-source lock.
/// The caller holds the run's operation lock. `authority` names who is
/// applying: only a human decision marks the review as accepted; a policy
/// application leaves the review exactly as it was.
pub(super) fn apply_locked(
    state: &State,
    mut run: RunRecord,
    candidate_label: &str,
    quiet: bool,
    authority: ApplyAuthority,
) -> Result<()> {
    anyhow::ensure!(
        matches!(
            run.status,
            RunStatus::ReadyForEvaluation | RunStatus::Evaluated
        ),
        "run {} is {}; apply requires a completed, unapplied run",
        run.id,
        run.status.as_str()
    );
    crate::planning::verify_delivery(&run)?;
    let candidate = find_candidate(&run, candidate_label)?;
    let normalized_label = candidate.label.clone();
    let source_key = hex::encode(Sha256::digest(run.source_path.to_string_lossy().as_bytes()));
    let _source_lock = OperationLock::acquire(
        &state
            .root
            .join("locks")
            .join(format!("source-{source_key}.lock")),
        "another apply operation is already modifying this source",
    )?;
    let applied = crate::coherence::gate(&run, &normalized_label, &state.run_dir(&run.id))
        .and_then(|gate| match gate {
            AcceptGate::Legacy => Ok((source::safe_apply(&run, &normalized_label)?, None)),
            AcceptGate::Compatible(validity) => Ok((
                source::apply_validated(&run, &normalized_label, &validity.world_digest)?,
                Some(validity),
            )),
            AcceptGate::Blocked(validity) => Err(CoherenceBlocked {
                run_id: run.id.clone(),
                validity,
            }
            .into()),
        });
    let (report, validity) = match applied {
        Ok(applied) => applied,
        Err(error) => {
            let message = format!("{error:#}");
            let blocked = error.downcast_ref::<CoherenceBlocked>();
            run.outcome.application = if blocked.is_some()
                || message.contains("source changed")
                || message.contains("source has changed")
                || message.contains("source drift")
            {
                ApplicationState::BlockedBySourceDrift
            } else {
                ApplicationState::Failed
            };
            run.outcome.phase = RunPhase::Finished;
            if let Some(blocked) = blocked {
                remember_validity(&mut run, &blocked.validity);
            }
            let database = Database::open(state.db_path())?;
            persist_event(
                state,
                &database,
                EventRecord {
                    run_id: run.id.clone(),
                    candidate_label: Some(normalized_label.clone()),
                    event_type: "application.failed".into(),
                    timestamp: Utc::now(),
                    payload: serde_json::json!({
                        "application": run.outcome.application,
                        "error": message,
                        "coherence": blocked.map(|blocked| &blocked.validity),
                    }),
                    ..EventRecord::default()
                },
                &mut run,
            )?;
            return Err(error);
        }
    };
    if let Some(validity) = &validity {
        remember_validity(&mut run, validity);
    }
    run.applied_candidate = Some(normalized_label.clone());
    run.status = RunStatus::Applied;
    if authority == ApplyAuthority::Human {
        run.outcome.review = ReviewState::Accepted;
    }
    run.outcome.application = ApplicationState::Applied;
    run.outcome.applied_by = Some(authority.applied_by());
    run.outcome.phase = RunPhase::Finished;
    let mut db = Database::open(state.db_path())?;
    db.sync_run(&run)?;
    persist_event(
        state,
        &db,
        EventRecord {
            run_id: run.id.clone(),
            candidate_label: Some(normalized_label.clone()),
            event_type: "result.applied".into(),
            timestamp: Utc::now(),
            payload: serde_json::json!({
                "files_changed": report.files_changed,
                "coherence": validity.as_ref().filter(|validity| validity.world_changed),
            }),
            ..EventRecord::default()
        },
        &mut run,
    )?;
    if !quiet {
        println!(
            "Applied Candidate {} to {} ({} file(s) changed).",
            normalized_label,
            run.source_path.display(),
            report.files_changed
        );
    }
    Ok(())
}

/// Apply `run_id`'s sole candidate under the auto-apply policy, if and only if
/// the evidence is complete for the exact world being modified (see
/// `docs/plan-0.3-auto-apply-and-attach.md`, part 5.2). This skeleton is not
/// yet wired to any surface: it reports the run as not ready and does nothing.
pub fn auto_apply(state: &State, run_id: &str) -> Result<ApplyOutcome> {
    let resolved_run_id = state.resolve_run_id(run_id)?;
    let _run = state.load_run(&resolved_run_id)?;
    Ok(ApplyOutcome::Skipped {
        reason: "not_ready".into(),
    })
}
