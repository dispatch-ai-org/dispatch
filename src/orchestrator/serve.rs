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
    collections::{HashMap, HashSet},
    fs,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result};
use chrono::Utc;
use sha2::{Digest, Sha256};

use super::{ApplyOutcome, WorkLine, apply, auto_apply, persist_event, work_line};
use crate::{
    ApplicationState, Config, Decision, EventRecord, LifecycleState, OwnerState, ReviewState,
    RunMode, RunRecord, SourceKind, WorkResult,
    coherence::{self, WorkView, world},
    db::Database,
    lock::{OperationLock, shutdown_signal},
    process::{IdentityState, ProcessIdentity, identity_state},
    source,
    state::State,
};

/// `dispatch serve [--root <path>] [--json]`. Loops until Ctrl+C, SIGTERM or
/// SIGHUP; a second `serve` on the same root refuses to start. The owner
/// decides and records; this loop only renders what it did.
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

    let mut owner = Owner::new(root, kind)?;
    let mut view = View::default();

    loop {
        tokio::select! {
            _ = ticker.tick() => {}
            _ = shutdown_signal() => return Ok(()),
        }
        let tick = owner.tick(state);
        if let Some(error) = &tick.error {
            eprintln!("serve: {error}");
        }
        if json
            && tick.moved
            && let Some(digest) = &tick.digest
        {
            println!("{}", serde_json::json!({"type": "world", "digest": digest}));
        }
        // Render every tick: a run attached, finished or applied by another
        // process changes the view without any verdict or apply of our own.
        // `render` prints only what differs from what is already shown.
        view.render(&tick.runs, json);
        let _ = io::stdout().flush();
    }
}

/// The project owner for one integration root: everything `serve` decides
/// and records, and nothing it prints. One `tick` observes the world, adopts
/// orphaned attached Work, re-evaluates unowned Work and auto-applies what
/// policy allows, all under the same locks as before.
pub(crate) struct Owner {
    root: PathBuf,
    kind: SourceKind,
    last_signal: Option<world::Signal>,
    /// Each followed run's work signal when it was last evaluated.
    work: HashMap<String, String>,
    scratch: tempfile::TempDir,
}

/// What one tick did, and the root's runs as they stand after it.
#[derive(Default)]
pub(crate) struct Tick {
    pub moved: bool,
    pub adopted: bool,
    pub persisted: bool,
    pub applied: bool,
    pub digest: Option<String>,
    pub runs: Vec<RunRecord>,
    /// The first error of the tick. SQLite busy and other transient
    /// failures are retried on the next tick rather than ending the loop.
    pub error: Option<String>,
}

impl Tick {
    fn report(&mut self, error: &anyhow::Error) {
        self.error.get_or_insert_with(|| format!("{error:#}"));
    }
}

impl Owner {
    pub(crate) fn new(root: PathBuf, kind: SourceKind) -> Result<Self> {
        Ok(Self {
            root,
            kind,
            last_signal: None,
            work: HashMap::new(),
            scratch: tempfile::Builder::new()
                .prefix("dispatch-owner-")
                .tempdir()
                .context("failed to create the owner's scratch directory")?,
        })
    }

    pub(crate) fn tick(&mut self, state: &State) -> Tick {
        let mut tick = Tick::default();
        let signal_now = match world::signal(&self.root, &self.kind) {
            Ok(signal) => Some(signal),
            Err(error) => {
                tick.report(&error);
                None
            }
        };
        tick.moved = match (&signal_now, &self.last_signal) {
            (Some(now), Some(previous)) => now != previous,
            (Some(_), None) => true,
            (None, _) => false,
        };
        if let Some(signal_now) = &signal_now {
            self.last_signal = Some(signal_now.clone());
        }

        let runs = self.load(state, &mut tick);
        tick.adopted = adopt_orphans(state, &runs, &mut tick);
        let moved = tick.moved || tick.adopted;
        self.reevaluate(state, &runs, &mut tick, moved);
        for run in ready_for_auto_apply(&runs) {
            match auto_apply(state, &run.id) {
                Ok(ApplyOutcome::Applied { .. }) => {
                    tick.applied = true;
                    // The apply is the doorbell: re-observe immediately
                    // rather than waiting for the next tick's signal.
                    self.reevaluate(state, &runs, &mut tick, true);
                }
                Ok(_) => {}
                Err(error) => tick.report(&error),
            }
        }
        if tick.digest.is_none() {
            tick.digest = signal_now.map(|signal| signal.0);
        }
        tick.runs = if tick.adopted || tick.persisted || tick.applied {
            self.load(state, &mut tick)
        } else {
            runs
        };
        tick
    }

    fn load(&self, state: &State, tick: &mut Tick) -> Vec<RunRecord> {
        load_source_runs(state, &self.root).unwrap_or_else(|error| {
            tick.report(&error);
            Vec::new()
        })
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
/// disk/DB. A run's source never changes, so the projected metadata decides
/// which runs to load; other projects' runs are never loaded.
pub(crate) fn load_source_runs(state: &State, root: &Path) -> Result<Vec<RunRecord>> {
    let mut runs = Vec::new();
    for path in state.list_metadata_paths()? {
        let projected: RunRecord = serde_json::from_slice(&fs::read(&path)?)
            .with_context(|| format!("invalid metadata at {}", path.display()))?;
        if projected.source_path == root {
            runs.push(state.load_run(&projected.id)?);
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
fn adopt_orphans(state: &State, runs: &[RunRecord], errors: &mut Tick) -> bool {
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

impl Owner {
    /// Re-evaluate every active attached run whose live owner state is not
    /// `Live` (a live wrapper owns those; part 14.4 step (b)/(c)) when the
    /// world moved or the work did: observe the world against that run's own
    /// baseline, evaluate, drop `analysis_uncertain`-only reasons, and persist
    /// through `apply::persist_verdict` what is worth recording. The first
    /// world digest observed is the tick's: `observe` hashes the current tree
    /// whichever baseline it is given.
    fn reevaluate(
        &mut self,
        state: &State,
        runs: &[RunRecord],
        tick: &mut Tick,
        world_moved: bool,
    ) {
        let (root, kind) = (self.root.as_path(), &self.kind);
        let mut followed = HashSet::new();
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
            // The work's signal is its patch so far, taken without the run's
            // lock into a scratch file: `finish` takes that lock without
            // waiting, so the owner holds it only when something moved.
            let scratch = self.scratch.path().join(format!("{}.patch", candidate.id));
            let work =
                source::snapshot_delta(&candidate.baseline_path, &attachment.workspace, &scratch)
                    .and_then(|()| Ok(hex::encode(Sha256::digest(fs::read(&scratch)?))));
            let work = match work {
                Ok(work) => work,
                Err(error) => {
                    tick.report(&error);
                    continue;
                }
            };
            followed.insert(candidate.id.clone());
            if !world_moved && self.work.get(&candidate.id) == Some(&work) {
                continue;
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
                    tick.report(&error);
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
                tick.report(&error);
                continue;
            }
            let world = match world::observe(root, &run.baseline_path, &run.baseline_commit, kind) {
                Ok(world) => world,
                Err(error) => {
                    tick.report(&error);
                    continue;
                }
            };
            tick.digest.get_or_insert_with(|| world.digest.clone());
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
                    tick.report(&error);
                    continue;
                }
            };
            // Compared with the verdict stored on the run, not with anything
            // this process remembers: a restarted owner still records a change.
            let validity = coherence::watch::settle(validity);
            let stored = run.coherence.as_ref().and_then(|c| c.validity.as_ref());
            if !coherence::watch::worth_recording(stored, &validity) {
                self.work.insert(run.id.clone(), work);
                continue;
            }
            let database = match Database::open(state.db_path()) {
                Ok(database) => database,
                Err(error) => {
                    tick.report(&error);
                    continue;
                }
            };
            if let Err(error) = apply::persist_verdict(state, &database, &mut run, &validity) {
                tick.report(&error);
                continue;
            }
            self.work.insert(run.id.clone(), work);
            tick.persisted = true;
        }
        self.work.retain(|id, _| followed.contains(id));
    }
}

/// Every attached run ready to auto-apply: finished, `Ready`, unreviewed,
/// unapplied, and `capabilities.integrate`.
fn ready_for_auto_apply(runs: &[RunRecord]) -> impl Iterator<Item = &RunRecord> {
    runs.iter().filter(|run| {
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
}

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

/// The project view: what is shown, so each tick prints only what changed.
/// Shared by `serve` in the foreground and `watch`.
#[derive(Default)]
pub(crate) struct View {
    shown: HashMap<String, String>,
    last_block: Vec<String>,
}

impl View {
    /// Print the project view. `--json` emits one `{"type":"work",...}`
    /// object per run whose displayed fields changed since the last tick;
    /// otherwise the whole block is printed, redrawn in place on a TTY and
    /// appended as lines otherwise.
    pub(crate) fn render(&mut self, runs: &[RunRecord], json: bool) {
        let (shown, last_block) = (&mut self.shown, &mut self.last_block);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_owner_state_maps_identity_outcomes() {
        assert_eq!(live_owner_state(None), OwnerState::Unknown);
    }
}
