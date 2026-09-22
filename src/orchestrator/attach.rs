//! `dispatch attach` (foreign form) and `dispatch finish`: attached external
//! work Dispatch did not select, route or execute, as a `RunRecord` with
//! `mode: Attached`. See `docs/plan-0.3-auto-apply-and-attach.md`, parts 6.2
//! and 14.3–14.9 (the design freeze this module implements exactly).
//!
//! The wrapped form (`dispatch attach -- <command...>`) is parsed but not run
//! yet; `create` refuses it. S4 implements that runtime.

use super::*;
use crate::{
    AttachCapabilities, AttachConfidence, AttachmentRecord, AttemptDetail, BaselineProvenance,
    FinishReason, OwnerState, admission,
};

/// Request to attach external work Dispatch did not launch. See part 14.3.
pub struct AttachRequest {
    /// Default: the current directory.
    pub workspace: PathBuf,
    /// Default: the repository's main worktree when `workspace` is a linked
    /// Git worktree; required for a plain directory.
    pub root: Option<PathBuf>,
    pub task: Option<String>,
    pub agent: Option<String>,
    pub pid: Option<u32>,
    /// `Some` selects the wrapped form, not yet implemented (S4).
    pub command: Option<Vec<String>>,
    pub allow_unsafe_local: bool,
    pub auto_apply: bool,
}

/// `dispatch attach --workspace <path> ...` (foreign form): create a
/// `RunMode::Attached` run observing an already-running agent, without
/// spawning anything or touching the workspace. See part 14.8 for the
/// refusal order and exact messages.
pub fn create(state: &State, request: AttachRequest) -> Result<RunRecord> {
    if request.command.is_some() {
        bail!(
            "wrapped attach is not available yet; use --workspace to attach an existing worktree"
        );
    }

    let workspace = source::resolve_source(Some(&request.workspace))?;
    let root = match &request.root {
        Some(root) => source::resolve_source(Some(root))?,
        None => match source::repo_identity(&workspace)? {
            Some(identity) => identity.main_worktree,
            None => bail!("--root is required for a plain directory"),
        },
    };

    // 14.8, in order: same-checkout, different repository, no merge base,
    // plain-directory foreign attach, checks without acknowledgement,
    // already attached.
    anyhow::ensure!(
        workspace != root,
        "attach needs a separate worktree; run git worktree add"
    );

    let workspace_identity = source::repo_identity(&workspace)?;
    let commit = match &workspace_identity {
        Some(workspace_repo) => {
            let root_identity = source::repo_identity(&root)?;
            anyhow::ensure!(
                root_identity.is_some_and(|root_repo| root_repo.key == workspace_repo.key),
                "workspace belongs to a different repository than the integration root"
            );
            source::merge_base(&root, &workspace)?
                .context("no common history between the workspace and the integration root")?
        }
        None => bail!("a plain directory can be attached only by wrapping the agent command"),
    };
    let workspace_identity = workspace_identity.expect("checked above: workspace is Git");

    let (config, config_path) = Config::discover(&root, None)?;
    config.validate()?;
    anyhow::ensure!(
        config.checks.verify.is_empty() || request.allow_unsafe_local,
        "finish runs your checks.verify on the host; pass --allow-unsafe-local"
    );
    let unsafe_local = request.allow_unsafe_local;

    if let Some(existing_id) = find_active_attachment(state, &workspace)? {
        bail!("workspace already attached as {existing_id}");
    }

    state.initialize()?;
    let mut db = Database::open(state.db_path())?;
    let run_id = Ulid::new().to_string();
    let run_dir = state.run_dir(&run_id);
    fs::create_dir(&run_dir)
        .with_context(|| format!("failed to create run directory {}", run_dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&run_dir, fs::Permissions::from_mode(0o700))?;
    }

    let task = request.task.clone().unwrap_or_else(|| {
        let basename = workspace
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| workspace.display().to_string());
        format!("attached work in {basename}")
    });
    write_text(&run_dir.join("task.md"), &task)?;
    write_text(
        &run_dir.join("config.snapshot.yml"),
        &serde_yaml::to_string(&config).context("failed to serialize effective configuration")?,
    )?;

    let snapshot = match source::materialize_baseline_from_commit(&root, &commit, &run_dir) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let _ = fs::remove_dir_all(&run_dir);
            return Err(error.context("baseline creation failed; incomplete run state was removed"));
        }
    };
    let provenance = BaselineProvenance::GitMergeBase {
        commit: commit.clone(),
    };
    let confidence = AttachConfidence::Full;

    let (source_kind, source_git_head) = source::inspect_source(&root)?;
    let source_fingerprint = source::fingerprint_tree(&root)?;

    let now = Utc::now();
    let candidate_id = Ulid::new().to_string();
    let harness_id = request.agent.clone().unwrap_or_else(|| "external".into());
    let diff_path = run_dir.join("delta.patch");
    let stdout_path = run_dir.join("attach-stdout.log");
    let stderr_path = run_dir.join("attach-stderr.log");
    let raw_telemetry_path = run_dir.join("harness.jsonl");
    for path in [&diff_path, &stdout_path, &stderr_path, &raw_telemetry_path] {
        write_text(path, "")?;
    }

    let candidate = CandidateRecord {
        id: candidate_id.clone(),
        label: "A".into(),
        harness_id: harness_id.clone(),
        harness_version: None,
        model: None,
        status: CandidateStatus::Running,
        workspace_path: workspace.clone(),
        prompt_path: run_dir.join("task.md"),
        stdout_path,
        stderr_path,
        diff_path,
        duration_ms: 0,
        exit_code: None,
        timed_out: false,
        tokens: None,
        token_semantics: None,
        cost_usd: None,
        error: None,
        diff_stats: DiffStats::default(),
        checks: Vec::new(),
    };

    let attempt = AttemptRecord {
        detail: AttemptDetail::default(),
        id: Ulid::new().to_string(),
        run_id: run_id.clone(),
        candidate_id: candidate_id.clone(),
        role: "attached".into(),
        ordinal: 1,
        generation: 1,
        harness_id,
        harness_version: None,
        requested_model: None,
        resolved_model: None,
        observed_model: None,
        requested_effort: None,
        resolved_effort: None,
        observed_effort: None,
        started_at: now,
        completed_at: None,
        outcome: "running".into(),
        raw_telemetry_path,
        resource: None,
    };

    let attachment = AttachmentRecord {
        version: 1,
        workspace: workspace.clone(),
        integration_root: root.clone(),
        repo_key: Some(workspace_identity.key),
        provenance,
        confidence,
        agent: request.agent.clone(),
        command: None,
        owner: None,
        agent_process: request.pid.map(admission::process_identity),
        owner_state: OwnerState::Unknown,
        capabilities: AttachCapabilities {
            observe: true,
            signal: true,
            control: false,
            integrate: request.auto_apply,
        },
        attached_at: now,
        finished_at: None,
        finish_reason: None,
    };

    let mut run = RunRecord {
        phase3: None,
        id: run_id.clone(),
        task: task.clone(),
        exact_prompt: task,
        source_path: root.clone(),
        source_kind,
        source_git_head,
        source_fingerprint,
        baseline_path: snapshot.baseline_path,
        baseline_commit: snapshot.baseline_commit,
        status: RunStatus::Running,
        mode: RunMode::Attached,
        state_revision: 0,
        outcome: RunOutcome {
            lifecycle: LifecycleState::Working,
            work_result: WorkResult::Pending,
            verification: VerificationState::NotRun,
            review: ReviewState::NotRequested,
            application: ApplicationState::NotApplied,
            phase: RunPhase::Executing,
            waiting_on: WaitingOn::None,
            applied_by: None,
            ..RunOutcome::default()
        },
        created_at: now,
        completed_at: None,
        environment: EnvironmentRecord {
            dispatch_version: crate::VERSION.into(),
            os: std::env::consts::OS.into(),
            architecture: std::env::consts::ARCH.into(),
            execution_backend: "local".into(),
            timeout_secs: config.execution.timeout_secs,
            cpus: config.execution.cpus,
            memory: config.execution.memory.clone(),
            max_parallel: config.execution.max_parallel,
            docker_image: None,
            resource_limits_enforced: false,
            unsafe_local,
            forwarded_env: config.execution.forwarded_env.clone(),
        },
        baseline_checks: Vec::new(),
        candidates: vec![candidate],
        attempts: vec![attempt],
        routing: None,
        allocation: None,
        capacity: None,
        admission: None,
        coherence: None,
        evaluation: None,
        applied_candidate: None,
        attachment: Some(attachment),
    };

    let created_event = EventRecord {
        run_id: run.id.clone(),
        candidate_label: None,
        event_type: "run.created".into(),
        timestamp: now,
        payload: serde_json::json!({
            "source": run.source_path,
            "source_kind": run.source_kind.as_str(),
            "config": config_path,
            "attachment": run.attachment,
        }),
        ..EventRecord::default()
    };
    let created = db.commit_run_transition(&mut run, created_event)?;
    publish_event(state, created, &run)?;

    persist_event(
        state,
        &db,
        EventRecord {
            run_id: run.id.clone(),
            candidate_label: None,
            event_type: "attach.created".into(),
            timestamp: Utc::now(),
            payload: serde_json::json!({"attachment": run.attachment}),
            ..EventRecord::default()
        },
        &mut run,
    )?;

    let provenance_text = match &run.attachment.as_ref().unwrap().provenance {
        BaselineProvenance::GitMergeBase { commit } => format!("commit {commit} (merge base)"),
        BaselineProvenance::SnapshotAtAttach => "snapshot at attach".to_owned(),
    };
    let confidence_text = match run.attachment.as_ref().unwrap().confidence {
        AttachConfidence::Full => "full",
        AttachConfidence::Partial => "partial",
    };
    println!("ATTACHED {}", run.id);
    println!("Workspace  {}", workspace.display());
    println!("Root       {}", root.display());
    println!("S0         {provenance_text}, {confidence_text} confidence");
    println!("Next: dispatch finish {} when the agent is done", run.id);

    Ok(run)
}

/// A `RunMode::Attached` run whose lifecycle is still `Working` for this
/// workspace, if one exists (part 14.8's "already attached" refusal and part
/// 6.9's reattach rule).
fn find_active_attachment(state: &State, workspace: &Path) -> Result<Option<String>> {
    for path in state.list_metadata_paths()? {
        let projected: RunRecord = serde_json::from_slice(&fs::read(&path)?)
            .with_context(|| format!("invalid metadata at {}", path.display()))?;
        let run = state.load_run(&projected.id)?;
        if run.mode == RunMode::Attached
            && run.outcome.lifecycle == LifecycleState::Working
            && run
                .attachment
                .as_ref()
                .is_some_and(|attachment| attachment.workspace == workspace)
        {
            return Ok(Some(run.id));
        }
    }
    Ok(None)
}

/// `dispatch finish <id>`: freeze Δ, run verification in the workspace
/// itself, and become an ordinary Ready result. See part 14.9.
pub async fn finish(state: &State, run_id: &str, allow_unsafe_local: bool) -> Result<RunRecord> {
    let resolved_run_id = state.resolve_run_id(run_id)?;
    let _run_lock = OperationLock::acquire(
        &state.run_dir(&resolved_run_id).join(".operation.lock"),
        "attached work has a foreground owner",
    )?;
    let mut run = state.load_run(&resolved_run_id)?;
    anyhow::ensure!(
        run.mode == RunMode::Attached && run.outcome.lifecycle == LifecycleState::Working,
        "attached work {} is not active",
        run.id
    );
    anyhow::ensure!(
        run.candidates.len() == 1,
        "attached work {} does not have exactly one candidate",
        run.id
    );

    let mut db = Database::open(state.db_path())?;
    let run_dir = state.run_dir(&run.id);
    let workspace = run.candidates[0].workspace_path.clone();
    let diff_path = run.candidates[0].diff_path.clone();

    let diff_stats = match source::collect_diff(&run.baseline_path, &workspace, &diff_path) {
        Ok(stats) => stats,
        Err(error) => {
            run.candidates[0].status = CandidateStatus::Failed;
            run.candidates[0].error = Some(format!("{error:#}"));
            refresh_outcome(&mut run);
            run.status = RunStatus::Failed;
            run.completed_at = Some(Utc::now());
            finish_attachment(&mut run, FinishReason::Explicit);
            db.sync_run(&run)?;
            persist_event(state, &db, run_finished_event(&run), &mut run)?;
            return Ok(run);
        }
    };
    run.candidates[0].diff_stats = diff_stats;

    let config = crate::coherence::run_config(&run_dir);
    let verify = config
        .as_ref()
        .map_or_else(Vec::new, |cfg| cfg.checks.verify.clone());
    if !verify.is_empty() {
        anyhow::ensure!(
            run.environment.unsafe_local || allow_unsafe_local,
            "finish runs your checks.verify on the host; pass --allow-unsafe-local"
        );
        let execution = config.map(|cfg| cfg.execution).unwrap_or_default();
        let results = crate::executor::run_checks_with_config(
            &workspace,
            &verify,
            CheckPhase::Verify,
            &run_dir.join("checks/verify"),
            execution,
        )
        .await;
        run.candidates[0].checks = results;
    }

    run.candidates[0].status = CandidateStatus::Completed;
    let attached_at = run
        .attachment
        .as_ref()
        .expect("attached run carries an attachment record")
        .attached_at;
    run.candidates[0].duration_ms = (Utc::now() - attached_at).num_milliseconds().max(0) as u64;
    run.candidates[0].exit_code = None;

    refresh_outcome(&mut run);
    run.status = if run.outcome.work_result == WorkResult::Ready {
        RunStatus::ReadyForEvaluation
    } else {
        RunStatus::Failed
    };
    run.completed_at = Some(Utc::now());
    let candidate_snapshot = run.candidates[0].clone();
    if let Some(attempt) = run.attempts.first_mut() {
        attempt.completed_at = run.completed_at;
        attempt.outcome = "completed".into();
        attempt.detail.result = Some(candidate_snapshot);
    }
    finish_attachment(&mut run, FinishReason::Explicit);

    db.sync_run(&run)?;
    persist_event(
        state,
        &db,
        EventRecord {
            run_id: run.id.clone(),
            candidate_label: None,
            event_type: "attach.finished".into(),
            timestamp: Utc::now(),
            payload: serde_json::json!({"reason": FinishReason::Explicit}),
            ..EventRecord::default()
        },
        &mut run,
    )?;
    persist_event(state, &db, run_finished_event(&run), &mut run)?;

    print_finish_summary(&run);
    println!("Next: dispatch check {}", run.id);

    Ok(run)
}

fn finish_attachment(run: &mut RunRecord, reason: FinishReason) {
    if let Some(attachment) = run.attachment.as_mut() {
        attachment.finished_at = Some(Utc::now());
        attachment.finish_reason = Some(reason);
    }
}

/// The same `run.finished` shape `run_dispatch` commits.
fn run_finished_event(run: &RunRecord) -> EventRecord {
    EventRecord {
        run_id: run.id.clone(),
        candidate_label: None,
        event_type: "run.finished".into(),
        timestamp: Utc::now(),
        payload: serde_json::json!({
            "candidate_count": run.candidates.len(),
            "outcome": run.outcome,
        }),
        ..EventRecord::default()
    }
}

/// A short equivalent of `print_single_result_summary` for attached work:
/// that function requires a routing or allocation decision, which attached
/// work never has.
fn print_finish_summary(run: &RunRecord) {
    let candidate = &run.candidates[0];
    let heading = if run.outcome.verification == VerificationState::Failed {
        "Verification failed"
    } else {
        "Ready for review"
    };
    println!("\n{heading}\n");
    println!("Task\n  {}\n", run.task);
    println!("Agent\n  {}\n", candidate.harness_id);
    println!("Verification\n  {}\n", result_verification(candidate));
    println!("Review\n  dispatch diff\n");
    println!("Then\n  dispatch accept\n  dispatch reject");
}
