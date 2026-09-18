use super::*;
use crate::{
    AttemptDetail, Clarification, FailureKind, GoalExecution, QuestionState, ResourceTier,
};
use rusqlite::OptionalExtension;

pub(crate) fn local_uid() -> u32 {
    #[cfg(unix)]
    {
        unsafe { libc::geteuid() }
    }
    #[cfg(not(unix))]
    {
        0
    }
}

pub(super) struct DeadlineGuard(tokio::task::JoinHandle<()>);
impl DeadlineGuard {
    pub(super) fn new(
        policy: Option<&GoalExecution>,
        cancellation: CancellationToken,
    ) -> Option<Self> {
        let deadline = policy?.deadline_at;
        Some(Self(tokio::spawn(async move {
            tokio::time::sleep((deadline - Utc::now()).to_std().unwrap_or_default()).await;
            cancellation.cancel();
        })))
    }
}
impl Drop for DeadlineGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn expects_execution(run: &RunRecord) -> bool {
    run.phase3.is_some()
        && run.outcome.lifecycle != LifecycleState::Finished
        && run.outcome.waiting_on != WaitingOn::Human
        && run.outcome.work_result != WorkResult::Cancelled
        && run.outcome.review != ReviewState::Rejected
}

fn unresolved_admission(db: &Database, id: &str) -> Result<bool> {
    Ok(db.connection().query_row(
        "SELECT EXISTS(SELECT 1 FROM admission_requests WHERE run_id=?1 AND status IN ('queued','admitted','reconciliation')) OR EXISTS(SELECT 1 FROM pool_leases l JOIN admission_requests r ON r.id=l.request_id WHERE r.run_id=?1)",
        [id], |row| row.get(0),
    )?)
}

fn interrupt_run(state: &State, db: &Database, run: &mut RunRecord, reason: &str) -> Result<()> {
    run.status = RunStatus::Interrupted;
    run.completed_at = Some(Utc::now());
    run.outcome.lifecycle = LifecycleState::Finished;
    run.outcome.work_result = WorkResult::Interrupted;
    run.outcome.phase = RunPhase::Finished;
    run.outcome.waiting_on = if unresolved_admission(db, &run.id)? {
        WaitingOn::Reconciliation
    } else {
        WaitingOn::None
    };
    run.phase3
        .as_mut()
        .unwrap()
        .failure
        .get_or_insert(FailureKind::InternalState);
    transition(
        state,
        db,
        run,
        "run.interrupted",
        serde_json::json!({"reason":reason}),
    )
}

// Called only after this foreground owner's work/guards have returned. Read
// the committed projection so a publication error cannot rewrite evidence.
fn finish_error<T>(state: &State, db: &Database, id: &str, result: Result<T>) -> Result<T> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            let repair = (|| -> Result<()> {
                if let Some(mut run) = db.committed_run_projection(id)?
                    && expects_execution(&run)
                {
                    interrupt_run(
                        state,
                        db,
                        &mut run,
                        &format!("foreground execution stopped: {error:#}"),
                    )?;
                }
                Ok(())
            })();
            match repair {
                Ok(()) => Err(error),
                Err(repair) => {
                    Err(error.context(format!("could not persist interruption: {repair:#}")))
                }
            }
        }
    }
}

pub(crate) fn repair_abandoned(state: &State, db: &Database, run: &mut RunRecord) -> Result<()> {
    if !expects_execution(run) {
        return Ok(());
    }
    let Ok(_lock) = OperationLock::acquire(
        &state.run_dir(&run.id).join(".operation.lock"),
        "run is supervised",
    ) else {
        return Ok(());
    };
    // The owner may have committed a question, delivery or new supervisor
    // since the caller read its projection. Re-read while holding the lock.
    let Some(current) = db.committed_run_projection(&run.id)? else {
        return Ok(());
    };
    *run = current;
    let mut supervisor = run.phase3.as_ref().and_then(|p| p.supervisor.clone());
    // Existing Phase 3 records already have an admission-owner identity.
    // A new answer owner holds the run lock, even for these older records.
    if supervisor.is_none() {
        supervisor = db.connection().query_row(
            "SELECT owner_pid,owner_start_identity,owner_boot_identity FROM admission_requests WHERE run_id=?1 ORDER BY enqueued_at DESC,id DESC LIMIT 1",
            [&run.id], |row| {
                Ok(crate::admission::identity_from_row(row.get(0)?, row.get(1)?, row.get(2)?, None))
            },
        ).optional()?.flatten();
    }
    let gone = supervisor.as_ref().is_some_and(|owner| {
        matches!(
            crate::admission::identity_state(owner),
            crate::admission::IdentityState::Gone | crate::admission::IdentityState::Reused
        )
    });
    // Completed attempts affirm that verification has also returned. An
    // unfinished attempt or unresolved lease can still have a live child;
    // leave that uncertainty to the existing supervision/reconciliation path.
    if expects_execution(run)
        && gone
        && !run.attempts.is_empty()
        && run.attempts.iter().all(|a| a.completed_at.is_some())
        && !unresolved_admission(db, &run.id)?
    {
        interrupt_run(
            state,
            db,
            run,
            "foreground supervisor is gone; work was not replayed",
        )?;
    }
    Ok(())
}

fn transition(
    state: &State,
    db: &Database,
    run: &mut RunRecord,
    kind: &str,
    payload: serde_json::Value,
) -> Result<()> {
    persist_event(
        state,
        db,
        EventRecord {
            run_id: run.id.clone(),
            attempt_id: run.attempts.last().map(|a| a.id.clone()),
            event_type: kind.into(),
            timestamp: Utc::now(),
            payload,
            ..Default::default()
        },
        run,
    )
}

pub(super) fn emit(run: &RunRecord, output: RunOutputMode) -> Result<()> {
    match output {
        RunOutputMode::Silent => {}
        RunOutputMode::Json => println!("{}", serde_json::to_string(&run_result(run))?),
        RunOutputMode::Jsonl => println!(
            "{}",
            serde_json::json!({"type":"result","result":run_result(run)})
        ),
        RunOutputMode::Human => {
            if let Some(q) = run
                .phase3
                .as_ref()
                .and_then(|p| p.questions.last())
                .filter(|q| q.state == QuestionState::Pending)
            {
                println!(
                    "{}\nQuestion {} (revision {}) — answer with dispatch answer {} {} --revision {} --answer <text>",
                    q.report.question, q.id, q.revision, run.id, q.id, q.revision
                );
            } else if !run.candidates.is_empty() {
                print_single_result_summary(
                    run,
                    if run.outcome.verification == VerificationState::Passed {
                        "Ready for review"
                    } else {
                        "Work stopped; inspect verification and attempt details"
                    },
                    false,
                    None,
                );
            } else {
                println!(
                    "Work stopped: {:?}",
                    run.phase3.as_ref().and_then(|p| p.failure)
                );
            }
            for attempt in &run.attempts {
                println!(
                    "  Attempt {}: {} / {} — {}{}",
                    attempt.ordinal,
                    attempt.resolved_model.as_deref().unwrap_or("unknown"),
                    attempt.resolved_effort.as_deref().unwrap_or("unknown"),
                    attempt.outcome,
                    attempt
                        .detail
                        .reason
                        .as_ref()
                        .map(|r| format!(" ({r})"))
                        .unwrap_or_default()
                );
            }
            println!(
                "{} attempt(s); final attempt: {}",
                run.attempts.len(),
                run.phase3
                    .as_ref()
                    .and_then(|p| p.final_attempt_id.as_deref())
                    .unwrap_or("none")
            );
        }
    }
    Ok(())
}

pub(super) fn stop(
    state: &State,
    db: &mut Database,
    run: &mut RunRecord,
    failure: FailureKind,
    reason: &str,
) -> Result<()> {
    if RUN_OUTPUT_MODE.load(Ordering::Relaxed) != RunOutputMode::Silent.code() {
        eprintln!("{reason}");
    }
    run.phase3.as_mut().unwrap().failure = Some(failure);
    run.completed_at = Some(Utc::now());
    run.status = if failure == FailureKind::CapacityAdmission {
        RunStatus::Deferred
    } else {
        RunStatus::Failed
    };
    run.outcome.lifecycle = LifecycleState::Finished;
    run.outcome.work_result = match failure {
        FailureKind::Cancelled => WorkResult::Cancelled,
        FailureKind::CapacityAdmission => WorkResult::Deferred,
        _ => WorkResult::Failed,
    };
    run.outcome.phase = RunPhase::Finished;
    run.outcome.review = ReviewState::NotRequested;
    run.outcome.waiting_on = if run
        .admission
        .as_ref()
        .is_some_and(|a| a.state == AdmissionState::Reconciliation)
    {
        WaitingOn::Reconciliation
    } else {
        WaitingOn::None
    };
    if let Some(attempt) = run.attempts.last_mut().filter(|a| a.completed_at.is_none()) {
        attempt.completed_at = run.completed_at;
        attempt.outcome = "not_launched".into();
        attempt.detail.failure = Some(failure);
        attempt.detail.capacity = run.capacity.clone();
        attempt.detail.admission = run.admission.clone().filter(|a| a.attempt_id == attempt.id);
    }
    db.sync_run(run)?;
    transition(
        state,
        db,
        run,
        "run.stopped",
        serde_json::json!({"failure":failure,"reason":reason}),
    )
}

pub(super) fn interrupted(run: &RunRecord) -> FailureKind {
    if Utc::now() >= run.phase3.as_ref().unwrap().deadline_at {
        FailureKind::Deadline
    } else {
        FailureKind::Cancelled
    }
}

fn rank(tier: &ResourceTier) -> u8 {
    match tier {
        ResourceTier::Light => 0,
        ResourceTier::Standard => 1,
        ResourceTier::Strong => 2,
    }
}

fn verification_failure(run: &RunRecord, candidate: &CandidateRecord) -> Option<FailureKind> {
    // Required checks must all be evaluable before any target failure can
    // justify spending. Missing exits include signals and failed spawns.
    if candidate
        .checks
        .iter()
        .any(|c| matches!(c.exit_code, Some(126 | 127)) || c.status == CheckStatus::TimedOut)
    {
        return Some(FailureKind::VerificationInfrastructure);
    }
    if candidate.checks.iter().any(|c| {
        !matches!(
            (&c.status, c.exit_code),
            (CheckStatus::Passed, Some(0)) | (CheckStatus::Failed, Some(1..=125))
        )
    }) {
        return Some(FailureKind::VerificationUnknown);
    }
    let failed = candidate
        .checks
        .iter()
        .filter(|c| c.status == CheckStatus::Failed);
    failed.clone().next()?;
    if run
        .baseline_checks
        .iter()
        .all(|b| b.status == CheckStatus::Passed)
        && failed.clone().any(|c| {
            run.baseline_checks
                .iter()
                .any(|b| b.command == c.command && b.status == CheckStatus::Passed)
        })
    {
        Some(FailureKind::TargetVerification)
    } else {
        Some(FailureKind::BaselineInfrastructure)
    }
}

async fn recovery_route(
    state: &State,
    run: &RunRecord,
    resources: &crate::config::ResourceConfig,
    config: &Config,
) -> Option<AllocationDecision> {
    let policy = run.phase3.as_ref()?;
    if policy.no_retry || run.attempts.len() >= policy.max_invocations.min(2) as usize {
        return None;
    }
    let previous = run.attempts.last()?.detail.decision.as_ref()?;
    let mut profiles = resources
        .profiles
        .iter()
        .filter(|p| {
            p.enabled
                && rank(&p.tier) > rank(&previous.selected.tier)
                && policy
                    .fixed_harness
                    .as_deref()
                    .is_none_or(|h| h == p.harness)
                && policy.fixed_model.as_deref().is_none_or(|m| m == p.model)
                && policy
                    .fixed_effort
                    .as_deref()
                    .is_none_or(|e| Some(e) == p.effort.as_deref())
        })
        .collect::<Vec<_>>();
    profiles.sort_by_key(|p| rank(&p.tier));
    let mut config = config.clone();
    // Dynamic selection is not a project constraint on a subsequent attempt.
    let old = if previous.selected.harness == "claude" {
        &mut config.harnesses.claude
    } else {
        &mut config.harnesses.codex
    };
    old.model = policy.fixed_model.clone();
    old.effort = policy.fixed_effort.clone();
    let mut blocked = None;
    for p in profiles {
        // Keep a bound recovery disposition for the existing admission authority
        // when every suitable resource is blocked; it records the specific cause.
        if blocked.is_none() {
            let mut constrained = resources.clone();
            constrained
                .profiles
                .retain(|candidate| candidate.harness == p.harness);
            blocked = crate::router::select_resource(
                &constrained,
                &previous.task_features,
                &run.environment.execution_backend,
                Some(&p.model),
                p.effort.as_deref(),
                policy.fixed_model.as_deref(),
                policy.fixed_effort.as_deref(),
            )
            .ok();
        }
        if let Ok(mut decision) = super::select_available_resource(
            state,
            &run.source_path,
            resources,
            &previous.task_features,
            &config,
            Some(&p.harness),
            Some(&p.model),
            p.effort.as_deref(),
            false,
        )
        .await
        {
            if rank(&decision.selected.tier) <= rank(&previous.selected.tier) {
                continue;
            }
            decision.policy_version = "bounded-recovery-v1".into();
            decision.reason = "One stronger recovery after a target check that passed on the original baseline failed after implementation.".into();
            return Some(decision);
        }
    }
    blocked
}

fn diagnostics(attempt: &AttemptRecord) -> String {
    let Some(candidate) = &attempt.detail.result else {
        return String::new();
    };
    let mut text = format!(
        "Previous attempt {} restarted from the same original baseline. Its workspace is retained and is not your input.\n",
        attempt.id
    );
    for check in candidate
        .checks
        .iter()
        .filter(|c| c.status != CheckStatus::Passed)
    {
        text.push_str(&format!("Failed check: {}\n", check.command));
        for path in [&check.stdout_path, &check.stderr_path] {
            let mut bytes = Vec::new();
            if let Ok(file) = fs::File::open(path) {
                let _ = file.take(4096).read_to_end(&mut bytes);
            }
            text.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    text.truncate(text.floor_char_boundary(16_384.min(text.len())));
    text
}

fn prepare(
    state: &State,
    run: &mut RunRecord,
    decision: AllocationDecision,
    reason: Option<String>,
) -> Result<CandidateRecord> {
    let id = Ulid::new().to_string();
    let dir = state.run_dir(&run.id).join("attempts").join(&id);
    let workspace = source::create_candidate_workspace(&run.baseline_path, &dir.join("workspace"))?;
    let mut prompt = run.exact_prompt.clone();
    if let Some(parent) = run.attempts.last() {
        prompt.push_str(&format!("\n\n{}", diagnostics(parent)));
    }
    if let Some(question) = run
        .phase3
        .as_ref()
        .unwrap()
        .questions
        .last()
        .filter(|q| q.state == QuestionState::Answered)
    {
        prompt.push_str(&format!(
            "\nClarification: {}\nAuthorized answer: {}\n",
            question.report.question,
            question.answer.as_deref().unwrap_or_default()
        ));
    }
    prompt.push_str("\nIf an essential ambiguity prevents safe completion, stop work and emit exactly this JSON as your final agent message: {\"dispatch_checkpoint\":{\"version\":1,\"question\":\"the essential question\",\"choices\":[]}}. You may add \"category\":\"factual\" inside dispatch_checkpoint only for factual task clarification. Omit category for funding, unsafe execution, permissions, or human review decisions. Do not wait in a live process. Otherwise complete the task normally.\n");
    let candidate = CandidateRecord {
        id: id.clone(),
        label: "A".into(),
        harness_id: decision.selected.harness.clone(),
        harness_version: None,
        model: None,
        status: CandidateStatus::Preparing,
        workspace_path: workspace,
        prompt_path: dir.join("prompt.txt"),
        stdout_path: dir.join("stdout.log"),
        stderr_path: dir.join("stderr.log"),
        diff_path: dir.join("diff.patch"),
        duration_ms: 0,
        exit_code: None,
        timed_out: false,
        tokens: None,
        token_semantics: None,
        cost_usd: None,
        error: None,
        diff_stats: DiffStats::default(),
        checks: vec![],
    };
    write_text(&candidate.prompt_path, &prompt)?;
    run.attempts.push(AttemptRecord {
        detail: AttemptDetail {
            parent_attempt_id: run.attempts.last().map(|a| a.id.clone()),
            reason,
            input_baseline: Some(run.baseline_path.clone()),
            decision: Some(decision.clone()),
            ..Default::default()
        },
        id,
        run_id: run.id.clone(),
        candidate_id: candidate.id.clone(),
        role: "executor".into(),
        ordinal: (run.attempts.len() + 1) as u32,
        generation: 1,
        harness_id: candidate.harness_id.clone(),
        harness_version: None,
        requested_model: Some(decision.selected.requested_model.clone()),
        resolved_model: Some(decision.selected.resolved_model.clone()),
        observed_model: None,
        requested_effort: decision.selected.effort.clone(),
        resolved_effort: decision.selected.effort.clone(),
        observed_effort: None,
        started_at: Utc::now(),
        completed_at: None,
        outcome: "preparing".into(),
        raw_telemetry_path: dir.join("harness.jsonl"),
        resource: Some(decision.selected),
    });
    run.candidates = vec![candidate.clone()];
    Ok(candidate)
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn drive(
    state: &State,
    db: &mut Database,
    run: RunRecord,
    config: Config,
    resources: crate::config::ResourceConfig,
    cancellation: CancellationToken,
    output: RunOutputMode,
) -> Result<RunRecord> {
    let id = run.id.clone();
    let result = drive_inner(state, db, run, config, resources, cancellation, output).await;
    finish_error(state, db, &id, result)
}

#[allow(clippy::too_many_arguments)]
async fn drive_inner(
    state: &State,
    db: &mut Database,
    mut run: RunRecord,
    mut config: Config,
    mut resources: crate::config::ResourceConfig,
    cancellation: CancellationToken,
    output: RunOutputMode,
) -> Result<RunRecord> {
    anyhow::ensure!(
        run.outcome.review != ReviewState::Rejected
            && run.outcome.work_result != WorkResult::Cancelled,
        "closed work cannot continue"
    );
    let initial = run.attempts.is_empty();
    if initial
        && run.baseline_checks.iter().any(|c| {
            !matches!(c.status, CheckStatus::Passed | CheckStatus::Failed)
                || matches!(c.exit_code, Some(126 | 127))
                || (c.status == CheckStatus::Failed && c.exit_code.is_none())
        })
    {
        stop(
            state,
            db,
            &mut run,
            FailureKind::BaselineInfrastructure,
            "baseline checks did not pass; no model escalation",
        )?;
        emit(&run, output)?;
        return Ok(run);
    }
    let mut decision = if initial {
        run.allocation.clone().unwrap()
    } else {
        run.attempts
            .last()
            .unwrap()
            .detail
            .decision
            .clone()
            .context("continuation route missing")?
    };
    let mut reason = (!initial).then(|| "clarification_answer".to_owned());
    loop {
        if cancellation.is_cancelled() || Utc::now() >= run.phase3.as_ref().unwrap().deadline_at {
            let failure = interrupted(&run);
            stop(
                state,
                db,
                &mut run,
                failure,
                "goal cancelled or deadline expired",
            )?;
            break;
        }
        if run.attempts.len() >= run.phase3.as_ref().unwrap().max_invocations.min(2) as usize {
            stop(
                state,
                db,
                &mut run,
                FailureKind::InvocationLimit,
                "goal invocation limit reached",
            )?;
            break;
        }
        if source::fingerprint_tree(&run.source_path)? != run.source_fingerprint {
            stop(
                state,
                db,
                &mut run,
                FailureKind::SourceDrift,
                "source drift before fresh attempt",
            )?;
            break;
        }
        let selected = &decision.selected;
        if !resources.profiles.iter().any(|p| {
            p.provider == selected.provider
                && p.funding_source == selected.funding_source
                && p.pool == selected.pool
                && p.model == selected.resolved_model
                && p.effort == selected.effort
                && p.harness == selected.harness
                && p.enabled
                && p.runtime == selected.runtime
                && p.service_mode == selected.service_mode
                && p.included
                && p.no_overage_verified
        }) {
            stop(
                state,
                db,
                &mut run,
                FailureKind::Authorization,
                "selected route is no longer authorized by the current resource configuration",
            )?;
            break;
        }
        config.harnesses.bind(&decision.selected, &resources);
        run.phase3.as_mut().unwrap().final_attempt_id = None;
        run.phase3.as_mut().unwrap().failure = None;
        let candidate = prepare(state, &mut run, decision.clone(), reason.take())?;
        run.status = RunStatus::Running;
        run.completed_at = None;
        run.outcome.lifecycle = LifecycleState::Working;
        run.outcome.work_result = WorkResult::Pending;
        run.outcome.review = ReviewState::NotRequested;
        run.outcome.waiting_on = WaitingOn::None;
        run.outcome.verification = VerificationState::NotRun;
        run.outcome.phase = RunPhase::Executing;
        db.sync_run(&run)?;
        let priority = run.phase3.as_ref().unwrap().priority;
        let permission = admit_attempt(
            state,
            db,
            &mut run,
            &config,
            &resources,
            &cancellation,
            output,
            priority,
        )
        .await;
        let guard = match permission {
            Ok(Some(guard)) => guard,
            other => {
                run.admission = db.admission_summary_for_run(&run.id)?;
                let failure =
                    if cancellation.is_cancelled() {
                        interrupted(&run)
                    } else {
                        run.phase3.as_ref().unwrap().failure.unwrap_or(
                            if matches!(other, Ok(None)) {
                                FailureKind::CapacityAdmission
                            } else {
                                FailureKind::InternalState
                            },
                        )
                    };
                let message = match other {
                    Err(e) => format!("{e:#}"),
                    _ => "capacity deferred".into(),
                };
                stop(state, db, &mut run, failure, &message)?;
                if failure == FailureKind::InternalState {
                    bail!(message);
                }
                break;
            }
        };
        #[cfg(test)]
        if CANCEL_AT_HANDOFF.try_with(|p| *p == 2).unwrap_or(false) {
            cancellation.cancel();
        }
        if cancellation.is_cancelled() {
            drop(guard);
            run.admission = db.admission_summary_for_run(&run.id)?;
            let failure = interrupted(&run);
            stop(state, db, &mut run, failure, "cancelled before handoff")?;
            bail!("cancelled before handoff");
        }
        let attempt_id = update_attempt_started(&mut run, &candidate.id);
        run.attempts.last_mut().unwrap().detail.admission =
            db.admission_summary_for_run(&run.id)?;
        run.attempts.last_mut().unwrap().detail.capacity = run.capacity.clone();
        db.sync_run(&run)?;
        let payload = serde_json::json!({"reason":run.attempts.last().unwrap().detail.reason});
        transition(state, db, &mut run, "attempt.started", payload)?;
        let prompt = fs::read_to_string(&candidate.prompt_path)?;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let (coordinator, token) = guard.handoff();
        let work = execute_candidate(
            candidate,
            prompt,
            config.clone(),
            run.baseline_path.clone(),
            cancellation.clone(),
            (tx, attempt_id),
            Some((coordinator, token, resources.capacity.heartbeat_secs)),
        );
        tokio::pin!(work);
        let execution = loop {
            tokio::select! {
                result=&mut work=>break result,
                Some(event)=rx.recv()=>{
                    if let Err(error)=persist_check_lifecycle(state,db,&mut run,event) {
                        cancellation.cancel(); let _=(&mut work).await;
                        return Err(error.context("failed to persist attempt verification"));
                    }
                }
            }
        };
        while let Ok(event) = rx.try_recv() {
            persist_check_lifecycle(state, db, &mut run, event)?;
        }
        run.admission = db.admission_summary_for_run(&run.id)?;
        if let Some(capacity) = &run.capacity {
            run.capacity = db.latest_capacity_observation(&capacity.pool_id)?;
            run.attempts.last_mut().unwrap().detail.capacity = run.capacity.clone();
        }
        let candidate = execution.candidate.clone();
        run.candidates = vec![candidate.clone()];
        refresh_outcome(&mut run);
        run.outcome.lifecycle = LifecycleState::Working;
        run.outcome.work_result = WorkResult::Pending;
        run.outcome.review = ReviewState::NotRequested;
        update_attempt_finished(&mut run, &candidate, &execution);
        let failure = if cancellation.is_cancelled() {
            Some(interrupted(&run))
        } else if !execution.admission_released {
            Some(FailureKind::InternalState)
        } else if candidate.status != CandidateStatus::Completed {
            Some(execution.failure.unwrap_or(FailureKind::HarnessProcess))
        } else if execution.checkpoint.as_ref().is_some_and(|r| r.is_err()) {
            Some(FailureKind::UnsupportedCheckpoint)
        } else if execution.checkpoint.is_some()
            && run.attempts.len() >= run.phase3.as_ref().unwrap().max_invocations.min(2) as usize
        {
            Some(FailureKind::InvocationLimit)
        } else {
            verification_failure(&run, &candidate)
        };
        let target_failure = failure == Some(FailureKind::TargetVerification);
        let last = run.attempts.last_mut().unwrap();
        last.detail.result = Some(candidate.clone());
        last.detail.admission = run.admission.clone();
        last.detail.failure = failure;
        let final_id = last.id.clone();
        // Persist completed evidence before any new attempt or question.
        db.sync_run(&run)?;
        transition(
            state,
            db,
            &mut run,
            "attempt.finished",
            serde_json::json!({"failure":failure}),
        )?;
        if let Some(failure) = failure.filter(|f| *f != FailureKind::TargetVerification) {
            stop(
                state,
                db,
                &mut run,
                failure,
                candidate
                    .error
                    .as_deref()
                    .unwrap_or("attempt did not safely complete"),
            )?;
            break;
        }
        if let Some(Ok(report)) = execution.checkpoint {
            let question = Clarification {
                id: Ulid::new().to_string(),
                run_id: run.id.clone(),
                attempt_id: final_id,
                generation: 1,
                revision: 1,
                report,
                state: QuestionState::Pending,
                answer: None,
                created_at: Utc::now(),
                resolved_at: None,
                actor_uid: None,
                actor: None,
            };
            run.phase3.as_mut().unwrap().questions.push(question);
            run.outcome.lifecycle = LifecycleState::Waiting;
            run.outcome.waiting_on = WaitingOn::Human;
            run.outcome.work_result = WorkResult::Pending;
            run.outcome.verification = VerificationState::NotRun;
            let payload =
                serde_json::json!({"question":run.phase3.as_ref().unwrap().questions.last()});
            transition(state, db, &mut run, "question.pending", payload)?;
            break;
        }
        let policy = run.phase3.as_mut().unwrap();
        policy.final_attempt_id = Some(final_id);
        policy.contributing_attempts = run.attempts.iter().map(|a| a.id.clone()).collect();
        if run.attempts.len() > 1 {
            policy.provenance = "mixed_attempt_context".into();
            let delivery = &mut run.candidates[0];
            delivery.harness_id = "mixed".into();
            delivery.harness_version = None;
            delivery.model = None;
            delivery.tokens = None;
            delivery.cost_usd = None;
            delivery.token_semantics = Some("see_per_attempt".into());
        }
        refresh_outcome(&mut run);
        run.phase3.as_mut().unwrap().failure = failure;
        if target_failure {
            resources = crate::commands::resources(state)?;
        }
        if target_failure && let Some(next) = recovery_route(state, &run, &resources, &config).await
        {
            run.outcome.lifecycle = LifecycleState::Preparing;
            run.outcome.work_result = WorkResult::Pending;
            run.outcome.review = ReviewState::NotRequested;
            run.outcome.phase = RunPhase::Preparing;
            transition(
                state,
                db,
                &mut run,
                "recovery.selected",
                serde_json::json!({"decision":next,"limit":2}),
            )?;
            decision = next;
            reason = Some("target_verification_failure".into());
            continue;
        }
        run.completed_at = Some(Utc::now());
        run.status = if run.outcome.work_result == WorkResult::Ready {
            RunStatus::ReadyForEvaluation
        } else {
            RunStatus::Failed
        };
        db.sync_run(&run)?;
        let payload = serde_json::json!({"outcome":run.outcome});
        transition(state, db, &mut run, "run.finished", payload)?;
        break;
    }
    emit(&run, output)?;
    Ok(run)
}

/// Local typed commands are authorized by the OS identity owning the run.
/// There is no caller-supplied actor/grant or remote transport in Phase 3.
#[derive(Debug)]
pub struct QuestionCommand {
    pub run_id: String,
    pub question_id: String,
    pub revision: u64,
    pub generation: u32,
}

fn resolve(
    state: &State,
    command: &QuestionCommand,
    answer: Option<String>,
) -> Result<(RunRecord, OperationLock, Option<OperationLock>)> {
    crate::commands::authorize_question_target(state, &command.run_id)?;
    let id = state.resolve_run_id(&command.run_id)?;
    let grant_lock = crate::commands::local_question_ownership(state, &id)?;
    let lock = OperationLock::acquire(
        &state.run_dir(&id).join(".operation.lock"),
        "run has a foreground owner",
    )?;
    let mut run = state.load_run(&id)?;
    if answer.is_some() {
        crate::commands::authorize_answer(state, &run, &command.question_id)?;
    }
    let policy = run
        .phase3
        .as_mut()
        .context("run has no clarification policy")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        anyhow::ensure!(
            fs::metadata(state.run_dir(&id))?.uid() == local_uid(),
            "local caller does not own the run"
        );
    }
    anyhow::ensure!(policy.owner_uid == local_uid(), "unauthorized local caller");
    anyhow::ensure!(
        run.outcome.lifecycle == LifecycleState::Waiting
            && run.outcome.waiting_on == WaitingOn::Human
            && run.outcome.review != ReviewState::Rejected
            && run.outcome.work_result != WorkResult::Cancelled,
        "run is not awaiting an answer"
    );
    let question = policy
        .questions
        .iter_mut()
        .find(|q| q.id == command.question_id)
        .context("wrong question")?;
    anyhow::ensure!(
        question.run_id == command.run_id
            && question.revision == command.revision
            && question.generation == command.generation
            && question.state == QuestionState::Pending,
        "stale, duplicate, or wrong-run question command"
    );
    if let Some(value) = &answer {
        anyhow::ensure!(
            !value.trim().is_empty() && value.len() <= 16_384,
            "invalid answer"
        );
    }
    question.state = if answer.is_some() {
        QuestionState::Answered
    } else {
        QuestionState::Cancelled
    };
    question.answer = answer;
    question.revision += 1;
    question.resolved_at = Some(Utc::now());
    question.actor_uid = Some(local_uid());
    question.actor = crate::commands::actor();
    run.outcome.waiting_on = WaitingOn::None;
    if question.state == QuestionState::Cancelled {
        run.outcome.lifecycle = LifecycleState::Finished;
        run.outcome.work_result = WorkResult::Cancelled;
        run.outcome.phase = RunPhase::Finished;
        run.status = RunStatus::Interrupted;
        run.completed_at = Some(Utc::now());
        policy.failure = Some(FailureKind::Cancelled);
    } else {
        run.outcome.lifecycle = LifecycleState::Preparing;
        run.outcome.phase = RunPhase::Preparing;
        policy.supervisor = Some(crate::admission::ProcessIdentity::current());
    }
    let db = Database::open(state.db_path())?;
    let result = transition(
        state,
        &db,
        &mut run,
        "question.resolved",
        serde_json::json!({"question_id":command.question_id,"revision":command.revision+1,"actor_uid":local_uid()}),
    );
    finish_error(state, &db, &run.id, result)?;
    Ok((run, lock, grant_lock))
}

pub async fn answer_question(
    state: &State,
    command: QuestionCommand,
    answer: String,
    output: RunOutputMode,
) -> Result<RunRecord> {
    RUN_OUTPUT_MODE.store(output.code(), Ordering::Relaxed);
    let (run, _lock, _grant_lock) = resolve(state, &command, Some(answer))?;
    let mut db = Database::open(state.db_path())?;
    let setup = (|| -> Result<_> {
        let config: Config = serde_yaml::from_str(&fs::read_to_string(
            state.run_dir(&run.id).join("config.snapshot.yml"),
        )?)?;
        config.validate()?;
        Ok((config, crate::commands::resources(state)?))
    })();
    let (config, resources) = finish_error(state, &db, &run.id, setup)?;
    let cancellation = operation_cancellation();
    let _signals = SignalListener::install(cancellation.clone());
    let _deadline = DeadlineGuard::new(run.phase3.as_ref(), cancellation.clone());
    drive(state, &mut db, run, config, resources, cancellation, output).await
}

pub fn cancel_question(
    state: &State,
    command: QuestionCommand,
    output: RunOutputMode,
) -> Result<RunRecord> {
    RUN_OUTPUT_MODE.store(output.code(), Ordering::Relaxed);
    let (run, _lock, _grant_lock) = resolve(state, &command, None)?;
    emit(&run, output)?;
    Ok(run)
}

/// Acquire an abandoned completed checkpoint, or continue its committed answer.
/// An interrupted invocation is never replayed.
pub(crate) async fn recover_checkpoint(
    state: &State,
    scope: &crate::commands::Scope,
    id: &str,
    revision: u64,
) -> Result<RunRecord> {
    scope.run(state, id)?; // Authorize before resolving or creating any lock path.
    let _lock = OperationLock::acquire(
        &state.run_dir(id).join(".operation.lock"),
        "run has a foreground owner",
    )?;
    let mut run = scope.run(state, id)?;
    crate::commands::ensure_recoverable(state, scope, &run, revision)?;
    let config: Config = serde_yaml::from_str(&fs::read_to_string(
        state.run_dir(id).join("config.snapshot.yml"),
    )?)?;
    config.validate()?;
    let resources = crate::commands::resources(state)?;
    run.phase3.as_mut().unwrap().supervisor = Some(crate::admission::ProcessIdentity::current());
    run.phase3.as_mut().unwrap().failure = None;
    let pending = run
        .phase3
        .as_ref()
        .unwrap()
        .questions
        .last()
        .is_some_and(|q| q.state == QuestionState::Pending);
    run.outcome.lifecycle = if pending {
        LifecycleState::Waiting
    } else {
        LifecycleState::Preparing
    };
    run.outcome.phase = RunPhase::Preparing;
    run.outcome.work_result = WorkResult::Pending;
    run.outcome.waiting_on = if pending {
        WaitingOn::Human
    } else {
        WaitingOn::None
    };
    run.status = RunStatus::Preparing;
    run.completed_at = None;
    let mut db = Database::open_control(state.db_path())?;
    transition(
        state,
        &db,
        &mut run,
        "run.recovered",
        serde_json::json!({"explicit":true}),
    )?;
    if pending {
        return Ok(run);
    }
    let cancellation = operation_cancellation();
    let _signals = SignalListener::install(cancellation.clone());
    let _deadline = DeadlineGuard::new(run.phase3.as_ref(), cancellation.clone());
    drive(
        state,
        &mut db,
        run,
        config,
        resources,
        cancellation,
        RunOutputMode::Silent,
    )
    .await
}
