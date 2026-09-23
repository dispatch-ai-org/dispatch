use super::*;
use crate::{
    FailureKind, ResourceTier,
    planning::{self, Artifact, Planning, Snapshot, Task, TaskState},
};

fn plan(run: &RunRecord) -> &Planning {
    run.phase3.as_ref().unwrap().planning.as_ref().unwrap()
}
fn plan_mut(run: &mut RunRecord) -> &mut Planning {
    run.phase3.as_mut().unwrap().planning.as_mut().unwrap()
}
fn event(state: &State, db: &Database, run: &mut RunRecord, kind: &str) -> Result<()> {
    phase3::transition(
        state,
        db,
        run,
        kind,
        serde_json::json!({"plan_revision":plan(run).revision}),
    )
}
fn fail(
    state: &State,
    db: &mut Database,
    run: &mut RunRecord,
    kind: FailureKind,
    message: &str,
) -> Result<()> {
    plan_mut(run).error = Some(message.into());
    run.candidates.clear();
    run.phase3.as_mut().unwrap().final_attempt_id = None;
    phase3::stop(state, db, run, kind, message)
}
fn boundary(run: &RunRecord, cancellation: &CancellationToken) -> Result<()> {
    anyhow::ensure!(
        !cancellation.is_cancelled() && Utc::now() < run.phase3.as_ref().unwrap().deadline_at,
        "goal cancelled or original deadline expired"
    );
    anyhow::ensure!(
        source::fingerprint_tree(&run.source_path)? == run.source_fingerprint,
        "source drift; planned work stopped"
    );
    Ok(())
}
fn has_extra(run: &RunRecord) -> bool {
    let missing_initial = plan(run)
        .tasks
        .iter()
        .filter(|t| {
            !run.attempts
                .iter()
                .any(|a| a.role == "task" && a.detail.task_id.as_deref() == Some(&t.id))
        })
        .count();
    !run.attempts
        .iter()
        .any(|a| matches!(a.role.as_str(), "repair" | "continuation"))
        && run.attempts.len() + missing_initial
            < run.phase3.as_ref().unwrap().max_invocations as usize
        && Utc::now() < run.phase3.as_ref().unwrap().deadline_at
}

async fn route(
    state: &State,
    run: &RunRecord,
    config: &Config,
    task: &planning::TaskSpec,
    minimum: ResourceTier,
) -> Result<AllocationDecision> {
    let resources = crate::commands::resources(state)?;
    let policy = run.phase3.as_ref().unwrap();
    let mut features = classify_task(&run.baseline_path, &task.objective)?;
    features.scope = if task.write.len() == 1 {
        crate::TaskScope::Localized
    } else {
        crate::TaskScope::MultiFile
    };
    let mut decision = select_available_resource(
        state,
        &run.source_path,
        &resources,
        &features,
        config,
        policy.fixed_harness.as_deref(),
        policy.fixed_model.as_deref(),
        policy.fixed_effort.as_deref(),
        Some(minimum),
    )
    .await?;
    decision.policy_version = "planned-contract-v1".into();
    decision.reason = "Validated child scope; owner risk/check mapping where configured. Planner labels do not establish suitability; private standalone policy is excluded.".into();
    Ok(decision)
}

/// One invocation through the existing admission, final fence, executor and cleanup.
/// The caller supplies an immutable task input, never another top-level goal.
#[allow(clippy::too_many_arguments)]
async fn invoke(
    state: &State,
    db: &mut Database,
    run: &mut RunRecord,
    config: &Config,
    cancellation: &CancellationToken,
    output: RunOutputMode,
    input: &Snapshot,
    decision: AllocationDecision,
    role: &str,
    task: Option<&str>,
    prompt: &str,
    consumed: Vec<String>,
) -> Result<CandidateExecution> {
    boundary(run, cancellation)?;
    planning::verify_snapshot(input)?;
    let resources = crate::commands::resources(state)?;
    let mut config = config.clone();
    config.checks.verify.clear(); // Checks run in a separate workspace with frozen verification files.
    config.harnesses.bind(&decision.selected, &resources);
    if role == "planner" {
        if decision.selected.harness == "claude" {
            config.harnesses.claude.read_only = true;
        } else {
            config.harnesses.codex.read_only = true;
        }
    }
    let candidate = phase3::prepare_input(state, run, decision, None, &input.path, prompt)?;
    let revision = plan(run).revision.clone();
    let attempt = run.attempts.last_mut().unwrap();
    attempt.role = role.into();
    attempt.detail.task_id = task.map(str::to_owned);
    attempt.detail.plan_revision = Some(revision);
    attempt.detail.input_snapshot_id = Some(input.id.clone());
    attempt.detail.consumed_artifacts = consumed;
    // Put actual identity in the report contract only after creating the attempt.
    let prompt = format!(
        "{prompt}\nReport identity: plan_revision={}, task_id={}, attempt_id={}, generation=1.\n",
        plan(run).revision,
        task.unwrap_or(if role == "planner" {
            "planner"
        } else {
            "integration"
        }),
        run.attempts.last().unwrap().id
    );
    write_text(&candidate.prompt_path, &prompt)?;
    run.candidates.clear();
    run.status = RunStatus::Running;
    run.completed_at = None;
    run.outcome.lifecycle = LifecycleState::Working;
    run.outcome.work_result = WorkResult::Pending;
    run.outcome.review = ReviewState::NotRequested;
    run.outcome.waiting_on = WaitingOn::None;
    run.outcome.verification = VerificationState::NotRun;
    run.outcome.phase = if role == "planner" {
        RunPhase::Planning
    } else {
        RunPhase::Executing
    };
    phase3::sync_transition(
        state,
        db,
        run,
        "attempt.prepared",
        serde_json::json!({"plan_revision":plan(run).revision}),
    )?;
    let priority = run.phase3.as_ref().unwrap().priority;
    let guard = admit_attempt(
        state,
        db,
        run,
        &config,
        &resources,
        cancellation,
        output,
        priority,
    )
    .await?
    .context("admission deferred; no automatic replay")?;
    boundary(run, cancellation)?;
    let id = update_attempt_started(run, &candidate.id);
    run.attempts.last_mut().unwrap().detail.admission = db.admission_summary_for_run(&run.id)?;
    run.attempts.last_mut().unwrap().detail.capacity = run.capacity.clone();
    phase3::sync_transition(
        state,
        db,
        run,
        "attempt.started",
        serde_json::json!({"plan_revision":plan(run).revision}),
    )?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let (coordinator, token) = guard.handoff();
    let work = execute_candidate(
        candidate,
        prompt,
        config,
        input.path.clone(),
        cancellation.clone(),
        (tx, id),
        Some((coordinator, token, resources.capacity.heartbeat_secs)),
    );
    tokio::pin!(work);
    let execution = loop {
        tokio::select! {
            result = &mut work => break result,
            Some(e) = rx.recv() => if let Err(error) = persist_check_lifecycle(state,db,run,e) { cancellation.cancel(); let _ = (&mut work).await; return Err(error); }
        }
    };
    while let Ok(e) = rx.try_recv() {
        persist_check_lifecycle(state, db, run, e)?;
    }
    run.admission = db.admission_summary_for_run(&run.id)?;
    update_attempt_finished(run, &execution.candidate, &execution);
    // Completion is not committed until checks and immutable output publication finish.
    // An interrupted boundary remains truthfully unfinished and is never replayed.
    Ok(execution)
}

fn finish_attempt(
    state: &State,
    db: &mut Database,
    run: &mut RunRecord,
    execution: &CandidateExecution,
    failure: Option<FailureKind>,
) -> Result<()> {
    let a = run.attempts.last_mut().unwrap();
    a.detail.result = Some(execution.candidate.clone());
    a.detail.failure = failure;
    a.detail.admission = run.admission.clone();
    run.candidates.clear();
    phase3::sync_transition(
        state,
        db,
        run,
        "attempt.finished",
        serde_json::json!({"plan_revision":plan(run).revision}),
    )
}

#[allow(clippy::too_many_arguments)]
async fn checks(
    state: &State,
    db: &Database,
    run: &mut RunRecord,
    config: &Config,
    snapshot: &Snapshot,
    commands: &[String],
    directory: &Path,
    cancellation: &CancellationToken,
) -> Result<Vec<crate::CheckResult>> {
    boundary(run, cancellation)?;
    planning::verify_snapshot(snapshot)?;
    let workspace =
        source::create_candidate_workspace(&snapshot.path, &directory.join("workspace"))?;
    source::restore_verification(
        &run.baseline_path,
        &workspace,
        &plan(run).owner_policy.verification_paths,
    )?;
    run.outcome.phase = RunPhase::Verifying;
    event(state, db, run, "planning.verification.started")?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let work = run_checks_with_observer(
        &workspace,
        commands,
        CheckPhase::Verify,
        directory,
        config.execution.clone(),
        cancellation.clone(),
        Some(tx),
        run.attempts.last().map(|a| a.id.clone()),
    );
    tokio::pin!(work);
    let results = loop {
        tokio::select! {
            result = &mut work => break result,
            Some(e) = rx.recv() => if let Err(error) = persist_check_lifecycle(state,db,run,e) { cancellation.cancel(); let _ = (&mut work).await; return Err(error); }
        }
    };
    while let Ok(e) = rx.try_recv() {
        persist_check_lifecycle(state, db, run, e)?;
    }
    boundary(run, cancellation)?;
    planning::verify_snapshot(snapshot)?;
    Ok(results)
}

fn passed(checks: &[crate::CheckResult]) -> bool {
    !checks.is_empty()
        && checks
            .iter()
            .all(|c| c.status == CheckStatus::Passed && c.exit_code == Some(0))
}
fn snapshot(
    source: &Path,
    directory: &Path,
    parent: Option<String>,
    contribution: Option<String>,
) -> Result<Snapshot> {
    let s = source::create_snapshot(source, directory)?;
    Ok(Snapshot {
        id: Ulid::new().to_string(),
        path: s.baseline_path,
        fingerprint: s.fingerprint,
        parent,
        contribution,
    })
}
fn execution_failure(
    run: &RunRecord,
    e: &CandidateExecution,
    cancellation: &CancellationToken,
) -> Option<FailureKind> {
    if cancellation.is_cancelled() {
        Some(phase3::interrupted(run))
    } else if !e.admission_released {
        Some(FailureKind::InternalState)
    } else if e.candidate.status != CandidateStatus::Completed {
        Some(e.failure.unwrap_or(FailureKind::HarnessProcess))
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn drive(
    state: &State,
    db: &mut Database,
    mut run: RunRecord,
    config: Config,
    _resources: crate::config::ResourceConfig,
    cancellation: CancellationToken,
    output: RunOutputMode,
) -> Result<RunRecord> {
    let result = drive_inner(state, db, &mut run, &config, &cancellation, output).await;
    if let Err(error) = result {
        // Read committed state before any terminal repair; never rewrite a committed artifact.
        if let Some(current) = db.committed_run_projection(&run.id)? {
            run = current;
        }
        let kind = if cancellation.is_cancelled()
            || Utc::now() >= run.phase3.as_ref().unwrap().deadline_at
        {
            phase3::interrupted(&run)
        } else {
            FailureKind::InternalState
        };
        fail(state, db, &mut run, kind, &format!("{error:#}"))?;
    }
    phase3::emit(&run, output)?;
    Ok(run)
}

async fn drive_inner(
    state: &State,
    db: &mut Database,
    run: &mut RunRecord,
    config: &Config,
    cancellation: &CancellationToken,
    output: RunOutputMode,
) -> Result<()> {
    boundary(run, cancellation)?;
    if plan(run).plan.is_none() {
        anyhow::ensure!(
            run.attempts.is_empty(),
            "the single planner cannot be replayed"
        );
        anyhow::ensure!(
            passed(&run.baseline_checks),
            "planned execution requires passing baseline checks; fix the verification contract before planning"
        );
        let input = plan(run).snapshots[0].clone();
        let prompt = format!(
            "Read-only planning. Do not edit source or implement work. Produce ONLY a final JSON envelope {{\"dispatch_plan\":{{\"version\":1,\"tasks\":[{{\"id\":\"task-name\",\"objective\":\"bounded objective\",\"read\":[\"file\"],\"write\":[\"file\"],\"acceptance\":[\"criterion\"],\"checks\":[\"approved check ID\"],\"prerequisites\":[],\"inputs\":[\"description\"],\"outputs\":[\"description\"]}}]}}}}. One to four tasks. All tasks required. No commands, grants, model choices, or changed limits. Order overlapping writes with dependencies. Independent branches still execute sequentially. Source scope is this frozen repository; no symlink scopes. One planner only; shared goal maximum {} supervised calls.\nOriginal goal (verbatim):\n{}\nApproved check catalog: {}\nOwner risk/verification policy: {}\nExplicit resource constraints: harness={:?}, model={:?}, effort={:?}.\n",
            run.phase3.as_ref().unwrap().max_invocations,
            run.task,
            serde_json::to_string(&plan(run).approved_checks)?,
            serde_json::to_string(&plan(run).owner_policy)?,
            run.phase3.as_ref().unwrap().fixed_harness,
            run.phase3.as_ref().unwrap().fixed_model,
            run.phase3.as_ref().unwrap().fixed_effort
        );
        let e = invoke(
            state,
            db,
            run,
            config,
            cancellation,
            output,
            &input,
            run.allocation.clone().unwrap(),
            "planner",
            None,
            &prompt,
            vec![],
        )
        .await?;
        let failure = execution_failure(run, &e, cancellation);
        finish_attempt(state, db, run, &e, failure)?;
        anyhow::ensure!(
            failure.is_none(),
            "planner failed; its invocation and evidence are retained; no second plan is permitted"
        );
        anyhow::ensure!(
            source::fingerprint_tree(&e.candidate.workspace_path)? == input.fingerprint,
            "planner edited source; edits were discarded from delivery and retained in its attempt"
        );
        let text = e.final_result.map_err(anyhow::Error::msg)?;
        let parsed = planning::parse(
            &text,
            &input.path,
            plan(run),
            run.phase3
                .as_ref()
                .unwrap()
                .max_invocations
                .saturating_sub(1),
        )?;
        let mut tasks = vec![];
        // Validate all resource requirements before launching any child.
        for id in planning::order(&parsed)? {
            let t = parsed.tasks.iter().find(|t| t.id == id).unwrap();
            let (lane, provenance) = planning::lane(t, &input.path, &plan(run).owner_policy);
            tasks.push(Task {
                id,
                state: TaskState::Pending,
                decision: route(state, run, config, t, lane).await?,
                feature_provenance: provenance,
                input: None,
                artifact: None,
                pending_prerequisite: None,
                extra_reason: None,
            });
        }
        plan_mut(run).plan = Some(parsed);
        plan_mut(run).tasks = tasks;
        event(state, db, run, "plan.validated")?;
    }
    loop {
        boundary(run, cancellation)?;
        if plan(run)
            .tasks
            .iter()
            .all(|t| t.state == TaskState::Integrated)
        {
            break;
        }
        let specs = plan(run).plan.as_ref().unwrap().clone();
        let index = plan(run).tasks.iter().enumerate().filter(|(_,t)| matches!(t.state,TaskState::Pending | TaskState::Waiting)).filter(|(_,t)| {
            let spec = specs.tasks.iter().find(|s| s.id == t.id).unwrap();
            spec.prerequisites.iter().chain(t.pending_prerequisite.iter()).all(|p| plan(run).tasks.iter().any(|prior| &prior.id == p && prior.state == TaskState::Integrated && prior.artifact.is_some()))
        }).min_by_key(|(_,t)| &t.id).map(|(i,_)|i).context("unfinished tasks have no satisfiable prerequisites; partial contributions are retained")?;
        let task = plan(run).tasks[index].clone();
        let spec = specs
            .tasks
            .iter()
            .find(|t| t.id == task.id)
            .unwrap()
            .clone();
        let continuation = task.state == TaskState::Waiting;
        let role = if continuation {
            "continuation"
        } else if task.extra_reason.is_some() {
            "repair"
        } else {
            "task"
        };
        if role != "task" {
            anyhow::ensure!(
                has_extra(run),
                "shared extra invocation already spent or unavailable"
            );
        }
        if continuation
            && task.pending_prerequisite.is_none()
            && task.extra_reason.as_deref() != Some("reasoning_assistance")
        {
            anyhow::ensure!(
                run.phase3
                    .as_ref()
                    .unwrap()
                    .questions
                    .last()
                    .is_some_and(|q| q.state == crate::QuestionState::Answered),
                "task requires an authorized answer"
            );
        }
        let mut consumed = vec![];
        for id in spec
            .prerequisites
            .iter()
            .chain(task.pending_prerequisite.iter())
        {
            let t = plan(run)
                .tasks
                .iter()
                .find(|t| &t.id == id)
                .context("prerequisite missing")?;
            let artifact = plan(run)
                .artifacts
                .iter()
                .find(|a| Some(&a.id) == t.artifact.as_ref())
                .context("prerequisite has no immutable artifact")?;
            planning::verify_artifact(run, artifact)?;
            consumed.push(artifact.id.clone());
        }
        let input = if role == "repair" {
            plan(run)
                .snapshots
                .iter()
                .find(|s| Some(&s.id) == task.input.as_ref())
                .context("repair input missing")?
                .clone()
        } else {
            plan(run).snapshots.last().unwrap().clone()
        };
        planning::verify_snapshot(&input)?;
        let (lane, _) = planning::lane(&spec, &run.baseline_path, &plan(run).owner_policy);
        let decision = route(
            state,
            run,
            config,
            &spec,
            if role == "repair" {
                ResourceTier::Strong
            } else {
                lane
            },
        )
        .await?;
        plan_mut(run).tasks[index].decision = decision.clone();
        plan_mut(run).tasks[index].input = Some(input.id.clone());
        plan_mut(run).tasks[index].state = TaskState::Working;
        event(state, db, run, "task.runnable")?;
        let mut prompt = format!(
            "Implement only this validated task. Original goal (verbatim):\n{}\nTask contract: {}\nRequired checked artifacts: {}\nInput snapshot: {}. Incidental prior contributions are context, not declared dependencies.\nNo new tasks, replanning, funding or permission changes. If essential clarification is needed emit final {{\"dispatch_checkpoint\":{{\"version\":1,\"question\":\"...\",\"choices\":[],\"category\":\"factual\"}}}}. For a missing existing task or reasoning assistance emit final {{\"dispatch_dependency\":{{\"version\":1,\"plan_revision\":\"identity below\",\"task_id\":\"identity below\",\"attempt_id\":\"identity below\",\"generation\":1,\"prerequisite\":\"existing task ID or null\",\"assistance\":null}}}}. Exactly one of prerequisite/assistance; assistance is a bounded reasoning request. Both consume the goal's one shared continuation slot. Otherwise finish normally.\n",
            run.task,
            serde_json::to_string(&spec)?,
            serde_json::to_string(&consumed)?,
            input.id
        );
        if let Some(parent) = run
            .attempts
            .iter()
            .rev()
            .find(|a| a.detail.task_id.as_deref() == Some(&spec.id))
        {
            prompt.push_str(
                &phase3::diagnostics(parent)
                    .replace("same original baseline", "task's recorded immutable input"),
            );
            if continuation {
                prompt.push_str("\nCheckpoint changes are retained in the previous diff but are not copied over the new prerequisite. Reconcile the requested task against this new input explicitly.\n");
            }
            if let Some(c) = &parent.detail.result {
                let bytes = fs::read(&c.diff_path)?;
                prompt.push_str(&format!(
                    "\nBounded previous contribution (may be truncated):\n{}",
                    String::from_utf8_lossy(&bytes[..bytes.len().min(8192)])
                ));
            }
        }
        if continuation
            && let Some(q) = run
                .phase3
                .as_ref()
                .unwrap()
                .questions
                .last()
                .filter(|q| q.state == crate::QuestionState::Answered)
        {
            prompt.push_str(&format!(
                "\nAuthorized answer: {}\n",
                q.answer.as_deref().unwrap_or_default()
            ));
        }
        let mut e = invoke(
            state,
            db,
            run,
            config,
            cancellation,
            output,
            &input,
            decision,
            role,
            Some(&spec.id),
            &prompt,
            consumed,
        )
        .await?;
        if let Some(failure) = execution_failure(run, &e, cancellation) {
            finish_attempt(state, db, run, &e, Some(failure))?;
            fail(
                state,
                db,
                run,
                failure,
                "child invocation did not safely complete; unrelated children were not launched",
            )?;
            return Ok(());
        }
        let scoped = validate_writes(&spec, &input.path, &e.candidate);
        if let Err(error) = scoped {
            finish_attempt(state, db, run, &e, Some(FailureKind::Authorization))?;
            return Err(error);
        }
        if e.checkpoint.is_some()
            || e.final_result
                .as_ref()
                .is_ok_and(|s| s.contains("dispatch_dependency"))
        {
            let report = e.checkpoint.clone();
            let text = e.final_result.clone();
            finish_attempt(state, db, run, &e, None)?;
            anyhow::ensure!(
                has_extra(run),
                "checkpoint cannot continue: goal's shared extra is unavailable"
            );
            plan_mut(run).tasks[index].state = TaskState::Waiting;
            if let Some(report) = report {
                let report = report.map_err(anyhow::Error::msg)?;
                let q = crate::Clarification {
                    id: Ulid::new().to_string(),
                    run_id: run.id.clone(),
                    attempt_id: run.attempts.last().unwrap().id.clone(),
                    generation: 1,
                    revision: 1,
                    report,
                    state: crate::QuestionState::Pending,
                    answer: None,
                    created_at: Utc::now(),
                    resolved_at: None,
                    actor_uid: None,
                    actor: None,
                };
                run.phase3.as_mut().unwrap().questions.push(q);
                run.outcome.lifecycle = LifecycleState::Waiting;
                run.outcome.waiting_on = WaitingOn::Human;
                event(state, db, run, "question.pending")?;
                return Ok(());
            }
            dependency(run, index, &text.map_err(anyhow::Error::msg)?)?;
            run.outcome.waiting_on = WaitingOn::Dependency;
            event(state, db, run, "dependency.recorded")?;
            continue;
        }
        let attempt_dir = e.candidate.prompt_path.parent().unwrap().to_path_buf();
        let published = snapshot(
            &e.candidate.workspace_path,
            &attempt_dir.join("output"),
            Some(input.id.clone()),
            Some(e.candidate.id.clone()),
        )?;
        let commands = spec
            .checks
            .iter()
            .map(|c| plan(run).approved_checks[c].clone())
            .collect::<Vec<_>>();
        e.candidate.checks = checks(
            state,
            db,
            run,
            config,
            &published,
            &commands,
            &attempt_dir.join("verification"),
            cancellation,
        )
        .await?;
        if !passed(&e.candidate.checks) {
            let mut comparison = run.clone();
            comparison.baseline_checks = checks(
                state,
                db,
                run,
                config,
                &input,
                &commands,
                &attempt_dir.join("input-verification"),
                cancellation,
            )
            .await?;
            let failure = phase3::verification_failure(&comparison, &e.candidate)
                .unwrap_or(FailureKind::VerificationUnknown);
            finish_attempt(state, db, run, &e, Some(failure))?;
            if failure == FailureKind::TargetVerification
                && !run.phase3.as_ref().unwrap().no_retry
                && has_extra(run)
            {
                plan_mut(run).tasks[index].state = TaskState::Pending;
                plan_mut(run).tasks[index].extra_reason =
                    Some("target_verification_failure".into());
                event(state, db, run, "recovery.selected")?;
                continue;
            }
            fail(
                state,
                db,
                run,
                failure,
                "child checks failed; shared recovery is unavailable or inappropriate",
            )?;
            return Ok(());
        }
        finish_attempt(state, db, run, &e, None)?;
        let artifact_id = integrate(
            state,
            db,
            run,
            cancellation,
            &input,
            &published,
            &e,
            Some(&spec.id),
        )?;
        plan_mut(run).tasks[index].artifact = Some(artifact_id);
        plan_mut(run).tasks[index].state = TaskState::Integrated;
        event(state, db, run, "task.integrated")?;
    }
    deliver(state, db, run, config, cancellation).await
}

#[allow(clippy::too_many_arguments)]
fn integrate(
    state: &State,
    db: &Database,
    run: &mut RunRecord,
    cancellation: &CancellationToken,
    input: &Snapshot,
    published: &Snapshot,
    e: &CandidateExecution,
    task: Option<&str>,
) -> Result<String> {
    let attempt_dir = e.candidate.prompt_path.parent().unwrap().to_path_buf();
    boundary(run, cancellation)?;
    run.outcome.phase = RunPhase::Integrating;
    event(state, db, run, "integration.started")?;
    anyhow::ensure!(
        plan(run).snapshots.last().unwrap().id == input.id,
        "integration input revision changed; stale contribution refused"
    );
    planning::verify_snapshot(input)?;
    let integrated = source::integrate_snapshot(
        &input.path,
        &e.candidate.diff_path,
        &attempt_dir.join("integration"),
    )?;
    anyhow::ensure!(
        integrated.fingerprint == published.fingerprint,
        "contribution cannot faithfully represent output tree; integration refused"
    );
    let artifact_id = Ulid::new().to_string();
    let output_snapshot = Snapshot {
        id: Ulid::new().to_string(),
        path: integrated.baseline_path,
        fingerprint: integrated.fingerprint,
        parent: Some(input.id.clone()),
        contribution: Some(artifact_id.clone()),
    };
    let artifact = Artifact {
        id: artifact_id.clone(),
        task_id: task.map(str::to_owned),
        attempt_id: run.attempts.last().unwrap().id.clone(),
        input: input.id.clone(),
        output: output_snapshot.id.clone(),
        patch: e.candidate.diff_path.clone(),
        patch_sha256: hex::encode(Sha256::digest(fs::read(&e.candidate.diff_path)?)),
        manifest: attempt_dir.join("artifact.json"),
        checks: e.candidate.checks.clone(),
        verification_changes: e
            .candidate
            .diff_stats
            .changed_files
            .iter()
            .filter(|f| {
                plan(run)
                    .owner_policy
                    .verification_paths
                    .iter()
                    .any(|p| planning::contains(p, f))
            })
            .cloned()
            .collect(),
    };
    planning::publish(&artifact.manifest, &serde_json::to_vec_pretty(&artifact)?)?;
    boundary(run, cancellation)?;
    plan_mut(run).snapshots.push(output_snapshot);
    plan_mut(run).artifacts.push(artifact);
    Ok(artifact_id)
}

fn validate_writes(
    spec: &planning::TaskSpec,
    input: &Path,
    candidate: &CandidateRecord,
) -> Result<()> {
    for file in &candidate.diff_stats.changed_files {
        anyhow::ensure!(
            spec.write.iter().any(|s| planning::contains(s, file)),
            "out-of-scope write: {file}"
        );
        planning::path(input, file)?;
        planning::path(&candidate.workspace_path, file)?;
    }
    Ok(())
}

fn dependency(run: &mut RunRecord, index: usize, text: &str) -> Result<()> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Envelope {
        dispatch_dependency: planning::DependencyReport,
    }
    let r = serde_json::from_str::<Envelope>(text)?.dispatch_dependency;
    let attempt = run.attempts.last().unwrap();
    let id = plan(run).tasks[index].id.clone();
    anyhow::ensure!(
        r.version == 1
            && r.plan_revision == plan(run).revision
            && r.task_id == id
            && r.attempt_id == attempt.id
            && r.generation == attempt.generation,
        "stale or cross-goal dependency report"
    );
    anyhow::ensure!(
        r.prerequisite.is_some() != r.assistance.is_some(),
        "one dependency or assistance request required"
    );
    if let Some(target) = r.prerequisite {
        anyhow::ensure!(target != id, "self dependency refused");
        let prerequisite = plan(run)
            .tasks
            .iter()
            .find(|t| t.id == target)
            .context("unknown prerequisite; new tasks are forbidden")?;
        anyhow::ensure!(
            matches!(
                prerequisite.state,
                TaskState::Pending | TaskState::Integrated
            ),
            "prerequisite is failed, cancelled, or itself awaiting continuation"
        );
        let p = plan(run).plan.as_ref().unwrap();
        anyhow::ensure!(
            !planning::depends(p, &target, &id),
            "dependency would create a cycle"
        );
        let producer = p.tasks.iter().find(|t| t.id == target).unwrap();
        let consumer = p.tasks.iter().find(|t| t.id == id).unwrap();
        anyhow::ensure!(
            producer.write.iter().any(|w| consumer
                .read
                .iter()
                .any(|r| planning::contains(r, w) || planning::contains(w, r))),
            "dependency is outside declared read scope"
        );
        // Completed downstream branches cannot be reinterpreted by a new edge.
        anyhow::ensure!(
            !plan(run)
                .tasks
                .iter()
                .any(|t| t.state == TaskState::Integrated && planning::depends(p, &t.id, &id)),
            "completed branch mutation refused"
        );
        plan_mut(run).tasks[index].pending_prerequisite = Some(target);
    } else {
        anyhow::ensure!(
            !run.phase3.as_ref().unwrap().no_retry,
            "automatic reasoning assistance is disabled by --no-retry"
        );
        let assistance = r.assistance.unwrap();
        anyhow::ensure!(
            !assistance.trim().is_empty() && assistance.len() <= 2048,
            "invalid assistance request"
        );
        plan_mut(run).tasks[index].extra_reason = Some("reasoning_assistance".into());
    }
    Ok(())
}

async fn deliver(
    state: &State,
    db: &mut Database,
    run: &mut RunRecord,
    config: &Config,
    cancellation: &CancellationToken,
) -> Result<()> {
    for a in &plan(run).artifacts {
        planning::verify_artifact(run, a)?;
    }
    let mut input = plan(run).snapshots.last().unwrap().clone();
    let dir = state.run_dir(&run.id).join("delivery");
    let mut root_checks = checks(
        state,
        db,
        run,
        config,
        &input,
        &config.checks.verify,
        &dir.join("checks"),
        cancellation,
    )
    .await?;
    plan_mut(run).root_checks = root_checks.clone();
    if !passed(&root_checks) {
        let mut target = run
            .attempts
            .last()
            .and_then(|a| a.detail.result.clone())
            .context("root verification lacks a contribution")?;
        target.checks = root_checks.clone();
        let failure =
            phase3::verification_failure(run, &target).unwrap_or(FailureKind::VerificationUnknown);
        if failure != FailureKind::TargetVerification
            || run.phase3.as_ref().unwrap().no_retry
            || !has_extra(run)
        {
            run.outcome.verification = if failure == FailureKind::TargetVerification {
                VerificationState::Failed
            } else {
                VerificationState::Inconclusive
            };
            fail(
                state,
                db,
                run,
                failure,
                "root verification failed; no eligible shared repair remains",
            )?;
            return Ok(());
        }
        let specs = &plan(run).plan.as_ref().unwrap().tasks;
        let spec = planning::TaskSpec {
            id: "integration".into(),
            objective: "Repair the integrated goal against the original approved root checks"
                .into(),
            read: specs.iter().flat_map(|t| t.read.clone()).collect(),
            write: specs.iter().flat_map(|t| t.write.clone()).collect(),
            acceptance: vec![run.task.clone()],
            checks: plan(run).approved_checks.keys().cloned().collect(),
            prerequisites: vec![],
            inputs: vec![input.id.clone()],
            outputs: vec!["Repaired integrated contribution".into()],
        };
        let decision = route(state, run, config, &spec, ResourceTier::Strong).await?;
        let prompt = format!(
            "One shared integration repair. Do not plan or add tasks. Original goal: {}\nAllowed contract: {}\nRoot checks failed: {}\nInput is the exact integrated snapshot; preserve prior contributions and original verification authority.",
            run.task,
            serde_json::to_string(&spec)?,
            serde_json::to_string(
                &root_checks
                    .iter()
                    .map(|c| (&c.command, &c.status, c.exit_code))
                    .collect::<Vec<_>>()
            )?
        );
        event(state, db, run, "recovery.selected")?;
        let consumed = plan(run).artifacts.iter().map(|a| a.id.clone()).collect();
        let mut e = invoke(
            state,
            db,
            run,
            config,
            cancellation,
            RunOutputMode::Silent,
            &input,
            decision,
            "repair",
            None,
            &prompt,
            consumed,
        )
        .await?;
        if let Some(failure) = execution_failure(run, &e, cancellation) {
            finish_attempt(state, db, run, &e, Some(failure))?;
            fail(state, db, run, failure, "integration repair failed")?;
            return Ok(());
        }
        if let Err(error) = validate_writes(&spec, &input.path, &e.candidate) {
            finish_attempt(state, db, run, &e, Some(FailureKind::Authorization))?;
            return Err(error);
        }
        if e.checkpoint.is_some()
            || e.final_result
                .as_ref()
                .is_ok_and(|s| s.contains("dispatch_dependency"))
        {
            finish_attempt(state, db, run, &e, Some(FailureKind::InvocationLimit))?;
            fail(
                state,
                db,
                run,
                FailureKind::InvocationLimit,
                "integration repair cannot request another continuation",
            )?;
            return Ok(());
        }
        let attempt_dir = e.candidate.prompt_path.parent().unwrap().to_path_buf();
        let published = snapshot(
            &e.candidate.workspace_path,
            &attempt_dir.join("output"),
            Some(input.id.clone()),
            Some(e.candidate.id.clone()),
        )?;
        e.candidate.checks = checks(
            state,
            db,
            run,
            config,
            &published,
            &config.checks.verify,
            &attempt_dir.join("verification"),
            cancellation,
        )
        .await?;
        let passed = passed(&e.candidate.checks);
        let failure = (!passed).then(|| {
            phase3::verification_failure(run, &e.candidate)
                .unwrap_or(FailureKind::VerificationUnknown)
        });
        finish_attempt(state, db, run, &e, failure)?;
        plan_mut(run).root_checks = e.candidate.checks.clone();
        if !passed {
            fail(
                state,
                db,
                run,
                failure.unwrap(),
                "shared integration repair did not pass root checks",
            )?;
            return Ok(());
        }
        integrate(state, db, run, cancellation, &input, &published, &e, None)?;
        event(state, db, run, "integration.repaired")?;
        input = plan(run).snapshots.last().unwrap().clone();
        // Verify again on the published integrated tree, never just the repair workspace.
        root_checks = checks(
            state,
            db,
            run,
            config,
            &input,
            &config.checks.verify,
            &dir.join("repaired-checks"),
            cancellation,
        )
        .await?;
        plan_mut(run).root_checks = root_checks.clone();
        if !self::passed(&root_checks) {
            e.candidate.checks = root_checks;
            let failure = phase3::verification_failure(run, &e.candidate)
                .unwrap_or(FailureKind::VerificationUnknown);
            fail(
                state,
                db,
                run,
                failure,
                "published repaired integration failed root checks",
            )?;
            return Ok(());
        }
    }
    boundary(run, cancellation)?;
    let mut candidate = run
        .attempts
        .iter()
        .rev()
        .find_map(|a| a.detail.result.clone())
        .context("no delivery contribution")?;
    candidate.id = Ulid::new().to_string();
    candidate.workspace_path = input.path.clone();
    candidate.diff_path = dir.join("diff.patch");
    candidate.diff_stats =
        source::collect_diff(&run.baseline_path, &input.path, &candidate.diff_path)?;
    candidate.harness_id = "mixed".into();
    candidate.harness_version = None;
    candidate.model = None;
    candidate.tokens = None;
    candidate.cost_usd = None;
    candidate.token_semantics = Some("see_per_attempt".into());
    candidate.checks = root_checks;
    candidate.status = CandidateStatus::Completed;
    candidate.duration_ms = run
        .attempts
        .iter()
        .filter_map(|a| a.detail.result.as_ref())
        .map(|c| c.duration_ms)
        .sum();
    plan_mut(run).final_candidate = Some(candidate.id.clone());
    plan_mut(run).final_patch_sha256 =
        Some(hex::encode(Sha256::digest(fs::read(&candidate.diff_path)?)));
    let contributors = plan(run)
        .artifacts
        .iter()
        .map(|a| a.attempt_id.clone())
        .collect();
    let policy = run.phase3.as_mut().unwrap();
    policy.contributing_attempts = contributors;
    policy.final_attempt_id = run.attempts.last().map(|a| a.id.clone());
    policy.failure = None;
    run.candidates = vec![candidate];
    refresh_outcome(run);
    run.completed_at = Some(Utc::now());
    run.status = RunStatus::ReadyForEvaluation;
    phase3::sync_transition(
        state,
        db,
        run,
        "run.finished",
        serde_json::json!({"plan_revision":plan(run).revision}),
    )
}
