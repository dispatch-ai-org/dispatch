//! `dispatch serve`: the repo-scoped foreground owner loop for foreign and
//! orphaned attached Work (part 6.7, 6.9, 14.4, 14.11 of
//! `docs/plan-0.3-auto-apply-and-attach.md`). One process per integration
//! root: every tick it observes the root's world, re-evaluates active
//! attached Work whose live owner is not `Live`, auto-applies Ready attached
//! Work whose `capabilities.integrate` is true, adopts Work whose stored
//! owner has gone, and prints the project view. It never finishes, launches,
//! kills or refreshes anything; wrapped attach (S4) and human review remain
//! the only things that do.

use std::{
    collections::HashMap,
    fs,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use chrono::Utc;
use sha2::{Digest, Sha256};

use super::{ApplyOutcome, WorkLine, apply, auto_apply, persist_event, work_line};
use crate::{
    ApplicationState, Config, Decision, EventRecord, LifecycleState, OwnerState, ReviewState,
    RunMode, RunRecord, SourceKind, WorkResult,
    coherence::{self, WorkView, watch::Policy, world},
    db::Database,
    lock::{OperationLock, shutdown_signal},
    process::{IdentityState, ProcessIdentity, identity_state},
    source,
    state::State,
};

/// `dispatch serve [--root <path>] [--json]`. Loops until Ctrl+C, SIGTERM or
/// SIGHUP; a second `serve` on the same root refuses to start.
pub async fn serve(state: &State, root: Option<PathBuf>, json: bool) -> Result<()> {
    state.initialize()?;
    let root = source::resolve_source(root.as_deref())?;
    let _serve_lock =
        OperationLock::acquire(&serve_lock_path(state, &root), "already serving this root")?;
    let (kind, _head) = source::inspect_source(&root)?;
    let (config, _config_path) = Config::discover(&root, None)?;
    let poll = Duration::from_secs(config.coherence.poll_secs.max(1));

    let mut ticker = tokio::time::interval(poll);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let mut last_signal: Option<world::Signal> = None;
    let mut policies: HashMap<String, Policy> = HashMap::new();
    let mut shown: HashMap<String, String> = HashMap::new();
    let mut last_block: Vec<String> = Vec::new();
    let mut first = true;

    loop {
        tokio::select! {
            _ = ticker.tick() => {}
            _ = shutdown_signal() => return Ok(()),
        }

        let mut errors = TickErrors::default();
        let mut changed = first;
        let mut digest: Option<String> = None;

        let signal_now = match world::signal(&root, &kind) {
            Ok(signal) => Some(signal),
            Err(error) => {
                errors.report(&error);
                None
            }
        };
        let moved = match (&signal_now, &last_signal) {
            (Some(now), Some(previous)) => now != previous,
            (Some(_), None) => true,
            (None, _) => false,
        };
        if let Some(signal_now) = &signal_now {
            last_signal = Some(signal_now.clone());
        }

        let adopted = adopt_orphans(state, &root, &mut errors);
        if moved || adopted {
            let pass = reevaluate(state, &root, &kind, &mut policies, &mut errors);
            changed |= pass.changed;
            digest = pass.digest;
        }

        for run in ready_for_auto_apply(state, &root, &mut errors) {
            match auto_apply(state, &run.id) {
                Ok(ApplyOutcome::Applied { .. }) => {
                    changed = true;
                    // The apply is the doorbell: re-observe immediately
                    // rather than waiting for the next tick's signal.
                    let pass = reevaluate(state, &root, &kind, &mut policies, &mut errors);
                    changed |= pass.changed;
                    digest = digest.or(pass.digest);
                }
                Ok(_) => {}
                Err(error) => errors.report(&error),
            }
        }

        if json && moved {
            let digest = digest.clone().or_else(|| signal_now.map(|signal| signal.0));
            if let Some(digest) = digest {
                println!("{}", serde_json::json!({"type": "world", "digest": digest}));
            }
        }

        // Render every tick: a run attached, finished or applied by another
        // process changes the view without any verdict or apply of our own.
        // `render_view` prints only what differs from what is already shown.
        let _ = changed;
        match load_source_runs(state, &root) {
            Ok(runs) => render_view(&runs, json, &mut shown, &mut last_block),
            Err(error) => errors.report(&error),
        }
        let _ = io::stdout().flush();

        first = false;
    }
}

/// Logs at most one error per tick to stderr; SQLite busy and other
/// transient failures are retried on the next tick rather than aborting the
/// loop (part 6.9, part 5 of the objective).
#[derive(Default)]
struct TickErrors {
    logged: bool,
}

impl TickErrors {
    fn report(&mut self, error: &anyhow::Error) {
        if !self.logged {
            eprintln!("serve: {error:#}");
            self.logged = true;
        }
    }
}

fn serve_lock_path(state: &State, root: &Path) -> PathBuf {
    let key = hex::encode(Sha256::digest(root.to_string_lossy().as_bytes()));
    state.root.join("locks").join(format!("serve-{key}.lock"))
}

fn run_lock_path(state: &State, run_id: &str) -> PathBuf {
    state.run_dir(run_id).join(".operation.lock")
}

/// Every run (any mode) whose `source_path` is `root`, loaded fresh from
/// disk/DB. Mirrors `orchestrator::load_latest_for_source`'s scan.
fn load_source_runs(state: &State, root: &Path) -> Result<Vec<RunRecord>> {
    let mut runs = Vec::new();
    for path in state.list_metadata_paths()? {
        let projected: RunRecord = serde_json::from_slice(&fs::read(&path)?)
            .with_context(|| format!("invalid metadata at {}", path.display()))?;
        let run = state.load_run(&projected.id)?;
        if run.source_path == root {
            runs.push(run);
        }
    }
    Ok(runs)
}

/// `ExactLive` maps to `Live`, `Gone`/`Reused` to `Gone`; no owner, or a
/// liveness check that could not tell, is `Unknown` (part 14.4's step (b)).
fn live_owner_state(owner: Option<&ProcessIdentity>) -> OwnerState {
    match owner {
        None => OwnerState::Unknown,
        Some(identity) => match identity_state(identity) {
            IdentityState::ExactLive => OwnerState::Live,
            IdentityState::Gone | IdentityState::Reused => OwnerState::Gone,
            IdentityState::Unknown => OwnerState::Unknown,
        },
    }
}

/// Adopt every active attached run whose stored owner state is `Live` but
/// whose owner process is now gone: commit `attach.adopted` once and mark
/// `owner_state: Adopted`. A run whose lock is held elsewhere is skipped,
/// not forced. Returns whether anything was adopted this tick.
fn adopt_orphans(state: &State, root: &Path, errors: &mut TickErrors) -> bool {
    let runs = match load_source_runs(state, root) {
        Ok(runs) => runs,
        Err(error) => {
            errors.report(&error);
            return false;
        }
    };
    let mut adopted = false;
    for candidate in runs {
        if candidate.mode != RunMode::Attached
            || candidate.outcome.lifecycle == LifecycleState::Finished
        {
            continue;
        }
        let is_orphaned = candidate.attachment.as_ref().is_some_and(|attachment| {
            attachment.owner_state == OwnerState::Live
                && live_owner_state(attachment.owner.as_ref()) == OwnerState::Gone
        });
        if !is_orphaned {
            continue;
        }
        let Ok(_lock) = OperationLock::acquire(
            &run_lock_path(state, &candidate.id),
            "run has a foreground owner",
        ) else {
            continue; // busy: another owner is active; skip, don't force it
        };
        let mut run = match state.load_run(&candidate.id) {
            Ok(run) => run,
            Err(error) => {
                errors.report(&error);
                continue;
            }
        };
        // Re-check under the lock: the state may have changed since the
        // unlocked scan above (finished, or already adopted).
        let still_orphaned = run.attachment.as_ref().is_some_and(|attachment| {
            attachment.owner_state == OwnerState::Live
                && live_owner_state(attachment.owner.as_ref()) == OwnerState::Gone
        });
        if run.outcome.lifecycle == LifecycleState::Finished || !still_orphaned {
            continue;
        }
        if let Some(attachment) = run.attachment.as_mut() {
            attachment.owner_state = OwnerState::Adopted;
        }
        let database = match Database::open(state.db_path()) {
            Ok(database) => database,
            Err(error) => {
                errors.report(&error);
                continue;
            }
        };
        let event = EventRecord {
            run_id: run.id.clone(),
            candidate_label: run.candidates.first().map(|c| c.label.clone()),
            event_type: "attach.adopted".into(),
            timestamp: Utc::now(),
            payload: serde_json::json!({"owner_state": "adopted"}),
            ..EventRecord::default()
        };
        if let Err(error) = persist_event(state, &database, event, &mut run) {
            errors.report(&error);
            continue;
        }
        adopted = true;
    }
    adopted
}

/// What one reevaluation pass did: whether any verdict was persisted, and
/// the world digest from the first `world::observe` call it made (`observe`
/// hashes the current tree independently of which run's baseline is passed
/// in, so every call this tick reports the same digest).
struct ObservePass {
    changed: bool,
    digest: Option<String>,
}

/// Re-evaluate every active attached run whose live owner state is not
/// `Live` (a live wrapper owns those; part 14.4 step (b)/(c)): observe the
/// world against that run's own baseline, evaluate, drop
/// `analysis_uncertain`-only reasons, apply the watcher's "worth sending"
/// rule, and persist through `apply::persist_verdict`.
fn reevaluate(
    state: &State,
    root: &Path,
    kind: &SourceKind,
    policies: &mut HashMap<String, Policy>,
    errors: &mut TickErrors,
) -> ObservePass {
    let mut pass = ObservePass {
        changed: false,
        digest: None,
    };
    let runs = match load_source_runs(state, root) {
        Ok(runs) => runs,
        Err(error) => {
            errors.report(&error);
            return pass;
        }
    };
    for candidate in runs {
        if candidate.mode != RunMode::Attached
            || candidate.outcome.lifecycle == LifecycleState::Finished
        {
            continue;
        }
        let Some(attachment) = candidate.attachment.as_ref() else {
            continue;
        };
        if live_owner_state(attachment.owner.as_ref()) == OwnerState::Live {
            continue; // the wrapper's own business
        }
        let Ok(_lock) = OperationLock::acquire(
            &run_lock_path(state, &candidate.id),
            "run has a foreground owner",
        ) else {
            continue; // busy: skip this tick, don't force it
        };
        let mut run = match state.load_run(&candidate.id) {
            Ok(run) => run,
            Err(error) => {
                errors.report(&error);
                continue;
            }
        };
        if run.outcome.lifecycle == LifecycleState::Finished {
            continue; // finished meanwhile (e.g. `dispatch finish` raced us)
        }
        let Some(attachment) = run.attachment.clone() else {
            continue;
        };
        if live_owner_state(attachment.owner.as_ref()) == OwnerState::Live {
            continue;
        }
        let delta_path = state.run_dir(&run.id).join("delta-live.patch");
        if let Err(error) =
            source::snapshot_delta(&run.baseline_path, &attachment.workspace, &delta_path)
        {
            errors.report(&error);
            continue;
        }
        let world = match world::observe(root, &run.baseline_path, &run.baseline_commit, kind) {
            Ok(world) => world,
            Err(error) => {
                errors.report(&error);
                continue;
            }
        };
        pass.digest.get_or_insert_with(|| world.digest.clone());
        let validity = match coherence::evaluate(
            &world,
            &WorkView {
                source: root,
                delta_patch: &delta_path,
                baseline: &run.baseline_path,
                baseline_commit: &run.baseline_commit,
            },
        ) {
            Ok(validity) => validity,
            Err(error) => {
                errors.report(&error);
                continue;
            }
        };
        let validity = coherence::watch::settle(validity);
        let policy = policies.entry(run.id.clone()).or_default();
        let Some(worth_sending) = policy.step(Instant::now(), Some(validity)) else {
            continue;
        };
        let database = match Database::open(state.db_path()) {
            Ok(database) => database,
            Err(error) => {
                errors.report(&error);
                continue;
            }
        };
        if let Err(error) = apply::persist_verdict(state, &database, &mut run, &worth_sending) {
            errors.report(&error);
            continue;
        }
        pass.changed = true;
    }
    pass
}

/// Every attached run ready to auto-apply: finished, `Ready`, unreviewed,
/// unapplied, and `capabilities.integrate`.
fn ready_for_auto_apply(state: &State, root: &Path, errors: &mut TickErrors) -> Vec<RunRecord> {
    match load_source_runs(state, root) {
        Ok(runs) => runs
            .into_iter()
            .filter(|run| {
                run.mode == RunMode::Attached
                    && run.outcome.lifecycle == LifecycleState::Finished
                    && run.outcome.work_result == WorkResult::Ready
                    && run.outcome.review == ReviewState::Pending
                    && run.outcome.application == ApplicationState::NotApplied
                    && run
                        .attachment
                        .as_ref()
                        .is_some_and(|attachment| attachment.capabilities.integrate)
            })
            .collect(),
        Err(error) => {
            errors.report(&error);
            Vec::new()
        }
    }
}

/// `working`/`ready`/`applied`/`blocked`/`finished` (part 14.11). Not part
/// of the frozen types: `application` already distinguishes applied and
/// source-drift-blocked; a `Ready`, unapplied run whose last stored verdict
/// is `Refresh`/`Stop` is shown as `blocked` too, since that is exactly why
/// auto-apply has not (yet) applied it.
/// The view's line for one run: the shared Work line from its stored
/// validity. Native runs' verdicts are never recomputed here.
fn describe(run: &RunRecord) -> (WorkLine, Option<Decision>) {
    let validity = run.coherence.as_ref().and_then(|c| c.validity.as_ref());
    (work_line(run, validity), validity.map(|v| v.decision))
}

/// Runs whose `source_path` is `root` and that are still active or finished
/// within the last hour (part 4/14.11 of the view), sorted for a stable
/// redraw.
fn view_rows(runs: &[RunRecord]) -> Vec<&RunRecord> {
    let now = Utc::now();
    let mut rows: Vec<&RunRecord> = runs
        .iter()
        .filter(|run| {
            run.outcome.lifecycle != LifecycleState::Finished
                || run
                    .completed_at
                    .is_some_and(|at| now - at < chrono::Duration::hours(1))
        })
        .collect();
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    rows
}

/// Print the project view. `--json` emits one `{"type":"work",...}` object
/// per run whose displayed fields changed since the last tick; otherwise the
/// whole block is printed, redrawn in place on a TTY and appended as lines
/// otherwise.
fn render_view(
    runs: &[RunRecord],
    json: bool,
    shown: &mut HashMap<String, String>,
    last_block: &mut Vec<String>,
) {
    let rows = view_rows(runs);

    if json {
        for run in rows {
            let (line, decision) = describe(run);
            let key = line.to_string();
            if shown.get(&run.id) != Some(&key) {
                println!(
                    "{}",
                    serde_json::json!({
                        "type": "work",
                        "run_id": run.id,
                        "agent": line.agent,
                        "verdict": decision,
                        "state": line.state,
                        "reason": line.reason.as_deref().unwrap_or("—"),
                        "origin": line.origin,
                        "s0": line.s0,
                        "verification": line.verification,
                        "review": line.review,
                        "applied_by": line.applied_by,
                        "overridden": line.verdict == "overridden",
                    })
                );
                shown.insert(run.id.clone(), key);
            }
        }
        return;
    }

    let lines: Vec<String> = rows
        .iter()
        .map(|run| {
            let id8 = &run.id[..8.min(run.id.len())];
            format!("{id8} · {}", describe(run).0)
        })
        .collect();

    // Rendered every tick, so redraw only when the block would differ.
    if *last_block == lines {
        return;
    }
    let tty = std::io::stdout().is_terminal();
    if tty && !last_block.is_empty() {
        print!("\x1b[{}A\x1b[0J", last_block.len());
    }
    for line in &lines {
        println!("{line}");
    }
    *last_block = lines;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_owner_state_maps_identity_outcomes() {
        assert_eq!(live_owner_state(None), OwnerState::Unknown);
    }
}
