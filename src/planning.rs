//! Bounded local planning contracts. Proposals never confer execution authority.
use crate::{AllocationDecision, CheckResult, GoalExecution, ResourceTier, RunRecord, source};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PlanningConfig {
    /// Owner-selected existing verification files, restored from B0 for checks.
    pub verification_paths: Vec<String>,
    pub routine: Vec<RoutineScope>,
}

impl PlanningConfig {
    pub fn is_empty(&self) -> bool {
        self.verification_paths.is_empty() && self.routine.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutineScope {
    pub paths: Vec<String>,
    pub checks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub version: u32,
    pub tasks: Vec<TaskSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskSpec {
    pub id: String,
    pub objective: String,
    pub read: Vec<String>,
    pub write: Vec<String>,
    pub acceptance: Vec<String>,
    pub checks: Vec<String>,
    pub prerequisites: Vec<String>,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Planning {
    pub version: u32,
    pub revision: String,
    pub approved_checks: std::collections::BTreeMap<String, String>,
    pub owner_policy: PlanningConfig,
    pub plan: Option<Plan>,
    pub tasks: Vec<Task>,
    pub snapshots: Vec<Snapshot>,
    pub artifacts: Vec<Artifact>,
    pub final_candidate: Option<String>,
    pub final_patch_sha256: Option<String>,
    pub root_checks: Vec<CheckResult>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub state: TaskState,
    pub decision: AllocationDecision,
    pub feature_provenance: String,
    pub input: Option<String>,
    pub artifact: Option<String>,
    pub pending_prerequisite: Option<String>,
    pub extra_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Pending,
    Working,
    Waiting,
    Integrated,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Snapshot {
    pub id: String,
    pub path: PathBuf,
    pub fingerprint: String,
    pub parent: Option<String>,
    pub contribution: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub task_id: Option<String>,
    pub attempt_id: String,
    pub input: String,
    pub output: String,
    pub patch: PathBuf,
    pub patch_sha256: String,
    pub manifest: PathBuf,
    pub checks: Vec<CheckResult>,
    pub verification_changes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DependencyReport {
    pub version: u32,
    pub plan_revision: String,
    pub task_id: String,
    pub attempt_id: String,
    pub generation: u32,
    pub prerequisite: Option<String>,
    pub assistance: Option<String>,
}

pub fn catalog(commands: &[String]) -> std::collections::BTreeMap<String, String> {
    commands
        .iter()
        .map(|c| {
            (
                format!(
                    "check-{}",
                    &crate::commands::digest(c).expect("string serialization")[..16]
                ),
                c.clone(),
            )
        })
        .collect()
}

pub fn path(root: &Path, value: &str) -> Result<PathBuf> {
    ensure!(
        !value.is_empty() && value.len() <= 512,
        "empty or oversized path"
    );
    let relative = Path::new(value);
    ensure!(
        relative
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
            && !value.contains('\\'),
        "unsafe repository-relative path: {value}"
    );
    ensure!(
        !relative
            .components()
            .any(|c| matches!(c.as_os_str().to_str(), Some(".git" | ".dispatch"))),
        "reserved path: {value}"
    );
    let mut current = root.to_path_buf();
    for part in relative.components() {
        current.push(part);
        if let Ok(meta) = fs::symlink_metadata(&current) {
            ensure!(
                !meta.file_type().is_symlink(),
                "scope contains a symlink: {value}"
            );
        }
    }
    Ok(current)
}

pub fn contains(scope: &str, file: &str) -> bool {
    Path::new(file).starts_with(scope)
}

pub fn validate_policy(root: &Path, config: &crate::Config) -> Result<()> {
    ensure!(
        !config.checks.verify.is_empty(),
        "planned execution requires owner-approved executable checks"
    );
    ensure!(
        !config.planning.verification_paths.is_empty(),
        "planned execution requires planning.verification_paths identifying the original tests and check scripts"
    );
    ensure!(
        config.planning.verification_paths.len() <= 64 && config.planning.routine.len() <= 32,
        "planning policy too large"
    );
    for name in &config.planning.verification_paths {
        ensure!(
            path(root, name)?.exists(),
            "verification path is missing: {name}"
        );
    }
    let approved = catalog(&config.checks.verify);
    for rule in &config.planning.routine {
        ensure!(
            !rule.paths.is_empty() && !rule.checks.is_empty(),
            "routine scope requires paths and approved check references"
        );
        for name in &rule.paths {
            path(root, name)?;
        }
        ensure!(
            rule.checks.iter().all(|c| approved.contains_key(c)),
            "routine scope refers to an unapproved check"
        );
    }
    Ok(())
}

fn bounded(values: &[String], max: usize) -> bool {
    !values.is_empty()
        && values.len() <= max
        && values
            .iter()
            .all(|s| !s.trim().is_empty() && s.len() <= 2048)
}

pub fn parse(text: &str, root: &Path, planning: &Planning, remaining: u32) -> Result<Plan> {
    ensure!(text.len() <= 32768, "plan exceeds 32 KiB");
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Envelope {
        dispatch_plan: Plan,
    }
    let plan = serde_json::from_str::<Envelope>(text)
        .context("final result must be one dispatch_plan envelope")?
        .dispatch_plan;
    validate(&plan, root, planning, remaining)?;
    Ok(plan)
}

pub fn validate(plan: &Plan, root: &Path, planning: &Planning, remaining: u32) -> Result<()> {
    ensure!(
        plan.version == 1 && (1..=4).contains(&plan.tasks.len()),
        "plan must contain one to four tasks using version 1"
    );
    ensure!(
        plan.tasks.len() <= remaining as usize,
        "plan exceeds remaining initial-invocation budget"
    );
    let mut ids = BTreeSet::new();
    for t in &plan.tasks {
        ensure!(
            !t.id.is_empty()
                && t.id.len() <= 48
                && t.id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                && ids.insert(&t.id),
            "invalid or duplicate task ID"
        );
        ensure!(
            !t.objective.trim().is_empty()
                && t.objective.len() <= 4096
                && bounded(&t.read, 32)
                && bounded(&t.write, 32)
                && bounded(&t.acceptance, 8)
                && bounded(&t.checks, 16)
                && bounded(&t.inputs, 8)
                && bounded(&t.outputs, 8),
            "task {} has missing or oversized contract fields",
            t.id
        );
        for p in t.read.iter().chain(&t.write) {
            path(root, p)?;
        }
        ensure!(
            t.checks
                .iter()
                .all(|c| planning.approved_checks.contains_key(c))
                && t.checks.iter().collect::<BTreeSet<_>>().len() == t.checks.len(),
            "task {} requires unknown or duplicate checks",
            t.id
        );
    }
    for t in &plan.tasks {
        ensure!(
            t.prerequisites.len() <= 3
                && t.prerequisites.iter().collect::<BTreeSet<_>>().len() == t.prerequisites.len()
                && t.prerequisites
                    .iter()
                    .all(|p| p != &t.id && ids.contains(p)),
            "invalid dependency for {}",
            t.id
        );
    }
    order(plan)?;
    for (i, a) in plan.tasks.iter().enumerate() {
        for b in &plan.tasks[i + 1..] {
            let overlap = a
                .write
                .iter()
                .any(|p| b.write.iter().any(|q| contains(p, q) || contains(q, p)));
            ensure!(
                !overlap || depends(plan, &a.id, &b.id) || depends(plan, &b.id, &a.id),
                "shared write scopes require an explicit dependency: {} / {}",
                a.id,
                b.id
            );
        }
    }
    Ok(())
}

pub fn depends(plan: &Plan, task: &str, prerequisite: &str) -> bool {
    let mut visited = BTreeSet::new();
    let mut pending = vec![task];
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        if let Some(t) = plan.tasks.iter().find(|t| t.id == id) {
            for p in &t.prerequisites {
                if p == prerequisite {
                    return true;
                }
                pending.push(p);
            }
        }
    }
    false
}

pub fn order(plan: &Plan) -> Result<Vec<String>> {
    let mut done = Vec::new();
    while done.len() < plan.tasks.len() {
        let next = plan
            .tasks
            .iter()
            .filter(|t| !done.contains(&t.id) && t.prerequisites.iter().all(|p| done.contains(p)))
            .min_by_key(|t| &t.id)
            .context("cyclic task dependencies")?;
        done.push(next.id.clone());
    }
    Ok(done)
}

pub fn lane(task: &TaskSpec, root: &Path, policy: &PlanningConfig) -> (ResourceTier, String) {
    // A model's natural-language labels cannot establish risk or check relevance.
    let routine = task.write.len() <= 2
        && task.write.iter().all(|p| root.join(p).is_file())
        && policy.routine.iter().any(|r| {
            task.write
                .iter()
                .all(|p| r.paths.iter().any(|s| contains(s, p)))
                && r.checks.iter().all(|c| task.checks.contains(c))
        });
    if routine {
        (ResourceTier::Light,"validated existing-file scope + owner routine/check mapping; acceptance text remains a proposal".into())
    } else if task.write.len() > 4
        || task
            .write
            .iter()
            .any(|p| root.join(p).is_dir() || p.ends_with(".h") || p.ends_with(".proto"))
    {
        (
            ResourceTier::Strong,
            "shared interface, directory, or broad write contract; planner difficulty not trusted"
                .into(),
        )
    } else {
        (
            ResourceTier::Standard,
            "bounded validated file contract; risk/check relevance unknown".into(),
        )
    }
}

pub fn planning(run: &RunRecord) -> Option<&Planning> {
    run.phase3.as_ref()?.planning.as_ref()
}

pub(crate) fn stop_active(run: &mut RunRecord, reason: &str) {
    if let Some(p) = run.phase3.as_mut().and_then(|p| p.planning.as_mut()) {
        p.error.get_or_insert_with(|| reason.to_owned());
        for task in &mut p.tasks {
            if matches!(task.state, TaskState::Working | TaskState::Waiting) {
                task.state = TaskState::Failed;
            }
        }
    }
}

/// Persisted policy excludes mutable outcome fields. Historical direct rows stay direct.
fn policy_json(policy: &GoalExecution) -> serde_json::Value {
    let p = policy.planning.as_ref().unwrap();
    serde_json::json!({"version":p.version,"revision":p.revision,"maximum":policy.max_invocations,"deadline":policy.deadline_at,"no_retry":policy.no_retry,"harness":policy.fixed_harness,"model":policy.fixed_model,"effort":policy.fixed_effort,"checks":p.approved_checks,"owner_policy":p.owner_policy})
}

pub(crate) fn persist(c: &Connection, run: &RunRecord) -> Result<()> {
    let Some(p) = planning(run) else {
        return Ok(());
    };
    let policy = serde_json::to_string(&policy_json(run.phase3.as_ref().unwrap()))?;
    let old: Option<String> = c
        .query_row(
            "SELECT policy_json FROM planned_goals WHERE run_id=?1",
            [&run.id],
            |r| r.get(0),
        )
        .optional()?;
    ensure!(
        old.as_ref().is_none_or(|v| v == &policy),
        "planned goal policy is immutable"
    );
    c.execute(
        "INSERT OR IGNORE INTO planned_goals(run_id,revision,policy_json) VALUES (?1,?2,?3)",
        params![run.id, p.revision, policy],
    )?;
    if let Some(plan) = &p.plan {
        let json = serde_json::to_string(plan)?;
        let old: Option<String> = c.query_row(
            "SELECT plan_json FROM planned_goals WHERE run_id=?1",
            [&run.id],
            |r| r.get(0),
        )?;
        ensure!(
            old.as_ref().is_none_or(|v| v == &json),
            "validated plan is immutable; dependency proposals are separate edges"
        );
        c.execute(
            "UPDATE planned_goals SET plan_json=?2 WHERE run_id=?1",
            params![run.id, json],
        )?;
    }
    for t in &p.tasks {
        c.execute("INSERT INTO planned_tasks(run_id,id,state,task_json) VALUES (?1,?2,?3,?4) ON CONFLICT(run_id,id) DO UPDATE SET state=excluded.state,task_json=excluded.task_json",params![run.id,t.id,format!("{:?}",t.state),serde_json::to_string(t)?])?;
    }
    for a in &p.artifacts {
        immutable(
            c,
            "planned_artifacts",
            &run.id,
            &a.id,
            &serde_json::to_string(a)?,
        )?;
    }
    for s in &p.snapshots {
        immutable(
            c,
            "planned_snapshots",
            &run.id,
            &s.id,
            &serde_json::to_string(s)?,
        )?;
    }
    Ok(())
}

fn immutable(c: &Connection, table: &str, run: &str, id: &str, json: &str) -> Result<()> {
    let old: Option<String> = c
        .query_row(
            &format!("SELECT payload_json FROM {table} WHERE run_id=?1 AND id=?2"),
            params![run, id],
            |r| r.get(0),
        )
        .optional()?;
    ensure!(
        old.as_deref().is_none_or(|s| s == json),
        "immutable lineage changed"
    );
    c.execute(
        &format!("INSERT OR IGNORE INTO {table}(run_id,id,payload_json) VALUES (?1,?2,?3)"),
        params![run, id, json],
    )?;
    Ok(())
}

/// Runs inside the existing final launch transaction, before spawn uncertainty.
pub(crate) fn fence(c: &Connection, attempt: &str) -> Result<()> {
    let json: Option<String> = c.query_row(
        "SELECT r.run_projection_json FROM runs r JOIN attempts a ON a.run_id=r.id WHERE a.id=?1",
        [attempt],
        |r| r.get(0),
    )?;
    let Some(json) = json else { return Ok(()) };
    let run: RunRecord = serde_json::from_str(&json)?;
    let Some(p) = planning(&run) else {
        return Ok(());
    };
    let policy = run.phase3.as_ref().unwrap();
    let frozen: String = c.query_row(
        "SELECT policy_json FROM planned_goals WHERE run_id=?1",
        [&run.id],
        |r| r.get(0),
    )?;
    ensure!(
        frozen == serde_json::to_string(&policy_json(policy))?,
        "persisted planning policy mismatch"
    );
    ensure!(
        chrono::Utc::now() < policy.deadline_at
            && run.outcome.lifecycle != crate::LifecycleState::Finished
            && run.outcome.work_result == crate::WorkResult::Pending,
        "planned goal closed or original deadline expired"
    );
    let a = run
        .attempts
        .iter()
        .find(|a| a.id == attempt)
        .context("attempt missing from committed plan")?;
    let consumed: u32 = c.query_row("SELECT COUNT(*) FROM planned_invocations WHERE run_id=?1 AND knowledge NOT IN ('launch_intent_committed','launch_not_started')",[&run.id],|r|r.get(0))?;
    let cap = policy
        .max_invocations
        .min(p.plan.as_ref().map_or(1, |p| p.tasks.len() as u32 + 2))
        .min(6);
    ensure!(consumed < cap, "planned invocation budget exhausted");
    let key = if a.role == "planner" {
        ensure!(p.plan.is_none(), "cannot replan");
        "planner".to_owned()
    } else if a.role == "task" {
        let task = a.detail.task_id.as_ref().context("task identity missing")?;
        ensure!(
            p.tasks
                .iter()
                .any(|t| &t.id == task && t.state == TaskState::Working),
            "task is not runnable"
        );
        format!("task:{task}")
    } else {
        let missing_initial = p
            .tasks
            .iter()
            .filter(|t| {
                !run.attempts
                    .iter()
                    .any(|a| a.role == "task" && a.detail.task_id.as_deref() == Some(&t.id))
            })
            .count() as u32;
        ensure!(
            consumed + missing_initial < cap,
            "extra invocation would consume a required task's budget"
        );
        ensure!(
            a.role == "continuation" || (a.role == "repair" && !policy.no_retry),
            "unauthorized extra invocation"
        );
        "extra".into()
    };
    let used: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM planned_invocations WHERE run_id=?1 AND slot=?2)",
        params![run.id, key],
        |r| r.get(0),
    )?;
    ensure!(!used, "planned slot already reserved; no automatic replay");
    c.execute("INSERT INTO planned_invocations(attempt_id,run_id,slot,knowledge) VALUES (?1,?2,?3,'spawn_may_have_occurred')",params![attempt,run.id,key])?;
    Ok(())
}

pub fn verify_snapshot(snapshot: &Snapshot) -> Result<()> {
    ensure!(
        source::fingerprint_tree(&snapshot.path)? == snapshot.fingerprint,
        "immutable snapshot hash mismatch: {}",
        snapshot.id
    );
    Ok(())
}

pub(crate) fn verify_artifact(run: &RunRecord, artifact: &Artifact) -> Result<()> {
    use sha2::{Digest, Sha256};
    ensure!(
        !artifact.checks.is_empty()
            && artifact
                .checks
                .iter()
                .all(|c| c.status == crate::CheckStatus::Passed && c.exit_code == Some(0)),
        "prerequisite lacks passing checks"
    );
    ensure!(
        hex::encode(Sha256::digest(fs::read(&artifact.patch)?)) == artifact.patch_sha256,
        "artifact patch hash mismatch"
    );
    let disk: Artifact = serde_json::from_str(&fs::read_to_string(&artifact.manifest)?)?;
    ensure!(
        serde_json::to_value(&disk)? == serde_json::to_value(artifact)?,
        "artifact manifest mismatch"
    );
    let output = planning(run)
        .context("planning lineage missing")?
        .snapshots
        .iter()
        .find(|s| {
            s.id == artifact.output
                && s.parent.as_deref() == Some(&artifact.input)
                && s.contribution.as_deref() == Some(&artifact.id)
        })
        .context("prerequisite not integrated")?;
    verify_snapshot(output)
}

/// A final delivery is a separate immutable root artifact, never a last-child patch.
pub(crate) fn verify_delivery(run: &RunRecord) -> Result<()> {
    use sha2::{Digest, Sha256};
    let Some(p) = planning(run) else {
        return Ok(());
    };
    let candidate = run
        .candidates
        .first()
        .context("planned goal has no complete delivery")?;
    ensure!(
        p.final_candidate.as_ref() == Some(&candidate.id),
        "planned delivery identity mismatch"
    );
    for artifact in &p.artifacts {
        verify_artifact(run, artifact)?;
    }
    ensure!(
        p.final_patch_sha256.as_ref()
            == Some(&hex::encode(Sha256::digest(fs::read(
                &candidate.diff_path
            )?))),
        "final patch hash mismatch"
    );
    verify_snapshot(p.snapshots.last().context("final snapshot missing")?)
}

/// Publish bytes once before committing artifact readiness. This is not a disk/DB transaction.
pub(crate) fn publish(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let parent = path.parent().context("artifact parent missing")?;
    fs::create_dir_all(parent)?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    staged.write_all(bytes)?;
    staged.as_file().sync_all()?;
    staged.persist_noclobber(path)?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}
