//! Scoped foreground commands. Transport data never establishes authority.
pub(crate) mod inspection;
use crate::{
    Config, EventRecord, LifecycleState, QuestionState, RunRecord, WorkResult,
    config::ResourceConfig,
    db::Database,
    orchestrator::{self, OperationLock, RunRequest},
    state::State,
};
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    future::Future,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::sync::oneshot;

pub const OPERATIONS: &[&str] = &[
    "initialize",
    "submit",
    "status",
    "result",
    "capacity",
    "events",
    "subscribe",
    "await",
    "answer",
    "cancel",
    "recover",
    "artifact",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    #[serde(default)]
    pub allow_plan: bool,
    pub principal: String,
    pub owner_uid: u32,
    pub source: PathBuf,
    pub state_root: PathBuf,
    pub config_digest: String,
    #[serde(default)]
    pub private_policy_revision: u64,
    pub profiles: Vec<Value>,
    pub timeout_secs: u64,
    pub max_invocations: u32,
    pub allow_unsafe_local: bool,
    pub delegate_factual: bool,
    pub expires_at: DateTime<Utc>,
}

pub fn digest(value: &impl Serialize) -> Result<String> {
    // serde_json's default map is sorted; round-trip canonicalizes struct/map key order.
    let value = serde_json::to_value(value)?;
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(&value)?)))
}

fn uid() -> u32 {
    crate::orchestrator::phase3::local_uid()
}

/// Explicit local-owner provisioning; never callable by the control protocol.
/// The key stays in protected state and is passed only through an inherited FD.
pub fn grant(
    state: &State,
    source: &Path,
    timeout_secs: u64,
    max_invocations: u32,
    allow_unsafe_local: bool,
    delegate_factual: bool,
) -> Result<PathBuf> {
    grant_mode(
        state,
        source,
        timeout_secs,
        max_invocations,
        allow_unsafe_local,
        delegate_factual,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn grant_mode(
    state: &State,
    source: &Path,
    timeout_secs: u64,
    max_invocations: u32,
    allow_unsafe_local: bool,
    delegate_factual: bool,
    allow_plan: bool,
) -> Result<PathBuf> {
    ensure!(
        (1..=86400).contains(&timeout_secs)
            && (1..=if allow_plan { 6 } else { 2 }).contains(&max_invocations),
        "invalid grant limits"
    );
    state.initialize()?;
    let source = source.canonicalize()?;
    let root = state.root.canonicalize()?;
    ensure!(!root.starts_with(&source), "state must be outside source");
    private_root(&root)?;
    let (config, _) = Config::discover(&source, None)?;
    config.validate()?;
    ensure!(
        config.execution.forwarded_env.is_empty(),
        "machine grants do not authorize environment forwarding"
    );
    ensure!(
        config.execution.backend != "local" || allow_unsafe_local,
        "authorization_required: local execution requires --allow-unsafe-local"
    );
    let resources = ResourceConfig::load(&root)?;
    ensure!(
        resources.allocation_enabled && resources.capacity.admission,
        "control requires allocation and shared admission enabled"
    );
    let profiles: Vec<Value> = resources
        .profiles
        .iter()
        .filter(|p| p.eligibility().is_ok() && p.runtime == config.execution.backend)
        .map(serde_json::to_value)
        .collect::<std::result::Result<_, _>>()?;
    ensure!(!profiles.is_empty(), "no included authorized resources");
    let principal = ulid::Ulid::new().to_string();
    if allow_plan {
        ensure!(
            max_invocations >= 2,
            "planned grant needs room for a planner and at least one task"
        );
        crate::planning::validate_policy(&source, &config)?;
    }
    let scope = Scope {
        allow_plan,
        principal: principal.clone(),
        owner_uid: uid(),
        source: source.clone(),
        state_root: root.clone(),
        config_digest: digest(&config)?,
        private_policy_revision: crate::private_evidence::policy_revision(state, &source)?,
        profiles,
        timeout_secs: timeout_secs.min(config.execution.timeout_secs),
        max_invocations,
        allow_unsafe_local,
        delegate_factual,
        expires_at: Utc::now() + chrono::TimeDelta::hours(24),
    };
    let secret = hex::encode(rand::random::<[u8; 32]>());
    let db = Database::open_control(state.db_path())?;
    db.connection().execute(
        "INSERT INTO control_grants(id,secret_hash,scope_json) VALUES (?1,?2,?3)",
        params![principal, digest(&secret)?, serde_json::to_string(&scope)?],
    )?;
    let dir = root.join("control-grants");
    fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    }
    let path = dir.join(format!("{principal}.key"));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    file.write_all(secret.as_bytes())?;
    file.sync_all()?;
    Ok(path)
}

fn private_root(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = fs::metadata(path)?;
        ensure!(
            m.uid() == uid() && m.mode() & 0o077 == 0,
            "control requires a private owner-only state directory (mode 0700)"
        );
    }
    Ok(())
}

pub struct Session {
    pub scope: Arc<Scope>,
    pub id: String,
    pub read_only: bool,
    _lock: Option<OperationLock>,
}
impl Session {
    #[cfg(unix)]
    pub fn from_fd(state: &State, fd: i32, read_only: bool) -> Result<Self> {
        use std::os::fd::FromRawFd;
        ensure!(fd >= 3, "grant handle must be separate from stdio");
        // Require a seekable protected regular file, not an indefinitely blocking grant pipe.
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        ensure!(
            unsafe { libc::fstat(fd, stat.as_mut_ptr()) } == 0,
            "invalid grant handle"
        );
        let stat = unsafe { stat.assume_init() };
        ensure!(
            stat.st_mode & libc::S_IFMT == libc::S_IFREG
                && stat.st_uid == uid()
                && stat.st_mode & 0o077 == 0
                && stat.st_size == 64,
            "grant handle must be an owner-only 64-byte regular file"
        );
        let mut file = unsafe { fs::File::from_raw_fd(fd) };
        let mut secret = String::new();
        Read::by_ref(&mut file)
            .take(65)
            .read_to_string(&mut secret)?;
        drop(file); // Do not inherit the capability into a harness.
        private_root(&state.root)?;
        let db = Database::open_read_only(state.db_path())?;
        let raw: String = db
            .connection()
            .query_row(
                "SELECT scope_json FROM control_grants WHERE secret_hash=?1",
                [digest(&secret)?],
                |r| r.get(0),
            )
            .optional()?
            .context("unauthorized grant")?;
        let scope: Scope = serde_json::from_str(&raw)?;
        scope.validate(state)?;
        let lock = if read_only {
            None
        } else {
            Some(OperationLock::acquire(
                &state
                    .root
                    .join("control-grants")
                    .join(format!("{}.lock", scope.principal)),
                "grant already has a foreground owner",
            )?)
        };
        Ok(Self {
            scope: Arc::new(scope),
            id: ulid::Ulid::new().to_string(),
            read_only,
            _lock: lock,
        })
    }
}
impl Scope {
    pub fn validate(&self, state: &State) -> Result<()> {
        ensure!(
            self.owner_uid == uid()
                && self.state_root == state.root.canonicalize()?
                && self.source == self.source.canonicalize()?,
            "unauthorized scope"
        );
        ensure!(
            Utc::now() < self.expires_at,
            "grant_expired: request IDs cannot be reused with this expired grant"
        );
        Ok(())
    }
    pub fn run(&self, state: &State, id: &str) -> Result<RunRecord> {
        self.validate(state)?;
        let db = Database::open_read_only(state.db_path())?;
        let allowed: bool = db.connection().query_row(
            "SELECT EXISTS(SELECT 1 FROM control_runs WHERE run_id=?1 AND principal=?2)",
            params![id, self.principal],
            |r| r.get(0),
        )?;
        ensure!(allowed, "unauthorized run");
        let run = db
            .committed_run_projection(id)?
            .context("not_ready: no committed run")?;
        ensure!(
            run.source_path.canonicalize()? == self.source,
            "unauthorized source lineage"
        );
        Ok(run)
    }
    pub fn policy(&self, state: &State) -> Result<()> {
        self.validate(state)?;
        let (config, _) = Config::discover(&self.source, None)?;
        ensure!(
            digest(&config)? == self.config_digest,
            "authorization_required: project policy changed; issue a new grant for new work"
        );
        Ok(())
    }
    pub fn receipt(&self, state: &State, id: &str, payload: &Value) -> Result<Option<Value>> {
        self.validate(state)?;
        let db = Database::open_read_only(state.db_path())?;
        let row:Option<(String,String,String)>=db.connection().query_row("SELECT payload_digest,run_id,reply_json FROM command_receipts WHERE principal=?1 AND request_id=?2",params![self.principal,id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        row.map(|(stored, run, reply)| {
            self.run(state, &run)?;
            ensure!(
                stored == digest(payload)?,
                "request_conflict: request ID has different content"
            );
            Ok(serde_json::from_str(&reply)?)
        })
        .transpose()
    }
}

pub(crate) struct CommandContext {
    pub scope: Arc<Scope>,
    pub session_id: String,
    pub request_id: String,
    pub payload_digest: String,
    pub operation: String,
    pub committed: Mutex<Option<oneshot::Sender<Value>>>,
}
tokio::task_local! { static CALLER: CommandContext; }
pub(crate) async fn scoped<F: Future>(context: CommandContext, work: F) -> F::Output {
    CALLER.scope(context, work).await
}
pub(crate) fn actor() -> Option<String> {
    CALLER
        .try_with(|c| format!("machine:{}", c.scope.principal))
        .ok()
}
pub(crate) fn private_policy_revision() -> Option<u64> {
    CALLER.try_with(|c| c.scope.private_policy_revision).ok()
}
pub(crate) fn invocation_limit(planned: bool) -> u32 {
    CALLER
        .try_with(|c| c.scope.max_invocations)
        .unwrap_or(if planned { 6 } else { 2 })
        .min(if planned { 6 } else { 2 })
}

pub(crate) fn validate_submission(
    state: &State,
    request: &RunRequest,
    config: &Config,
) -> Result<()> {
    CALLER
        .try_with(|c| {
            c.scope.policy(state)?;
            ensure!(
                !request.plan || c.scope.allow_plan,
                "authorization_required: grant does not permit planned execution"
            );
            ensure!(
                c.scope.private_policy_revision
                    == crate::private_evidence::policy_revision(state, &c.scope.source)?,
                "authorization_required: private policy changed; issue a new grant for new work"
            );
            ensure!(
                request.source.canonicalize()? == c.scope.source
                    && request.config_path.is_none()
                    && request.backend.is_none()
                    && request.agent.is_none()
                    && request.harnesses.is_empty()
                    && !request.allow_forwarded_env
                    && request.priority == 0,
                "unauthorized submit scope"
            );
            ensure!(
                request
                    .timeout_secs
                    .is_some_and(|t| t > 0 && t <= c.scope.timeout_secs)
                    && config.execution.forwarded_env.is_empty()
                    && (!request.allow_unsafe_local || c.scope.allow_unsafe_local),
                "unauthorized execution budget"
            );
            Ok(())
        })
        .unwrap_or(Ok(()))
}

pub(crate) fn resources(state: &State) -> Result<ResourceConfig> {
    let mut resources = ResourceConfig::load(&state.root)?;
    if let Ok(scope) = CALLER.try_with(|c| c.scope.clone()) {
        scope.policy(state)?;
        ensure!(
            resources.allocation_enabled && resources.capacity.admission,
            "authorization_required: allocation or admission disabled"
        );
        resources
            .profiles
            .retain(|p| serde_json::to_value(p).is_ok_and(|v| scope.profiles.contains(&v)));
        ensure!(
            !resources.profiles.is_empty(),
            "authorization_required: no resources within grant and current policy"
        );
    }
    Ok(resources)
}

/// A local CLI answer/cancel must not take a live controller's checkpoint.
/// Hold the same grant lock through any explicitly human-owned continuation.
pub(crate) fn local_question_ownership(state: &State, id: &str) -> Result<Option<OperationLock>> {
    if actor().is_some() {
        return Ok(None);
    }
    let db = Database::open_control(state.db_path())?;
    let principal: Option<String> = db
        .connection()
        .query_row(
            "SELECT principal FROM control_runs WHERE run_id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    principal
        .map(|principal| {
            OperationLock::acquire(
                &state
                    .root
                    .join("control-grants")
                    .join(format!("{principal}.lock")),
                "run has a foreground control owner",
            )
        })
        .transpose()
}

pub(crate) fn authorize_question_target(state: &State, id: &str) -> Result<()> {
    CALLER
        .try_with(|c| {
            // Disconnect only reduces authority and must still clean up after expiry.
            if c.operation != "disconnect" {
                c.scope.run(state, id)?;
            }
            let db = Database::open_read_only(state.db_path())?;
            let owner: String = db.connection().query_row(
                "SELECT session_id FROM control_runs WHERE run_id=?1 AND principal=?2",
                params![id, c.scope.principal],
                |r| r.get(0),
            )?;
            ensure!(
                owner == c.session_id,
                "recovery_required: not the current session owner"
            );
            Ok(())
        })
        .unwrap_or(Ok(()))
}

pub(crate) fn authorize_answer(state: &State, run: &RunRecord, question_id: &str) -> Result<()> {
    CALLER
        .try_with(|c| {
            c.scope.run(state, &run.id)?;
            c.scope.policy(state)?;
            let db = Database::open_read_only(state.db_path())?;
            let (cancelled, owner): (bool, String) = db.connection().query_row(
                "SELECT cancelled,session_id FROM control_runs WHERE run_id=?1",
                [&run.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            ensure!(
                owner == c.session_id,
                "recovery_required: command is not from current session owner"
            );
            ensure!(!cancelled, "closed work cannot continue");
            let q = run
                .phase3
                .as_ref()
                .and_then(|p| p.questions.iter().find(|q| q.id == question_id))
                .context("wrong question")?;
            ensure!(
                c.scope.delegate_factual && q.report.category.as_deref() == Some("factual"),
                "authorization_required: question category is not delegated"
            );
            Ok(())
        })
        .unwrap_or(Ok(()))
}

fn matching(c: &CommandContext, event: &EventRecord) -> bool {
    matches!(
        (c.operation.as_str(), event.event_type.as_str()),
        ("submit", "run.created")
            | ("answer", "question.resolved")
            | ("recover", "run.recovered")
            | ("cancel", "cancel.requested")
    )
}
fn reply(c: &CommandContext, run: &RunRecord, event: &EventRecord) -> Value {
    json!({"type":"response","protocol_version":1,"request_id":c.request_id,"ok":true,"result":{"run_id":run.id,"state_revision":run.state_revision,"cursor":event.sequence,"accepted":true}})
}
pub(crate) fn commit_receipt(
    connection: &Connection,
    run: &RunRecord,
    event: &mut EventRecord,
) -> Result<()> {
    CALLER.try_with(|c| {
        if !matching(c,event) {return Ok(())}
        ensure!(Utc::now() < c.scope.expires_at, "grant_expired");
        if c.operation=="submit" {
            let busy: bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM control_runs c JOIN runs r ON r.id=c.run_id WHERE c.principal=?1 AND c.session_id=?2 AND json_extract(r.outcome_json,'$.lifecycle') != 'finished')",params![c.scope.principal,c.session_id],|r|r.get(0))?;
            ensure!(!busy,"busy: one active goal per session");
            connection.execute("INSERT INTO control_runs(run_id,principal,session_id) VALUES (?1,?2,?3)",params![run.id,c.scope.principal,c.session_id])?;
        }
        if c.operation=="cancel" {connection.execute("UPDATE control_runs SET cancelled=1 WHERE run_id=?1 AND principal=?2",params![run.id,c.scope.principal])?;}
        if c.operation=="recover" {connection.execute("UPDATE control_runs SET session_id=?2 WHERE run_id=?1 AND principal=?3",params![run.id,c.session_id,c.scope.principal])?;}
        connection.execute("INSERT INTO command_receipts(principal,request_id,payload_digest,run_id,reply_json) VALUES (?1,?2,?3,?4,?5)",params![c.scope.principal,c.request_id,c.payload_digest,run.id,serde_json::to_string(&reply(c,run,event))?])?;
        Ok(())
    }).unwrap_or(Ok(()))
}
pub(crate) fn notify_commit(run: &RunRecord, event: &EventRecord) {
    let _ = CALLER.try_with(|c| {
        if matching(c, event)
            && let Some(tx) = c.committed.lock().expect("receipt notification").take()
        {
            let _ = tx.send(reply(c, run, event));
        }
    });
}

/// Cancellation is a committed request, not a premature cleanup outcome. It
/// appends an event without invalidating the supervisor's current projection.
pub(crate) fn cancel(
    state: &State,
    scope: &Scope,
    run_id: &str,
    revision: u64,
) -> Result<RunRecord> {
    let run = scope.run(state, run_id)?;
    ensure!(
        run.state_revision == revision,
        "stale_revision: run changed"
    );
    ensure!(
        run.outcome.lifecycle != LifecycleState::Finished,
        "terminal: execution is finished"
    );
    let db = Database::open_control(state.db_path())?;
    let transaction = rusqlite::Transaction::new_unchecked(
        db.connection(),
        rusqlite::TransactionBehavior::Immediate,
    )?;
    let owner: String = transaction.query_row(
        "SELECT session_id FROM control_runs WHERE run_id=?1 AND principal=?2",
        params![run_id, scope.principal],
        |r| r.get(0),
    )?;
    ensure!(
        CALLER.try_with(|c| c.session_id == owner).unwrap_or(false),
        "unauthorized foreground owner"
    );
    let current = db
        .committed_run_projection(run_id)?
        .context("missing run")?;
    ensure!(
        current.state_revision == revision,
        "stale_revision: run changed"
    );
    let sequence: u64 = transaction.query_row(
        "SELECT COALESCE(MAX(sequence),0)+1 FROM events WHERE run_id=?1",
        [run_id],
        |r| r.get(0),
    )?;
    let mut event = EventRecord {
        run_id: run_id.into(),
        sequence,
        event_type: "cancel.requested".into(),
        actor: actor().context("machine caller required")?,
        timestamp: Utc::now(),
        payload: json!({"outcome":run.outcome,"state_revision":revision}),
        ..Default::default()
    };
    transaction.execute("INSERT INTO events(run_id,event_type,timestamp,payload_json,protocol_version,sequence,generation,actor) VALUES (?1,?2,?3,?4,1,?5,1,?6)",params![run_id,event.event_type,event.timestamp.to_rfc3339(),serde_json::to_string(&event.payload)?,sequence,event.actor])?;
    commit_receipt(&transaction, &run, &mut event)?;
    transaction.commit()?;
    notify_commit(&run, &event);
    Ok(run)
}

/// Finish an idle question after owner disconnect/cancel without reopening it.
pub(crate) fn close_question(state: &State, session: &Session, run: &RunRecord) -> Result<()> {
    if let Some(q) = run.phase3.as_ref().and_then(|p| {
        p.questions
            .iter()
            .find(|q| q.state == QuestionState::Pending)
    }) {
        let context = CommandContext {
            scope: session.scope.clone(),
            session_id: session.id.clone(),
            request_id: String::new(),
            payload_digest: String::new(),
            operation: "disconnect".into(),
            committed: Mutex::new(None),
        };
        CALLER.sync_scope(context, || {
            orchestrator::cancel_question(
                state,
                orchestrator::QuestionCommand {
                    run_id: run.id.clone(),
                    question_id: q.id.clone(),
                    revision: q.revision,
                    generation: q.generation,
                },
                orchestrator::RunOutputMode::Silent,
            )?;
            Ok::<_, anyhow::Error>(())
        })?;
    }
    Ok(())
}

pub(crate) fn ensure_machine_review_denied() -> Result<()> {
    ensure!(
        actor().is_none(),
        "authorization_required: human review is not delegated"
    );
    Ok(())
}

pub(crate) fn ensure_recoverable(
    state: &State,
    scope: &Scope,
    run: &RunRecord,
    revision: u64,
) -> Result<()> {
    scope.run(state, &run.id)?;
    scope.policy(state)?;
    ensure!(
        run.state_revision == revision,
        "stale_revision: run changed"
    );
    let policy = run.phase3.as_ref().context("no bounded policy")?;
    ensure!(
        policy.planning.is_none(),
        "planned crash recovery is unsupported; inspect retained evidence without replay"
    );
    ensure!(
        policy.max_invocations <= scope.max_invocations
            && policy.deadline_at
                <= run.created_at + chrono::TimeDelta::seconds(scope.timeout_secs as i64),
        "recovery_ineligible: persisted budget exceeds grant"
    );
    let db = Database::open_read_only(state.db_path())?;
    let cancelled: bool = db.connection().query_row(
        "SELECT cancelled FROM control_runs WHERE run_id=?1",
        [&run.id],
        |r| r.get(0),
    )?;
    ensure!(
        !cancelled
            && run.outcome.work_result != WorkResult::Cancelled
            && run.outcome.review == crate::ReviewState::NotRequested,
        "recovery_ineligible: closed work"
    );
    ensure!(
        run.attempts.len() == 1
            && run.attempts[0].completed_at.is_some()
            && run.attempts[0].detail.failure.is_none()
            && policy.final_attempt_id.is_none()
            && policy.questions.last().is_some_and(|q| matches!(
                q.state,
                QuestionState::Answered | QuestionState::Pending
            ))
            && run.attempts.len() < (policy.max_invocations as usize)
            && Utc::now() < policy.deadline_at,
        "recovery_ineligible: requires a completed checkpoint and committed answer with unused original budget"
    );
    let owner = policy
        .supervisor
        .as_ref()
        .context("uncertain previous owner")?;
    ensure!(
        matches!(
            crate::admission::identity_state(owner),
            crate::admission::IdentityState::Gone | crate::admission::IdentityState::Reused
        ),
        "owner_live_or_uncertain"
    );
    let unresolved:bool=db.connection().query_row("SELECT EXISTS(SELECT 1 FROM admission_requests WHERE run_id=?1 AND status IN ('queued','admitted','reconciliation'))",[&run.id],|r|r.get(0))?;
    ensure!(!unresolved, "recovery_ineligible: unresolved admission");
    ensure!(
        crate::source::fingerprint_tree(&run.source_path)? == run.source_fingerprint,
        "source_drift"
    );
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Initialize,
    Submit {
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        plan: bool,
        task: String,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        effort: Option<String>,
    },
    Status {
        run_id: String,
    },
    Result {
        run_id: String,
    },
    Capacity {
        run_id: String,
    },
    Events {
        run_id: String,
        after: u64,
    },
    Subscribe {
        run_id: String,
        after: u64,
        timeout_ms: u64,
    },
    Await {
        run_id: String,
        after: u64,
        predicate: Predicate,
        timeout_ms: u64,
    },
    Answer {
        run_id: String,
        question_id: String,
        revision: u64,
        generation: u32,
        answer: String,
    },
    Cancel {
        run_id: String,
        revision: u64,
    },
    Recover {
        run_id: String,
        revision: u64,
    },
    Artifact {
        run_id: String,
        attempt_id: String,
        kind: ArtifactKind,
        #[serde(default)]
        offset: u64,
    },
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Predicate {
    AttentionRequired,
    ExecutionFinished,
    WaitingReasonChanged,
    StateChanged,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    FinalDiff,
    Diff,
    Stdout,
    Stderr,
}
impl Operation {
    pub(crate) fn mutation(&self) -> bool {
        matches!(
            self,
            Self::Submit { .. } | Self::Answer { .. } | Self::Cancel { .. } | Self::Recover { .. }
        )
    }
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Submit { .. } => "submit",
            Self::Answer { .. } => "answer",
            Self::Cancel { .. } => "cancel",
            Self::Recover { .. } => "recover",
            _ => "read",
        }
    }
}

pub(crate) async fn execute(state: &State, scope: &Scope, command: Operation) -> Result<RunRecord> {
    match command {
        Operation::Submit {
            plan,
            task,
            model,
            effort,
        } => {
            orchestrator::run_dispatch(
                state,
                RunRequest {
                    plan,
                    max_invocations: None,
                    source: scope.source.clone(),
                    task,
                    harnesses: vec![],
                    agent: None,
                    model,
                    effort,
                    config_path: None,
                    backend: None,
                    timeout_secs: Some(scope.timeout_secs),
                    max_parallel: Some(1),
                    priority: 0,
                    no_retry: scope.max_invocations == 1,
                    allow_unsafe_local: scope.allow_unsafe_local,
                    allow_forwarded_env: false,
                    output: orchestrator::RunOutputMode::Silent,
                    refreshed_from: None,
                },
            )
            .await
        }
        Operation::Answer {
            run_id,
            question_id,
            revision,
            generation,
            answer,
        } => {
            orchestrator::answer_question(
                state,
                orchestrator::QuestionCommand {
                    run_id,
                    question_id,
                    revision,
                    generation,
                },
                answer,
                orchestrator::RunOutputMode::Silent,
            )
            .await
        }
        Operation::Recover { run_id, revision } => {
            orchestrator::phase3::recover_checkpoint(state, scope, &run_id, revision).await
        }
        _ => anyhow::bail!("not an execution command"),
    }
}
