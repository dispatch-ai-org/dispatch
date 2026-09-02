use std::{
    collections::{HashSet, VecDeque},
    ffi::OsString,
    fs,
    future::Future,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    pin::Pin,
    process::Command,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use futures::{StreamExt, stream::FuturesUnordered};
use rand::seq::SliceRandom;
use sha2::{Digest, Sha256};
use ulid::Ulid;

use crate::{
    CandidateRecord, CandidateStatus, CheckPhase, CheckStatus, Config, DiffStats,
    EnvironmentRecord, EvaluationOutcome, EvaluationRecord, EventRecord, RoutingDecision,
    RunRecord, RunStatus, VERSION,
    classifier::classify_task,
    db::Database,
    executor::{
        CancellationToken, ExecutionStatus, Executor, run_checks_with_config_and_cancel,
        trusted_host_executable,
    },
    harness::{
        HarnessRunRequest, SUPPORTED_HARNESSES, adapter_for, build_prompt, probe_version,
        run_harness,
    },
    router::rank_harnesses,
    source,
    state::{State, write_text},
};

const EVALUATION_REASONS: &[&str] = &[
    "correctness",
    "completeness",
    "architecture",
    "maintainability",
    "readability",
    "tests",
    "edge-cases",
    "cleaner-change",
    "performance",
    "cost",
    "latency",
    "other",
];

pub struct RunRequest {
    pub source: PathBuf,
    pub task: String,
    pub harnesses: Vec<String>,
    pub route: bool,
    pub config_path: Option<PathBuf>,
    pub backend: Option<String>,
    pub timeout_secs: Option<u64>,
    pub max_parallel: Option<usize>,
    pub allow_unsafe_local: bool,
    pub allow_forwarded_env: bool,
}

#[derive(Default)]
pub struct EvaluationInput {
    pub winner: Option<String>,
    pub reasons: Vec<String>,
    pub explanation: Option<String>,
}

pub fn init(state: &State, path: &Path, force: bool) -> Result<()> {
    let project = if path.exists() {
        path.canonicalize()
            .with_context(|| format!("failed to resolve {}", path.display()))?
    } else {
        fs::create_dir_all(path)?;
        path.canonicalize()?
    };
    let projected_state_root = canonicalize_allow_missing(&state.root)?;
    anyhow::ensure!(
        !projected_state_root.starts_with(&project),
        "Dispatch state directory must be outside the project tree: {}",
        state.root.display()
    );
    let config_path = project.join("dispatch.yml");
    write_project_config(&config_path, Config::example_yaml(), force)?;
    state.initialize()?;
    Database::open(state.db_path())?;
    println!("Created {}", config_path.display());
    println!("State directory: {}", state.root.display());
    Ok(())
}

fn write_project_config(path: &Path, contents: &str, force: bool) -> Result<()> {
    let existing = match fs::symlink_metadata(path) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(error).with_context(|| format!("failed to inspect {}", path.display()));
        }
    };
    if let Some(metadata) = &existing {
        anyhow::ensure!(
            !metadata.file_type().is_symlink(),
            "refusing to write configuration through symlink {}",
            path.display()
        );
        anyhow::ensure!(
            metadata.is_file(),
            "{} is not a regular file",
            path.display()
        );
        if !force {
            bail!(
                "{} already exists (use --force to replace it)",
                path.display()
            );
        }
    }

    if existing.is_none() {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .with_context(|| format!("failed to create {}", path.display()))?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        return Ok(());
    }

    let parent = path.parent().context("configuration path has no parent")?;
    let temp = parent.join(format!(".dispatch-config-{}.tmp", Ulid::new()));
    let replacement = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp, path).with_context(|| format!("failed to replace {}", path.display()))?;
        Ok(())
    })();
    if replacement.is_err() {
        let _ = fs::remove_file(&temp);
    }
    replacement
}

pub async fn doctor(state: &State, source_path: &Path, config_path: Option<&Path>) -> Result<()> {
    let source = source::resolve_source(Some(source_path))?;
    let projected_state_root = canonicalize_allow_missing(&state.root)?;
    anyhow::ensure!(
        !projected_state_root.starts_with(&source),
        "Dispatch state directory must be outside the source tree: {}",
        state.root.display()
    );
    let (config, discovered) = Config::discover(&source, config_path)?;
    config.validate()?;
    state.initialize()?;
    let db = Database::open(state.db_path())?;
    db.health_check()?;

    println!("Dispatch doctor\n");
    print_detection("Git", &command_version("git", &["--version"]));
    print_detection("Docker", &command_version("docker", &["--version"]));
    println!(
        "{:<17} ✓ bundled (schema {})",
        "SQLite",
        db.schema_version()?
    );
    println!();

    let harnesses = [
        ("Claude Code", "claude"),
        ("Codex", "codex"),
        ("Cursor", "cursor"),
    ];
    let mut ready = 0usize;
    for (label, harness_id) in harnesses {
        let adapter = adapter_for(harness_id, &config.harnesses)?;
        let detection = adapter.detect().await;
        let detail = if detection.available {
            ready += 1;
            let custom = match harness_id {
                "claude" => config.harnesses.claude.executable.is_some(),
                "codex" => config.harnesses.codex.executable.is_some(),
                "cursor" => config.harnesses.cursor.executable.is_some(),
                _ => false,
            };
            match detection.executable {
                Some(path) if !custom && !path.starts_with(&source) => probe_version(&path)
                    .await
                    .ok()
                    .flatten()
                    .or_else(|| Some(path.display().to_string())),
                Some(path) => Some(format!(
                    "{} (version probe skipped for untrusted path)",
                    path.display()
                )),
                None => None,
            }
        } else {
            None
        };
        print_detection(label, &detail);
    }
    println!("{:<17} ✓ built in", "Fake harnesses");

    let (kind, head) = source::inspect_source(&source)?;
    println!();
    println!("{:<17} {}", "Source", source.display());
    println!("{:<17} {}", "Source type", kind.as_str());
    if let Some(head) = head {
        println!("{:<17} {}", "Git HEAD", head);
    }
    println!(
        "{:<17} {}",
        "Configuration",
        discovered
            .as_ref()
            .map_or_else(|| "defaults".into(), |p| p.display().to_string())
    );
    println!(
        "{:<17} {} baseline / {} verify",
        "Checks",
        config.checks.baseline.len(),
        config.checks.verify.len()
    );
    println!("{:<17} {}", "Backend", config.execution.backend);
    println!("{:<17} ✓ ready", "Snapshotting");
    println!(
        "{:<17} {}",
        "Verification",
        if config.checks.verify.is_empty() {
            "not configured"
        } else {
            "configured"
        }
    );
    println!();
    if config.execution.backend == "local" {
        println!(
            "Found {ready} installed real harness(es). Real harnesses and project checks are blocked on the host unless each run explicitly uses --allow-unsafe-local."
        );
        println!("Deterministic fake harnesses are ready without that opt-in.");
    } else {
        println!(
            "Docker isolation is selected. Docker daemon access, image contents, harness versions, and authentication are not validated by doctor; run with a harness-enabled image. Deterministic fakes are ready."
        );
    }
    Ok(())
}

pub fn recommend(state: &State, source_path: &Path, task: &str) -> Result<()> {
    anyhow::ensure!(!task.trim().is_empty(), "task must not be empty");
    let source_path = source::resolve_source(Some(source_path))?;
    let projected_state_root = canonicalize_allow_missing(&state.root)?;
    anyhow::ensure!(
        !projected_state_root.starts_with(&source_path),
        "Dispatch state directory must be outside the source tree: {}",
        state.root.display()
    );
    let features = classify_task(&source_path, task)?;
    state.initialize()?;
    let database = Database::open(state.db_path())?;
    let supported_real_harnesses = supported_real_harnesses();
    let ranked = rank_harnesses(&database, &features, &supported_real_harnesses)?;

    println!("Task");
    println!(
        "  language: {}",
        features.language.as_deref().unwrap_or("unknown")
    );
    println!("  kind: {}", features.task_kind.as_str());
    println!("  scope: {}", features.scope.as_str());

    let recommendations = ranked
        .iter()
        .filter(|prediction| prediction.evidence.is_some())
        .collect::<Vec<_>>();
    if recommendations.is_empty() {
        println!("\nNo compatible routing evidence is available.");
        return Ok(());
    }

    println!("\nRecommendations");
    for (index, prediction) in recommendations.into_iter().enumerate() {
        let evidence = prediction
            .evidence
            .as_ref()
            .expect("recommendations contain evidence");
        let specificity = specificity_label(evidence.specificity);
        println!("\n{}. {}", index + 1, prediction.harness);
        println!(
            "   benchmark success: {}/{} ({:.1}%)",
            prediction.successes,
            prediction.attempts,
            prediction.score.expect("evidence has a score") * 100.0
        );
        println!(
            "   evidence specificity: {}/3 ({specificity})",
            evidence.specificity
        );
        println!("   source: {}", evidence.prior.source);
        println!("   dataset: {}", evidence.prior.dataset);
        println!("   dataset version: {}", evidence.prior.dataset_version);
        println!(
            "   model: {}",
            evidence.prior.model.as_deref().unwrap_or("<unknown>")
        );
    }
    Ok(())
}

fn supported_real_harnesses() -> Vec<String> {
    SUPPORTED_HARNESSES
        .iter()
        .filter(|harness| !harness.starts_with("fake-"))
        .map(|harness| (*harness).to_owned())
        .collect()
}

async fn select_routed_harness(
    state: &State,
    source_path: &Path,
    task: &str,
    config: &Config,
) -> Result<RoutingDecision> {
    anyhow::ensure!(
        config.execution.backend == "local",
        "--route requires the local backend so harness execution eligibility can be established; use --harnesses for Docker execution"
    );
    let features = classify_task(source_path, task)?;
    state.initialize()?;
    let database = Database::open(state.db_path())?;
    let ranked = rank_harnesses(&database, &features, &supported_real_harnesses())?;
    let mut has_compatible_evidence = false;

    for prediction in ranked {
        let Some(evidence) = prediction.evidence else {
            continue;
        };
        has_compatible_evidence = true;
        let adapter = adapter_for(&prediction.harness, &config.harnesses)?;
        if !adapter.detect().await.available {
            continue;
        }
        return Ok(RoutingDecision {
            version: 1,
            task_features: features,
            selected_harness: prediction.harness,
            successes: prediction.successes,
            attempts: prediction.attempts,
            specificity: evidence.specificity,
            source: evidence.prior.source,
            dataset: evidence.prior.dataset,
            dataset_version: evidence.prior.dataset_version,
            model: evidence.prior.model,
        });
    }

    if !has_compatible_evidence {
        bail!(
            "No compatible routing evidence is available.\nUse --harnesses to select harnesses explicitly."
        );
    }
    bail!(
        "Compatible routing evidence is available, but no predicted harness is execution-eligible locally.\nUse --harnesses to select harnesses explicitly."
    )
}

fn specificity_label(specificity: u8) -> &'static str {
    match specificity {
        0 => "generic",
        3 => "exact",
        _ => "partial",
    }
}

fn print_routing_decision(decision: &RoutingDecision) {
    println!("Routing");
    println!("  selected: {}", decision.selected_harness);
    println!(
        "  benchmark success: {}/{} ({:.1}%)",
        decision.successes,
        decision.attempts,
        decision.successes as f64 / decision.attempts as f64 * 100.0
    );
    println!(
        "  evidence specificity: {}/3 ({})",
        decision.specificity,
        specificity_label(decision.specificity)
    );
    println!("  source: {}", decision.source);
    println!("  dataset: {}", decision.dataset);
    println!("  dataset version: {}", decision.dataset_version);
    println!(
        "  model: {}",
        decision.model.as_deref().unwrap_or("<unknown>")
    );
    println!();
}

pub async fn run_dispatch(state: &State, request: RunRequest) -> Result<RunRecord> {
    anyhow::ensure!(!request.task.trim().is_empty(), "task must not be empty");
    let source_path = source::resolve_source(Some(&request.source))?;
    let projected_state_root = canonicalize_allow_missing(&state.root)?;
    anyhow::ensure!(
        !projected_state_root.starts_with(&source_path),
        "Dispatch state directory must be outside the source tree: {}",
        state.root.display()
    );
    let (mut config, config_path) = Config::discover(&source_path, request.config_path.as_deref())?;
    if let Some(backend) = request.backend {
        config.execution.backend = backend;
    }
    if let Some(timeout) = request.timeout_secs {
        config.execution.timeout_secs = timeout;
    }
    if let Some(max_parallel) = request.max_parallel {
        config.execution.max_parallel = max_parallel;
    }
    config.validate()?;
    anyhow::ensure!(
        config.execution.forwarded_env.is_empty() || request.allow_forwarded_env,
        "configuration requests forwarding environment variables ({names}); review them and re-run with --allow-forwarded-env",
        names = config.execution.forwarded_env.join(", ")
    );

    let (mut harnesses, routing) = if request.route {
        let decision = select_routed_harness(state, &source_path, &request.task, &config).await?;
        (vec![decision.selected_harness.clone()], Some(decision))
    } else {
        (
            request
                .harnesses
                .into_iter()
                .map(|id| id.trim().to_lowercase())
                .filter(|id| !id.is_empty())
                .collect::<Vec<_>>(),
            None,
        )
    };
    anyhow::ensure!(!harnesses.is_empty(), "at least one harness is required");
    let mut seen = HashSet::new();
    for harness_id in &harnesses {
        anyhow::ensure!(
            seen.insert(harness_id.clone()),
            "duplicate harness: {harness_id}"
        );
        adapter_for(harness_id, &config.harnesses)?;
    }
    let executes_untrusted_host_code = harnesses.iter().any(|id| !id.starts_with("fake-"))
        || !config.checks.baseline.is_empty()
        || !config.checks.verify.is_empty();
    let unsafe_local = config.execution.backend == "local" && executes_untrusted_host_code;
    anyhow::ensure!(
        config.execution.backend != "local"
            || !executes_untrusted_host_code
            || request.allow_unsafe_local,
        "local execution cannot isolate the original source from real harnesses or project checks; use --backend docker, or explicitly accept the risk with --allow-unsafe-local"
    );
    anyhow::ensure!(
        harnesses.len() <= 702,
        "at most 702 candidates are supported in v0"
    );

    if let Some(decision) = &routing {
        print_routing_decision(decision);
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
    write_text(&run_dir.join("task.md"), &request.task)?;
    write_text(
        &run_dir.join("config.snapshot.yml"),
        &serde_yaml::to_string(&config).context("failed to serialize effective configuration")?,
    )?;
    let cancellation = CancellationToken::new();
    let _signal_listener = SignalListener::install(cancellation.clone());

    println!("RUN {run_id}\n");
    println!("Creating baseline...");
    let snapshot = match source::create_snapshot(&source_path, &run_dir) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let _ = fs::remove_dir_all(&run_dir);
            return Err(error.context("baseline creation failed; incomplete run state was removed"));
        }
    };
    println!("Creating baseline... done ({})", snapshot.baseline_commit);

    let exact_prompt = build_prompt(&request.task);
    let now = Utc::now();
    let mut run = RunRecord {
        id: run_id.clone(),
        task: request.task,
        exact_prompt: exact_prompt.clone(),
        source_path,
        source_kind: snapshot.kind,
        source_git_head: snapshot.git_head,
        source_fingerprint: snapshot.fingerprint,
        baseline_path: snapshot.baseline_path,
        baseline_commit: snapshot.baseline_commit,
        status: RunStatus::Preparing,
        created_at: now,
        completed_at: None,
        environment: EnvironmentRecord {
            dispatch_version: VERSION.into(),
            os: std::env::consts::OS.into(),
            architecture: std::env::consts::ARCH.into(),
            execution_backend: config.execution.backend.clone(),
            timeout_secs: config.execution.timeout_secs,
            cpus: config.execution.cpus,
            memory: config.execution.memory.clone(),
            max_parallel: config.execution.max_parallel,
            docker_image: (config.execution.backend == "docker")
                .then(|| config.execution.docker_image.clone()),
            resource_limits_enforced: config.execution.backend == "docker",
            unsafe_local,
            forwarded_env: config.execution.forwarded_env.clone(),
        },
        baseline_checks: Vec::new(),
        candidates: Vec::new(),
        routing,
        evaluation: None,
        applied_candidate: None,
    };
    state.save_run(&run)?;
    db.sync_run(&run)?;
    persist_event(
        state,
        &db,
        EventRecord {
            run_id: run.id.clone(),
            candidate_label: None,
            event_type: "run.created".into(),
            timestamp: now,
            payload: serde_json::json!({
                "source": run.source_path,
                "source_kind": run.source_kind.as_str(),
                "config": config_path,
            }),
        },
    )?;
    persist_event(
        state,
        &db,
        EventRecord {
            run_id: run.id.clone(),
            candidate_label: None,
            event_type: "baseline.started".into(),
            timestamp: Utc::now(),
            payload: serde_json::json!({"commit": run.baseline_commit}),
        },
    )?;

    let mut baseline_passed = true;
    let baseline_commands = if config.checks.baseline.is_empty() {
        &config.checks.verify
    } else {
        &config.checks.baseline
    };
    if !baseline_commands.is_empty() {
        println!("Running baseline checks...");
        let baseline_check_dir = run_dir.join("baseline-check");
        let baseline_check_workspace = match source::create_candidate_workspace(
            &run.baseline_path,
            &baseline_check_dir.join("workspace"),
        ) {
            Ok(workspace) => workspace,
            Err(error) => {
                let message = format!("failed to prepare baseline checks: {error:#}");
                finish_failed(state, &mut db, &mut run, &message)?;
                bail!(message);
            }
        };
        run.baseline_checks = run_checks_with_config_and_cancel(
            &baseline_check_workspace,
            baseline_commands,
            CheckPhase::Baseline,
            &run_dir.join("checks/baseline"),
            config.execution.clone(),
            cancellation.clone(),
        )
        .await;
        baseline_passed = run
            .baseline_checks
            .iter()
            .all(|check| check.status == CheckStatus::Passed);
        println!(
            "Running baseline checks... {}",
            if baseline_passed { "pass" } else { "FAIL" }
        );
        state.save_run(&run)?;
        db.sync_run(&run)?;
    }
    if cancellation.is_cancelled() {
        finish_interrupted(state, &mut db, &mut run)?;
        bail!(
            "run {} interrupted; the frozen baseline was preserved",
            run.id
        );
    }
    persist_event(
        state,
        &db,
        EventRecord {
            run_id: run.id.clone(),
            candidate_label: None,
            event_type: "baseline.finished".into(),
            timestamp: Utc::now(),
            payload: serde_json::json!({
                "commit": run.baseline_commit,
                "checks_passed": baseline_passed,
            }),
        },
    )?;

    // Randomize the durable label mapping independently of execution order.
    let mut labels = (0..harnesses.len())
        .map(candidate_label)
        .collect::<Vec<_>>();
    labels.shuffle(&mut rand::rng());
    let mut pending = VecDeque::new();
    println!("\nPreparing {} candidate(s)...", harnesses.len());
    for (harness_id, label) in harnesses.drain(..).zip(labels) {
        let candidate_id = Ulid::new().to_string();
        let candidate_dir = run_dir.join("candidates").join(candidate_id.to_lowercase());
        let workspace = match source::create_candidate_workspace(
            &run.baseline_path,
            &candidate_dir.join("workspace"),
        ) {
            Ok(workspace) => workspace,
            Err(error) => {
                let message = format!("failed to prepare candidate {label}: {error:#}");
                finish_failed(state, &mut db, &mut run, &message)?;
                bail!(message);
            }
        };
        let prompt_path = candidate_dir.join("prompt.txt");
        let stdout_path = candidate_dir.join("stdout.log");
        let stderr_path = candidate_dir.join("stderr.log");
        let diff_path = candidate_dir.join("diff.patch");
        if let Err(error) = write_text(&prompt_path, &exact_prompt) {
            let message = format!("failed to persist candidate {label} prompt: {error:#}");
            finish_failed(state, &mut db, &mut run, &message)?;
            bail!(message);
        }
        let candidate = CandidateRecord {
            id: candidate_id,
            label,
            harness_id,
            harness_version: None,
            model: None,
            status: CandidateStatus::Preparing,
            workspace_path: workspace,
            prompt_path,
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
        run.candidates.push(candidate.clone());
        pending.push_back(candidate);
    }
    run.candidates
        .sort_by(|left, right| left.label.cmp(&right.label));
    run.status = RunStatus::Running;
    state.save_run(&run)?;
    db.sync_run(&run)?;

    if config.execution.backend == "local" && executes_untrusted_host_code {
        eprintln!(
            "warning: real harnesses are running locally; use --backend docker with a harness-enabled image for a container boundary"
        );
    }

    type CandidateFuture = Pin<Box<dyn Future<Output = CandidateRecord> + Send>>;
    let mut running: FuturesUnordered<CandidateFuture> = FuturesUnordered::new();
    let parallelism = config
        .execution
        .max_parallel
        .min(run.candidates.len())
        .max(1);

    let mut interrupted = false;
    let mut orchestration_error = None;
    'candidate_loop: while !pending.is_empty() || !running.is_empty() {
        if cancellation.is_cancelled() {
            interrupted = true;
            break;
        }
        while running.len() < parallelism && !cancellation.is_cancelled() {
            let Some(mut candidate) = pending.pop_front() else {
                break;
            };
            candidate.status = CandidateStatus::Running;
            replace_candidate(&mut run, candidate.clone());
            let persisted = (|| -> Result<()> {
                state.save_run(&run)?;
                db.sync_run(&run)?;
                persist_event(
                    state,
                    &db,
                    EventRecord {
                        run_id: run.id.clone(),
                        candidate_label: Some(candidate.label.clone()),
                        event_type: "candidate.started".into(),
                        timestamp: Utc::now(),
                        payload: serde_json::json!({"harness": candidate.harness_id}),
                    },
                )
            })();
            if let Err(error) = persisted {
                orchestration_error = Some(error.context("failed to persist candidate start"));
                break 'candidate_loop;
            }
            println!("Candidate {}   running", candidate.label);
            let candidate_config = config.clone();
            let candidate_prompt = exact_prompt.clone();
            let baseline_path = run.baseline_path.clone();
            let candidate_cancellation = cancellation.clone();
            running.push(Box::pin(async move {
                execute_candidate(
                    candidate,
                    candidate_prompt,
                    candidate_config,
                    baseline_path,
                    candidate_cancellation,
                )
                .await
            }));
        }

        let next = tokio::select! {
            candidate = running.next() => candidate,
            _ = cancellation.cancelled() => {
                interrupted = true;
                None
            }
        };
        if interrupted {
            break;
        }
        let Some(candidate) = next else {
            continue;
        };
        let event_type = match candidate.status {
            CandidateStatus::Completed => "candidate.finished",
            CandidateStatus::TimedOut => "candidate.timed_out",
            _ => "candidate.failed",
        };
        println!(
            "Candidate {}   {:<16} {}",
            candidate.label,
            candidate.status.as_str(),
            format_duration(candidate.duration_ms)
        );
        replace_candidate(&mut run, candidate.clone());
        let persisted = (|| -> Result<()> {
            state.save_run(&run)?;
            db.sync_run(&run)?;
            persist_event(
                state,
                &db,
                EventRecord {
                    run_id: run.id.clone(),
                    candidate_label: Some(candidate.label.clone()),
                    event_type: event_type.into(),
                    timestamp: Utc::now(),
                    payload: serde_json::json!({
                        "status": candidate.status.as_str(),
                        "duration_ms": candidate.duration_ms,
                        "files_changed": candidate.diff_stats.files_changed,
                    }),
                },
            )
        })();
        if let Err(error) = persisted {
            orchestration_error = Some(error.context("failed to persist candidate completion"));
            break;
        }
    }

    if interrupted || orchestration_error.is_some() {
        cancellation.cancel();
        while let Some(candidate) = running.next().await {
            replace_candidate(&mut run, candidate);
        }
        for mut candidate in pending {
            candidate.status = if interrupted {
                CandidateStatus::Cancelled
            } else {
                CandidateStatus::Failed
            };
            let message = if interrupted {
                "run interrupted before candidate started"
            } else {
                "run failed before candidate started"
            };
            candidate.error = Some(message.into());
            let _ = write_text(&candidate.stdout_path, "");
            let _ = write_text(&candidate.stderr_path, &format!("{message}\n"));
            let _ = write_text(&candidate.diff_path, "");
            replace_candidate(&mut run, candidate);
        }
        for candidate in &mut run.candidates {
            if matches!(
                candidate.status,
                CandidateStatus::Preparing | CandidateStatus::Running | CandidateStatus::Verifying
            ) {
                candidate.status = if interrupted {
                    CandidateStatus::Cancelled
                } else {
                    CandidateStatus::Failed
                };
                append_candidate_error(
                    candidate,
                    if interrupted {
                        "run interrupted".into()
                    } else {
                        "run orchestration failed".into()
                    },
                );
            }
        }
        if let Some(error) = orchestration_error {
            let message = format!("{error:#}");
            if let Err(persist_error) = finish_failed(state, &mut db, &mut run, &message) {
                return Err(error.context(format!(
                    "failed to persist terminal run status: {persist_error:#}"
                )));
            }
            return Err(error);
        }
        finish_interrupted(state, &mut db, &mut run)?;
        bail!(
            "run {} interrupted; partial candidate workspaces were preserved",
            run.id
        );
    }

    run.status = RunStatus::ReadyForEvaluation;
    run.completed_at = Some(Utc::now());
    run.candidates
        .sort_by(|left, right| left.label.cmp(&right.label));
    let finalized = (|| -> Result<()> {
        state.save_run(&run)?;
        db.sync_run(&run)?;
        persist_event(
            state,
            &db,
            EventRecord {
                run_id: run.id.clone(),
                candidate_label: None,
                event_type: "run.ready_for_evaluation".into(),
                timestamp: Utc::now(),
                payload: serde_json::json!({"candidate_count": run.candidates.len()}),
            },
        )
    })();
    if let Err(error) = finalized {
        let message = format!("failed to finalize run: {error:#}");
        finish_failed(state, &mut db, &mut run, &message)?;
        return Err(error.context("failed to finalize run"));
    }

    println!("\nRun ready for evaluation.\n");
    println!(
        "Baseline verification {}\n",
        checks_summary(&run.baseline_checks)
    );
    print_candidates(&run, false);
    println!("Inspect: dispatch inspect {} A", run.id);
    println!("Compare: dispatch compare {}", run.id);
    Ok(run)
}

async fn execute_candidate(
    mut candidate: CandidateRecord,
    prompt: String,
    config: Config,
    baseline_path: PathBuf,
    cancellation: CancellationToken,
) -> CandidateRecord {
    let candidate_dir = candidate
        .prompt_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| candidate.workspace_path.clone());
    let timeout = Duration::from_secs(config.execution.timeout_secs);

    let execution = async {
        let adapter = adapter_for(&candidate.harness_id, &config.harnesses)?;
        let executor = Executor::new(config.execution.clone());
        let request = HarnessRunRequest::new(&candidate.workspace_path, prompt, &candidate_dir)
            .with_timeout(timeout)
            .with_cancellation(cancellation.clone());
        run_harness(adapter.as_ref(), &executor, request).await
    }
    .await;

    match execution {
        Ok(result) => {
            let _ = write_text(
                &candidate_dir.join("harness.jsonl"),
                &result.execution.stdout,
            );
            candidate.harness_version = result.harness_version;
            candidate.model = result.model;
            candidate.duration_ms = result.execution.duration_ms;
            candidate.exit_code = result.execution.exit_code;
            candidate.timed_out = result.execution.timed_out;
            candidate.tokens = result.tokens;
            candidate.token_semantics = result.token_semantics;
            candidate.cost_usd = result.cost_usd;
            candidate.error = result.execution.error;
            candidate.stdout_path = result.execution.stdout_path;
            candidate.stderr_path = result.execution.stderr_path;
            candidate.status = match result.execution.status {
                ExecutionStatus::Succeeded => CandidateStatus::Verifying,
                ExecutionStatus::TimedOut => CandidateStatus::TimedOut,
                ExecutionStatus::Cancelled => CandidateStatus::Cancelled,
                ExecutionStatus::Failed | ExecutionStatus::SpawnFailed => CandidateStatus::Failed,
            };

            if candidate.status == CandidateStatus::Verifying {
                candidate.checks = run_checks_with_config_and_cancel(
                    &candidate.workspace_path,
                    &config.checks.verify,
                    CheckPhase::Verify,
                    &candidate_dir.join("checks"),
                    config.execution.clone(),
                    cancellation.clone(),
                )
                .await;
                candidate.status = if cancellation.is_cancelled() {
                    CandidateStatus::Cancelled
                } else {
                    CandidateStatus::Completed
                };
            }
        }
        Err(error) => {
            let message = format!("{error:#}");
            candidate.status = if message.contains(" is unavailable:") {
                CandidateStatus::MissingHarness
            } else {
                CandidateStatus::Failed
            };
            candidate.error = Some(message.clone());
            let _ = write_text(&candidate.stdout_path, "");
            let _ = write_text(&candidate.stderr_path, &format!("{message}\n"));
            let _ = write_text(&candidate_dir.join("harness.jsonl"), "");
        }
    }

    match source::collect_diff(
        &baseline_path,
        &candidate.workspace_path,
        &candidate.diff_path,
    ) {
        Ok(stats) => candidate.diff_stats = stats,
        Err(error) => {
            append_candidate_error(&mut candidate, format!("diff collection failed: {error:#}"));
            if candidate.status == CandidateStatus::Completed {
                candidate.status = CandidateStatus::Failed;
            }
            let _ = write_text(&candidate.diff_path, "");
        }
    }
    candidate
}

fn candidate_label(index: usize) -> String {
    let mut number = index + 1;
    let mut label = Vec::new();
    while number > 0 {
        number -= 1;
        label.push((b'A' + (number % 26) as u8) as char);
        number /= 26;
    }
    label.iter().rev().collect()
}

fn replace_candidate(run: &mut RunRecord, candidate: CandidateRecord) {
    if let Some(existing) = run
        .candidates
        .iter_mut()
        .find(|item| item.id == candidate.id)
    {
        *existing = candidate;
    } else {
        run.candidates.push(candidate);
    }
    run.candidates
        .sort_by(|left, right| left.label.cmp(&right.label));
}

fn append_candidate_error(candidate: &mut CandidateRecord, error: String) {
    match &mut candidate.error {
        Some(existing) => {
            existing.push_str("; ");
            existing.push_str(&error);
        }
        None => candidate.error = Some(error),
    }
}

fn persist_event(state: &State, db: &Database, event: EventRecord) -> Result<()> {
    state.append_event(&event)?;
    db.record_event(&event)
}

fn finish_interrupted(state: &State, db: &mut Database, run: &mut RunRecord) -> Result<()> {
    run.status = RunStatus::Interrupted;
    run.completed_at = Some(Utc::now());
    run.candidates
        .sort_by(|left, right| left.label.cmp(&right.label));
    state.save_run(run)?;
    db.sync_run(run)?;
    persist_event(
        state,
        db,
        EventRecord {
            run_id: run.id.clone(),
            candidate_label: None,
            event_type: "run.interrupted".into(),
            timestamp: Utc::now(),
            payload: serde_json::json!({"candidate_count": run.candidates.len()}),
        },
    )
}

fn finish_failed(
    state: &State,
    db: &mut Database,
    run: &mut RunRecord,
    message: &str,
) -> Result<()> {
    run.status = RunStatus::Failed;
    run.completed_at = Some(Utc::now());
    run.candidates
        .sort_by(|left, right| left.label.cmp(&right.label));
    state.save_run(run)?;
    db.sync_run(run)?;
    persist_event(
        state,
        db,
        EventRecord {
            run_id: run.id.clone(),
            candidate_label: None,
            event_type: "run.failed".into(),
            timestamp: Utc::now(),
            payload: serde_json::json!({"error": message}),
        },
    )
}

struct OperationLock {
    file: fs::File,
    #[cfg(not(unix))]
    path: PathBuf,
}

impl OperationLock {
    fn acquire(path: &Path, busy_message: &str) -> Result<Self> {
        if let Some(parent) = path.parent() {
            let parent_is_new = !parent.exists();
            fs::create_dir_all(parent)?;
            #[cfg(unix)]
            if parent_is_new {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
            }
        }
        if let Ok(metadata) = fs::symlink_metadata(path) {
            anyhow::ensure!(
                !metadata.file_type().is_symlink(),
                "refusing operation lock symlink {}",
                path.display()
            );
        }

        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(path)?;
            // SAFETY: the descriptor remains owned by this guard until Drop.
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if result != 0 {
                bail!("{busy_message}: {}", std::io::Error::last_os_error());
            }
            Ok(Self { file })
        }

        #[cfg(not(unix))]
        {
            let file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(path)
                .with_context(|| busy_message.to_owned())?;
            Ok(Self {
                file,
                path: path.to_path_buf(),
            })
        }
    }
}

impl Drop for OperationLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // SAFETY: this unlocks only the descriptor locked by acquire.
            unsafe {
                libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
            }
        }
        #[cfg(not(unix))]
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

struct SignalListener {
    task: tokio::task::JoinHandle<()>,
}

impl SignalListener {
    fn install(cancellation: CancellationToken) -> Self {
        let task = tokio::spawn(async move {
            shutdown_signal().await;
            cancellation.cancel();
        });
        Self { task }
    }
}

impl Drop for SignalListener {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    if let Ok(mut terminate) = signal(SignalKind::terminate()) {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
    } else {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

pub fn status(state: &State, id: Option<&str>) -> Result<()> {
    let run = match id {
        Some(id) => state.load_run(id)?,
        None => load_latest(state)?,
    };
    let reveal = run.evaluation.is_some();
    print_run_header(&run, reveal);
    println!();
    print_candidates(&run, reveal);
    Ok(())
}

pub fn history(state: &State, limit: usize) -> Result<()> {
    state.initialize()?;
    let db = Database::open(state.db_path())?;
    let rows = db.list_runs(limit)?;
    if rows.is_empty() {
        println!("No Dispatch runs yet.");
        return Ok(());
    }
    println!(
        "{:<28} {:<22} {:<11} {:<10} TASK",
        "RUN", "CREATED", "STATUS", "CANDIDATES"
    );
    for row in rows {
        let task = one_line(&row.task, 52);
        println!(
            "{:<28} {:<22} {:<11} {:<10} {}",
            row.id, row.created_at, row.status, row.candidate_count, task
        );
    }
    Ok(())
}

pub fn show(state: &State, run_id: &str) -> Result<()> {
    let resolved_run_id = state.resolve_run_id(run_id)?;
    let run = state.load_run(&resolved_run_id)?;
    let reveal = run.evaluation.is_some();
    print_run_header(&run, reveal);
    println!("\nTask\n{}", run.task);
    println!("\nBaseline checks");
    print_checks(&run.baseline_checks);
    println!("\nCandidates");
    print_candidates(&run, reveal);
    for candidate in &run.candidates {
        println!("Candidate {} artifacts", candidate.label);
        println!("  stdout   {}", candidate.stdout_path.display());
        println!("  stderr   {}", candidate.stderr_path.display());
        println!("  diff     {}", candidate.diff_path.display());
        println!("  checks");
        print_checks(&candidate.checks);
        println!();
    }
    if let Some(evaluation) = &run.evaluation {
        print_evaluation(evaluation);
    }
    if let Some(label) = &run.applied_candidate {
        println!("\nApplied candidate: {label}");
    }
    Ok(())
}

pub fn diff(
    state: &State,
    run_id: &str,
    candidate: &str,
    stat: bool,
    name_only: bool,
) -> Result<()> {
    let run = state.load_run(run_id)?;
    let candidate = find_candidate(&run, candidate)?;
    if name_only {
        for path in &candidate.diff_stats.changed_files {
            println!("{path}");
        }
        return Ok(());
    }
    if stat {
        print_diff_stats(candidate);
        return Ok(());
    }
    println!("Candidate {}\n", candidate.label);
    print_diff_stats(candidate);
    println!();
    let patch = fs::read_to_string(&candidate.diff_path)
        .with_context(|| format!("failed to read {}", candidate.diff_path.display()))?;
    print!("{patch}");
    Ok(())
}

pub fn inspect(state: &State, run_id: &str, candidate: &str, shell: bool) -> Result<()> {
    let run = state.load_run(run_id)?;
    let candidate = find_candidate(&run, candidate)?;
    println!(
        "Candidate {} workspace:\n\n{}",
        candidate.label,
        candidate.workspace_path.display()
    );
    if shell {
        let shell_program = std::env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into());
        let status = Command::new(shell_program)
            .current_dir(&candidate.workspace_path)
            .status()
            .context("failed to start shell")?;
        anyhow::ensure!(status.success(), "candidate shell exited with {status}");
    }
    Ok(())
}

pub fn compare(
    state: &State,
    run_id: &str,
    mut input: EvaluationInput,
    interactive: bool,
) -> Result<()> {
    let resolved_run_id = state.resolve_run_id(run_id)?;
    let _run_lock = OperationLock::acquire(
        &state.run_dir(&resolved_run_id).join(".operation.lock"),
        "another compare/apply operation is already using this run",
    )?;
    let mut run = state.load_run(&resolved_run_id)?;
    let already_evaluated = run.evaluation.is_some();
    if !already_evaluated
        && !matches!(
            run.status,
            RunStatus::ReadyForEvaluation | RunStatus::Applied
        )
    {
        bail!(
            "run {} is {}; evaluation requires a run that is ready for evaluation",
            run.id,
            run.status.as_str()
        );
    }
    let reveal = already_evaluated;
    print_run_header(&run, reveal);
    println!("\nTask:\n{}\n", run.task);
    println!(
        "Baseline verification {}\n",
        checks_summary(&run.baseline_checks)
    );
    print_candidates(&run, reveal);

    if interactive && input.winner.is_none() && !already_evaluated {
        input = prompt_for_evaluation(&run)?;
    }
    if input.winner.is_none() {
        if let Some(evaluation) = &run.evaluation {
            print_evaluation(evaluation);
        } else {
            println!("\nUse --evaluate, or --winner A|B|tie|neither, to record an evaluation.");
        }
        return Ok(());
    }
    if already_evaluated {
        bail!(
            "run {} already has an evaluation; it was not overwritten",
            run.id
        );
    }

    let winner = input.winner.as_deref().expect("checked above");
    let outcome = parse_outcome(&run, winner)?;
    let reasons = normalize_reasons(input.reasons)?;
    let evaluation = EvaluationRecord {
        outcome,
        reasons,
        explanation: input.explanation,
        created_at: Utc::now(),
        blind: true,
    };
    run.evaluation = Some(evaluation.clone());
    if run.status != RunStatus::Applied {
        run.status = RunStatus::Evaluated;
    }
    let mut db = Database::open(state.db_path())?;
    db.save_evaluation(&run.id, &evaluation)?;
    persist_event(
        state,
        &db,
        EventRecord {
            run_id: run.id.clone(),
            candidate_label: None,
            event_type: "evaluation.submitted".into(),
            timestamp: Utc::now(),
            payload: serde_json::to_value(&evaluation)?,
        },
    )?;
    // Metadata is the reveal source of truth and is written last. Any earlier
    // validation, database, event, or serialization failure leaves it blind.
    state.save_run(&run)?;

    println!("\nEvaluation recorded. Harness identities are now revealed:\n");
    print_candidates(&run, true);
    print_evaluation(&evaluation);
    Ok(())
}

pub fn apply(state: &State, run_id: &str, candidate_label: &str) -> Result<()> {
    let resolved_run_id = state.resolve_run_id(run_id)?;
    let _run_lock = OperationLock::acquire(
        &state.run_dir(&resolved_run_id).join(".operation.lock"),
        "another compare/apply operation is already using this run",
    )?;
    let mut run = state.load_run(&resolved_run_id)?;
    anyhow::ensure!(
        matches!(
            run.status,
            RunStatus::ReadyForEvaluation | RunStatus::Evaluated
        ),
        "run {} is {}; apply requires a completed, unapplied run",
        run.id,
        run.status.as_str()
    );
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
    let report = source::safe_apply(&run, &normalized_label)?;
    run.applied_candidate = Some(normalized_label.clone());
    run.status = RunStatus::Applied;
    state.save_run(&run)?;
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
            payload: serde_json::json!({"files_changed": report.files_changed}),
        },
    )?;
    println!(
        "Applied Candidate {} to {} ({} file(s) changed).",
        normalized_label,
        run.source_path.display(),
        report.files_changed
    );
    Ok(())
}

pub fn read_verbatim(path: &Path) -> Result<String> {
    if path == Path::new("-") {
        let mut value = String::new();
        io::stdin().read_to_string(&mut value)?;
        return Ok(value);
    }
    fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))
}

fn prompt_for_evaluation(run: &RunRecord) -> Result<EvaluationInput> {
    let labels = run
        .candidates
        .iter()
        .map(|c| c.label.as_str())
        .collect::<Vec<_>>()
        .join("/");
    println!("\nChoose {labels}/Tie/Neither (blank cancels):");
    print!("> ");
    io::stdout().flush()?;
    let mut winner = String::new();
    io::stdin().read_line(&mut winner)?;
    let winner = winner.trim().to_owned();
    if winner.is_empty() {
        return Ok(EvaluationInput::default());
    }
    parse_outcome(run, &winner)?;

    println!("\nOptional structured reasons (comma-separated, or blank):");
    println!("{}", EVALUATION_REASONS.join(", "));
    print!("> ");
    io::stdout().flush()?;
    let mut reasons = String::new();
    io::stdin().read_line(&mut reasons)?;
    let reasons = reasons
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();

    println!(
        "\nAdditional reasoning (optional, unrestricted). End with a line containing only '.':"
    );
    let mut explanation = String::new();
    loop {
        let mut line = String::new();
        let bytes = io::stdin().read_line(&mut line)?;
        if bytes == 0 || matches!(line.as_str(), ".\n" | ".\r\n" | ".") {
            break;
        }
        explanation.push_str(&line);
    }
    Ok(EvaluationInput {
        winner: Some(winner),
        reasons,
        explanation: (!explanation.is_empty()).then_some(explanation),
    })
}

fn parse_outcome(run: &RunRecord, value: &str) -> Result<EvaluationOutcome> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("tie") {
        return Ok(EvaluationOutcome::Tie);
    }
    if value.eq_ignore_ascii_case("neither") {
        return Ok(EvaluationOutcome::Neither);
    }
    let candidate = find_candidate(run, value)?;
    Ok(EvaluationOutcome::Candidate(candidate.label.clone()))
}

fn normalize_reasons(reasons: Vec<String>) -> Result<Vec<String>> {
    let mut output = Vec::new();
    for reason in reasons {
        let reason = reason.trim().to_lowercase();
        if !EVALUATION_REASONS.contains(&reason.as_str()) {
            bail!(
                "unknown structured reason: {reason:?}\nvalid reasons: {}",
                EVALUATION_REASONS.join(", ")
            );
        }
        if !output.contains(&reason) {
            output.push(reason);
        }
    }
    Ok(output)
}

fn find_candidate<'a>(run: &'a RunRecord, value: &str) -> Result<&'a CandidateRecord> {
    run.candidates
        .iter()
        .find(|candidate| {
            candidate.label.eq_ignore_ascii_case(value) || candidate.id.eq_ignore_ascii_case(value)
        })
        .with_context(|| format!("run {} has no candidate {value}", run.id))
}

fn load_latest(state: &State) -> Result<RunRecord> {
    let path = state
        .list_metadata_paths()?
        .into_iter()
        .next()
        .context("no Dispatch runs yet")?;
    let bytes = fs::read(&path)?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid metadata at {}", path.display()))
}

fn print_run_header(run: &RunRecord, reveal: bool) {
    println!("RUN {}", run.id);
    println!("Status           {}", run.status.as_str());
    println!("Source           {}", run.source_path.display());
    println!("Source type      {}", run.source_kind.as_str());
    println!("Baseline         {}", run.baseline_commit);
    if reveal {
        println!("Harness mapping  revealed");
    } else {
        println!("Harness mapping  blind");
    }
    if let Some(decision) = &run.routing {
        println!();
        print_routing_decision(decision);
    }
}

fn print_candidates(run: &RunRecord, reveal: bool) {
    if run.candidates.is_empty() {
        println!("No candidates recorded.");
        return;
    }
    for candidate in &run.candidates {
        let title = if reveal {
            format!("Candidate {} ({})", candidate.label, candidate.harness_id)
        } else {
            format!("Candidate {}", candidate.label)
        };
        println!("{title}");
        println!("  Status          {}", candidate.status.as_str());
        println!("  Verification    {}", verification_summary(candidate));
        println!(
            "  Runtime         {}",
            format_duration(candidate.duration_ms)
        );
        println!(
            "  Cost            {}",
            candidate
                .cost_usd
                .map_or_else(|| "unknown".into(), |v| format!("${v:.4}"))
        );
        println!(
            "  Task tokens     {}",
            candidate
                .tokens
                .map_or_else(|| "unknown".into(), |v| v.to_string())
        );
        println!(
            "  Token basis     {}",
            candidate.token_semantics.as_deref().unwrap_or("unknown")
        );
        println!("  Files changed   {}", candidate.diff_stats.files_changed);
        println!(
            "  Diff             +{} / -{}",
            candidate.diff_stats.lines_added, candidate.diff_stats.lines_removed
        );
        if reveal && let Some(error) = &candidate.error {
            println!("  Error           {}", one_line(error, 100));
        }
        println!();
    }
}

fn print_checks(checks: &[crate::CheckResult]) {
    if checks.is_empty() {
        println!("  none configured");
        return;
    }
    for check in checks {
        println!(
            "  {:<8} {:<8} {:>8}  {}",
            format!("{:?}", check.phase).to_lowercase(),
            format!("{:?}", check.status).to_uppercase(),
            format_duration(check.duration_ms),
            check.command
        );
        println!("    stdout {}", check.stdout_path.display());
        println!("    stderr {}", check.stderr_path.display());
    }
}

fn print_diff_stats(candidate: &CandidateRecord) {
    println!("Files changed: {}", candidate.diff_stats.files_changed);
    println!(
        "+{} / -{}",
        candidate.diff_stats.lines_added, candidate.diff_stats.lines_removed
    );
}

fn print_evaluation(evaluation: &EvaluationRecord) {
    let outcome = match &evaluation.outcome {
        EvaluationOutcome::Candidate(label) => format!("Candidate {label}"),
        EvaluationOutcome::Tie => "Tie".into(),
        EvaluationOutcome::Neither => "Neither".into(),
    };
    println!("\nEvaluation");
    println!("  Outcome         {outcome}");
    if !evaluation.reasons.is_empty() {
        println!("  Reasons         {}", evaluation.reasons.join(", "));
    }
    if let Some(explanation) = &evaluation.explanation {
        println!("  Explanation:\n{}", indent(explanation, "    "));
    }
}

fn verification_summary(candidate: &CandidateRecord) -> &'static str {
    if candidate.checks.is_empty() && candidate.status == CandidateStatus::TimedOut {
        return "not run — candidate timed out";
    }
    checks_summary(&candidate.checks)
}

fn checks_summary(checks: &[crate::CheckResult]) -> &'static str {
    if checks.is_empty() {
        return "not configured";
    }
    if checks.iter().all(|c| c.status == CheckStatus::Passed) {
        "PASS"
    } else {
        "FAIL"
    }
}

fn format_duration(ms: u64) -> String {
    let seconds = ms / 1000;
    if seconds < 60 {
        format!("{}.{:01}s", seconds, (ms % 1000) / 100)
    } else {
        format!("{}m{:02}s", seconds / 60, seconds % 60)
    }
}

fn command_version(program: &str, args: &[&str]) -> Option<String> {
    let executable = trusted_host_executable(program).ok()?;
    let output = Command::new(executable).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = if output.stdout.is_empty() {
        &output.stderr
    } else {
        &output.stdout
    };
    Some(one_line(&String::from_utf8_lossy(value), 80))
}

fn print_detection(label: &str, version: &Option<String>) {
    match version {
        Some(version) => println!("{label:<17} ✓ {version}"),
        None => println!("{label:<17} – not found"),
    }
}

fn one_line(value: &str, max_chars: usize) -> String {
    let mut text = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > max_chars {
        text = text
            .chars()
            .take(max_chars.saturating_sub(1))
            .collect::<String>();
        text.push('…');
    }
    text
}

fn canonicalize_allow_missing(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut cursor = absolute.as_path();
    let mut missing = Vec::<OsString>::new();
    loop {
        match fs::canonicalize(cursor) {
            Ok(mut resolved) => {
                for component in missing.iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                missing.push(
                    cursor
                        .file_name()
                        .with_context(|| format!("could not resolve {}", path.display()))?
                        .to_os_string(),
                );
                cursor = cursor
                    .parent()
                    .with_context(|| format!("could not resolve {}", path.display()))?;
            }
            Err(error) => {
                return Err(error).with_context(|| format!("failed to resolve {}", path.display()));
            }
        }
    }
}

fn indent(value: &str, prefix: &str) -> String {
    value
        .lines()
        .map(|line| format!("{prefix}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_format_is_compact() {
        assert_eq!(format_duration(1_234), "1.2s");
        assert_eq!(format_duration(125_000), "2m05s");
    }

    #[test]
    fn timed_out_candidate_reports_verification_not_run() {
        let candidate = CandidateRecord {
            id: "candidate".into(),
            label: "A".into(),
            harness_id: "fake-timeout".into(),
            harness_version: None,
            model: None,
            status: CandidateStatus::TimedOut,
            workspace_path: PathBuf::new(),
            prompt_path: PathBuf::new(),
            stdout_path: PathBuf::new(),
            stderr_path: PathBuf::new(),
            diff_path: PathBuf::new(),
            duration_ms: 1_000,
            exit_code: None,
            timed_out: true,
            tokens: None,
            token_semantics: None,
            cost_usd: None,
            error: None,
            diff_stats: DiffStats::default(),
            checks: vec![],
        };

        assert_eq!(
            verification_summary(&candidate),
            "not run — candidate timed out"
        );
    }

    #[test]
    fn validates_structured_reasons_and_deduplicates() {
        let reasons = normalize_reasons(vec![
            " Correctness ".into(),
            "correctness".into(),
            "cleaner-change".into(),
        ])
        .unwrap();
        assert_eq!(reasons, vec!["correctness", "cleaner-change"]);
        let error = normalize_reasons(vec!["vibes".into()])
            .unwrap_err()
            .to_string();
        assert!(error.contains("valid reasons:"));
        assert!(error.contains("edge-cases, cleaner-change"));
    }

    #[test]
    fn init_generates_valid_config_with_verification_prompt() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let state = State {
            root: temp.path().join("state"),
        };

        init(&state, &project, false).unwrap();
        let contents = fs::read_to_string(project.join("dispatch.yml")).unwrap();
        assert!(contents.contains("Replace [] with your project's verification commands"));
        let config: Config = serde_yaml::from_str(&contents).unwrap();
        config.validate().unwrap();
        assert!(config.checks.verify.is_empty());
    }

    #[test]
    fn init_rejects_state_nested_in_project_before_writing() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        fs::create_dir(&project).unwrap();
        let state = State {
            root: project.join(".dispatch"),
        };

        let error = init(&state, &project, false).unwrap_err().to_string();
        assert!(error.contains("outside the project tree"));
        assert!(!project.join("dispatch.yml").exists());
        assert!(!state.root.exists());
    }

    #[cfg(unix)]
    #[test]
    fn init_refuses_configuration_symlinks() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let outside = temp.path().join("outside.yml");
        fs::create_dir(&project).unwrap();
        symlink(&outside, project.join("dispatch.yml")).unwrap();
        let state = State {
            root: temp.path().join("state"),
        };

        let error = init(&state, &project, true).unwrap_err().to_string();
        assert!(error.contains("through symlink"));
        assert!(!outside.exists());
        assert!(!state.root.exists());
    }
}
