use std::{path::PathBuf, process::Command, time::Duration};

use anyhow::{Context, Result};
use chrono::{DateTime, TimeDelta, Utc};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use ulid::Ulid;

use crate::{AdmissionState, AdmissionSummary, db::Database, executor::ExecutionObserver};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub start: Option<String>,
    pub boot: Option<String>,
    pub process_group: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityState {
    ExactLive,
    Gone,
    Reused,
    Unknown,
}

impl ProcessIdentity {
    pub fn current() -> Self {
        process_identity(std::process::id())
    }
}

pub fn process_identity(pid: u32) -> ProcessIdentity {
    ProcessIdentity {
        pid,
        start: process_start(pid),
        boot: boot_identity(),
        process_group: i32::try_from(pid).ok(),
    }
}

pub fn identity_state(expected: &ProcessIdentity) -> IdentityState {
    if !pid_exists(expected.pid) {
        return IdentityState::Gone;
    }
    let current = process_identity(expected.pid);
    match (
        &expected.start,
        &expected.boot,
        &current.start,
        &current.boot,
    ) {
        (Some(a), Some(b), Some(c), Some(d)) if a == c && b == d => IdentityState::ExactLive,
        (Some(_), Some(_), Some(_), Some(_)) => IdentityState::Reused,
        _ => IdentityState::Unknown,
    }
}

pub fn process_group_exists(group: Option<i32>) -> Option<bool> {
    #[cfg(unix)]
    {
        let group = group?;
        // SAFETY: signal zero only queries the process-group existence.
        let result = unsafe { libc::kill(-group, 0) };
        if result == 0 {
            Some(true)
        } else {
            let error = std::io::Error::last_os_error();
            match error.raw_os_error() {
                Some(libc::ESRCH) => Some(false),
                Some(libc::EPERM) => Some(true),
                _ => None,
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = group;
        None
    }
}

fn pid_exists(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let Ok(pid) = i32::try_from(pid) else {
            return false;
        };
        // SAFETY: signal zero does not modify the target process.
        unsafe {
            libc::kill(pid, 0) == 0
                || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

#[cfg(target_os = "linux")]
fn boot_identity() -> Option<String> {
    std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .ok()
        .map(|value| value.trim().to_owned())
}

#[cfg(target_os = "linux")]
fn process_start(pid: u32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let tail = stat.rsplit_once(')')?.1;
    tail.split_whitespace().nth(19).map(str::to_owned)
}

#[cfg(target_os = "macos")]
fn boot_identity() -> Option<String> {
    command_output("/usr/sbin/sysctl", &["-n", "kern.boottime"])
}

#[cfg(target_os = "macos")]
fn process_start(pid: u32) -> Option<String> {
    command_output("/bin/ps", &["-o", "lstart=", "-p", &pid.to_string()])
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn boot_identity() -> Option<String> {
    None
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn process_start(_pid: u32) -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseToken {
    pub request_id: String,
    pub pool_id: String,
    pub owner_session: String,
    pub generation: u64,
    pub fence: u64,
    pub attempt_id: String,
    pub authorization_id: String,
    pub route_snapshot_json: String,
    pub configuration_revision: String,
    pub canonical_pool_identity: String,
    pub authorization_revision: u64,
}

#[derive(Debug, Clone)]
pub struct AdmissionBinding {
    pub attempt_id: String,
    pub route_snapshot_json: String,
    pub configuration_revision: String,
    pub canonical_pool_identity: String,
    pub authorization_id: String,
    pub authorization_revision: u64,
}

pub fn canonical_pool_identity(
    provider: &str,
    funding_source: &str,
    provider_buckets: &[String],
) -> String {
    let mut buckets = provider_buckets.to_vec();
    buckets.sort();
    buckets.dedup();
    let material = serde_json::json!({
        "provider": provider,
        "funding_source": funding_source,
        "provider_buckets": buckets,
    });
    format!(
        "sha256:{}",
        hex::encode(Sha256::digest(material.to_string().as_bytes()))
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcquireResult {
    Acquired(Box<LeaseToken>),
    Waiting,
    Reconciliation,
}

#[derive(Debug, Clone)]
pub struct AdmissionCoordinator {
    db_path: PathBuf,
    lease_duration: Duration,
    aging: Duration,
}

/// Owns acquired permission until the execution future accepts responsibility.
/// Every early return/cancellation uses the same affirmative-no-spawn disposal.
pub(crate) struct AcquiredLeaseGuard {
    lease: Option<(AdmissionCoordinator, LeaseToken)>,
}

impl AcquiredLeaseGuard {
    pub(crate) fn new(coordinator: AdmissionCoordinator, token: LeaseToken) -> Self {
        Self {
            lease: Some((coordinator, token)),
        }
    }

    pub(crate) fn handoff(mut self) -> (AdmissionCoordinator, LeaseToken) {
        self.lease
            .take()
            .expect("acquired lease transfers exactly once")
    }
}

impl Drop for AcquiredLeaseGuard {
    fn drop(&mut self) {
        if let Some((coordinator, token)) = &self.lease {
            // A storage failure remains conservatively owned for reconciliation.
            let _ = coordinator.release_not_launched(token);
        }
    }
}

#[derive(Debug)]
pub struct AdmissionLeaseObserver {
    coordinator: AdmissionCoordinator,
    token: LeaseToken,
}

impl AdmissionLeaseObserver {
    pub fn new(coordinator: AdmissionCoordinator, token: LeaseToken) -> Self {
        Self { coordinator, token }
    }
}

impl ExecutionObserver for AdmissionLeaseObserver {
    fn provider_failure(&self, failure: crate::FailureKind) -> Result<()> {
        if failure == crate::FailureKind::Authorization {
            return self.preflight_failed();
        }
        if failure != crate::FailureKind::CapacityAdmission {
            return Ok(());
        }
        let db = crate::db::Database::open(&self.coordinator.db_path)?;
        let mut observation = crate::capacity::unknown_observation(
            &self.token.pool_id,
            "standard",
            Utc::now(),
            300,
            "provider rejected an invocation; a fresh authoritative availability signal is required",
        );
        observation.source = "harness_rejection_v1".into();
        observation.scarcity = crate::ScarcityState::Exhausted;
        observation.attributable_attempt_id = Some(self.token.attempt_id.clone());
        let route: serde_json::Value = serde_json::from_str(&self.token.route_snapshot_json)?;
        if let Some(buckets) = route["provider_buckets"]
            .as_array()
            .filter(|b| !b.is_empty())
        {
            let template = observation.constraints[0].clone();
            observation.constraints = buckets
                .iter()
                .filter_map(|b| b.as_str())
                .map(|bucket| {
                    let mut constraint = template.clone();
                    constraint.kind = "provider_rejection".into();
                    constraint.provider_bucket_id = Some(bucket.into());
                    constraint.scope = crate::CapacityValue::Reported {
                        value: "configured shared funding pool".into(),
                    };
                    constraint
                })
                .collect();
            observation.mapping = crate::CapacityMapping::Mapped;
        }
        db.append_capacity_observation(&observation)
    }

    fn preflight_failed(&self) -> Result<()> {
        let db = crate::db::Database::open(&self.coordinator.db_path)?;
        db.connection().execute(
            "INSERT INTO capacity_authorizations(id,pool_id,authorization_revision,observation_id,route_revision,evidence_json,status,reason,created_at,valid_until) SELECT ?1,pool_id,authorization_revision,observation_id,route_revision,evidence_json,'rejected','adapter preflight rejected the bound invocation; revalidate funding and configuration',?2,valid_until FROM capacity_authorizations WHERE id=?3",
            rusqlite::params![ulid::Ulid::new().to_string(), Utc::now().to_rfc3339(), self.token.authorization_id],
        )?;
        Ok(())
    }

    fn authorize_launch(&self) -> Result<()> {
        self.coordinator.authorize_launch(&self.token)
    }

    fn spawn_failed(&self) -> Result<()> {
        self.coordinator.spawn_failed(&self.token)
    }

    fn child_spawned(
        &self,
        identity: &ProcessIdentity,
        backend_identity: Option<&str>,
    ) -> Result<()> {
        self.coordinator
            .child_spawned(&self.token, identity, backend_identity)
    }

    fn cleanup_confirmed(&self, identity: &ProcessIdentity) -> Result<()> {
        self.coordinator.cleanup_confirmed(&self.token, identity)
    }

    fn cleanup_after_unrecorded_spawn(
        &self,
        identity: &ProcessIdentity,
        backend_identity: Option<&str>,
        confirmed: bool,
    ) -> Result<()> {
        self.coordinator.cleanup_after_unrecorded_spawn(
            &self.token,
            identity,
            backend_identity,
            confirmed,
        )
    }
}

impl AdmissionCoordinator {
    pub fn new(db_path: impl Into<PathBuf>, lease_duration: Duration, aging: Duration) -> Self {
        Self {
            db_path: db_path.into(),
            lease_duration,
            aging,
        }
    }

    pub fn register_pool(
        &self,
        pool_id: &str,
        provider: &str,
        funding_source: &str,
        provider_buckets: &[String],
    ) -> Result<String> {
        let mut db = Database::open_control(&self.db_path)?;
        let now = Utc::now().to_rfc3339();
        let mut normalized = provider_buckets.to_vec();
        normalized.sort();
        normalized.dedup();
        let buckets = serde_json::to_string(&normalized)?;
        let canonical = canonical_pool_identity(provider, funding_source, &normalized);
        let transaction = db
            .connection_mut()
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = transaction
            .query_row(
                "SELECT id,provider,funding_source,provider_buckets_json FROM resource_pools WHERE canonical_identity=?1 OR (provider=?2 AND funding_source=?3 AND provider_buckets_json=?4) ORDER BY CASE WHEN canonical_identity=?1 THEN 0 ELSE 1 END LIMIT 1",
                params![canonical, provider, funding_source, buckets],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?)),
            )
            .optional()?;
        let id = if let Some((id, old_provider, old_funding, old_buckets)) = existing {
            anyhow::ensure!(
                (old_provider, old_funding, old_buckets)
                    == (
                        provider.to_owned(),
                        funding_source.to_owned(),
                        buckets.clone()
                    ),
                "canonical resource pool identity conflicts with registered allowance facts"
            );
            transaction.execute(
                "UPDATE resource_pools SET canonical_identity=?2,updated_at=?3 WHERE id=?1",
                params![id, canonical, now],
            )?;
            id
        } else {
            // A confirmed subscription change can reuse its local pool name.
            // Preserve its evidence and monotonic fence, and never detach any
            // queued, live, or uncertain owner from the old overlapping allowance.
            let previous: Option<String> = transaction
                .query_row(
                    "SELECT provider FROM resource_pools WHERE id=?1",
                    [pool_id],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(previous) = previous {
                anyhow::ensure!(
                    previous == provider,
                    "a resource pool cannot change provider"
                );
                let overlaps =
                    serde_json::to_string(&crate::db::overlapping_pools(&transaction, pool_id)?)?;
                let busy: bool = transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM pool_leases WHERE pool_id IN (SELECT value FROM json_each(?1))) OR EXISTS(SELECT 1 FROM admission_requests WHERE pool_id IN (SELECT value FROM json_each(?1)) AND status IN ('queued','admitted','reconciliation'))",
                    [&overlaps], |row| row.get(0),
                )?;
                anyhow::ensure!(
                    !busy,
                    "cannot change resource pool {pool_id}: its previous allowance still has queued or owned work; finish or reconcile that work first"
                );
                transaction.execute(
                    "UPDATE resource_pools SET funding_source=?2,provider_buckets_json=?3,canonical_identity=?4,updated_at=?5 WHERE id=?1",
                    params![pool_id, funding_source, buckets, canonical, now],
                )?;
            } else {
                transaction.execute(
                "INSERT INTO resource_pools(id,provider,funding_source,provider_buckets_json,max_active,next_fence,created_at,updated_at,canonical_identity) VALUES(?1,?2,?3,?4,1,0,?5,?5,?6)",
                params![pool_id, provider, funding_source, buckets, now, canonical],
            )?;
            }
            pool_id.to_owned()
        };
        transaction.commit()?;
        Ok(id)
    }

    pub fn enqueue_bound(
        &self,
        run_id: &str,
        pool_id: &str,
        owner_session: &str,
        generation: u64,
        priority: i32,
        binding: &AdmissionBinding,
    ) -> Result<AdmissionSummary> {
        let db = Database::open_control(&self.db_path)?;
        let id = Ulid::new().to_string();
        let now = Utc::now();
        let expires = now + duration_delta(self.lease_duration);
        let owner = ProcessIdentity::current();
        let attempt_id = (!binding.attempt_id.is_empty()).then_some(&binding.attempt_id);
        let authorization_id =
            (!binding.authorization_id.is_empty()).then_some(&binding.authorization_id);
        db.connection().execute(
            "INSERT INTO admission_requests(id,run_id,pool_id,owner_session,generation,priority,enqueued_at,heartbeat_at,expires_at,owner_pid,owner_start_identity,owner_boot_identity,status,attempt_id,route_snapshot_json,configuration_revision,authorization_revision,authorization_id,canonical_pool_identity) VALUES(?1,?2,?3,?4,?5,?6,?7,?7,?8,?9,?10,?11,'queued',?12,?13,?14,?15,?16,?17)",
            params![id, run_id, pool_id, owner_session, integer(generation)?, priority, stamp(now), stamp(expires), owner.pid, owner.start, owner.boot, attempt_id, binding.route_snapshot_json, binding.configuration_revision, integer(binding.authorization_revision)?, authorization_id, binding.canonical_pool_identity],
        )?;
        Ok(AdmissionSummary {
            request_id: id,
            pool_id: pool_id.to_owned(),
            attempt_id: binding.attempt_id.clone(),
            route_snapshot_json: binding.route_snapshot_json.clone(),
            configuration_revision: binding.configuration_revision.clone(),
            canonical_pool_identity: binding.canonical_pool_identity.clone(),
            authorization_id: binding.authorization_id.clone(),
            authorization_revision: binding.authorization_revision,
            owner_session: owner_session.to_owned(),
            generation,
            priority,
            state: AdmissionState::Queued,
            fence: None,
            enqueued_at: now,
            released_at: None,
        })
    }

    #[cfg(test)]
    pub fn enqueue(
        &self,
        run_id: &str,
        pool_id: &str,
        owner_session: &str,
        generation: u64,
        priority: i32,
    ) -> Result<AdmissionSummary> {
        self.enqueue_bound(
            run_id,
            pool_id,
            owner_session,
            generation,
            priority,
            &AdmissionBinding {
                attempt_id: String::new(),
                route_snapshot_json: "{}".into(),
                configuration_revision: "test".into(),
                canonical_pool_identity: "test".into(),
                authorization_id: String::new(),
                authorization_revision: 1,
            },
        )
    }

    pub fn try_acquire(&self, request: &AdmissionSummary) -> Result<AcquireResult> {
        anyhow::ensure!(
            self.heartbeat_request(request)?,
            "admission request ownership or generation was lost"
        );
        let db = Database::open_control(&self.db_path)?;
        for pool in crate::db::overlapping_pools(db.connection(), &request.pool_id)? {
            self.reconcile_expired_requests(&pool)?;
            self.reconcile_expired(&pool)?;
        }
        let mut db = Database::open_control(&self.db_path)?;
        let now = Utc::now();
        let expires = now + duration_delta(self.lease_duration);
        // Process discovery may invoke ps/sysctl on macOS. Resolve it before
        // entering SQLite's single-writer critical section.
        let owner = ProcessIdentity::current();
        let transaction = db
            .connection_mut()
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Recompute inside the write fence: another configuration may have
        // registered an overlapping pool since reconciliation inspected it.
        let pools = serde_json::to_string(&crate::db::overlapping_pools(
            &transaction,
            &request.pool_id,
        )?)?;
        let lease_state = transaction
            .query_row(
                "SELECT state FROM pool_leases WHERE pool_id IN (SELECT value FROM json_each(?1)) ORDER BY state='reconciliation' DESC LIMIT 1",
                [&pools],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if let Some(state) = lease_state {
            transaction.commit()?;
            return Ok(if state == "reconciliation" {
                AcquireResult::Reconciliation
            } else {
                AcquireResult::Waiting
            });
        }
        let aging_seconds = self.aging.as_secs().max(1);
        let head = transaction
            .query_row(
                "SELECT id FROM admission_requests WHERE pool_id IN (SELECT value FROM json_each(?1)) AND status='queued' ORDER BY MIN(1, priority + CAST(MAX(0,(julianday(?2)-julianday(enqueued_at))*86400)/?3 AS INTEGER)) DESC, enqueued_at, id LIMIT 1",
                params![pools, stamp(now), integer(aging_seconds)?],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if head.as_deref() != Some(&request.request_id) {
            transaction.commit()?;
            return Ok(AcquireResult::Waiting);
        }
        let changed = transaction.execute(
            "UPDATE admission_requests SET status='admitted',start_intent_at=?2,admitted_at=?2,heartbeat_at=?2,expires_at=?3 WHERE id=?1 AND owner_session=?4 AND generation=?5 AND status='queued'",
            params![request.request_id, stamp(now), stamp(expires), request.owner_session, integer(request.generation)?],
        )?;
        anyhow::ensure!(
            changed == 1,
            "admission request was superseded before lease acquisition"
        );
        let changed = transaction.execute(
            "UPDATE resource_pools SET next_fence=next_fence+1,updated_at=?2 WHERE id=?1",
            params![request.pool_id, stamp(now)],
        )?;
        anyhow::ensure!(changed == 1, "resource pool disappeared during acquisition");
        let fence: i64 = transaction.query_row(
            "SELECT next_fence FROM resource_pools WHERE id=?1",
            [&request.pool_id],
            |row| row.get(0),
        )?;
        let changed = transaction.execute(
            "UPDATE admission_requests SET fence=?2 WHERE id=?1 AND owner_session=?3 AND generation=?4 AND status='admitted'",
            params![request.request_id, fence, request.owner_session, integer(request.generation)?],
        )?;
        anyhow::ensure!(
            changed == 1,
            "admission request lost ownership before fencing"
        );
        let inserted = transaction.execute(
            "INSERT INTO pool_leases(pool_id,request_id,owner_session,generation,fence,state,start_intent_at,heartbeat_at,expires_at,owner_pid,owner_start_identity,owner_boot_identity,attempt_id,route_snapshot_json,configuration_revision,authorization_revision,authorization_id,canonical_pool_identity,launch_lifecycle) SELECT pool_id,id,owner_session,generation,?2,'start_intent',?3,?3,?4,?5,?6,?7,attempt_id,route_snapshot_json,configuration_revision,authorization_revision,authorization_id,canonical_pool_identity,'launch_intent_committed' FROM admission_requests WHERE id=?1 AND owner_session=?8 AND generation=?9 AND status='admitted'",
            params![request.request_id, fence, stamp(now), stamp(expires), owner.pid, owner.start, owner.boot, request.owner_session, integer(request.generation)?],
        )?;
        anyhow::ensure!(
            inserted == 1,
            "admission binding changed during lease acquisition"
        );
        transaction.commit()?;
        Ok(AcquireResult::Acquired(Box::new(LeaseToken {
            request_id: request.request_id.clone(),
            pool_id: request.pool_id.clone(),
            owner_session: request.owner_session.clone(),
            generation: request.generation,
            fence: u64::try_from(fence).context("invalid lease fence")?,
            attempt_id: request.attempt_id.clone(),
            authorization_id: request.authorization_id.clone(),
            route_snapshot_json: request.route_snapshot_json.clone(),
            configuration_revision: request.configuration_revision.clone(),
            canonical_pool_identity: request.canonical_pool_identity.clone(),
            authorization_revision: request.authorization_revision,
        })))
    }

    pub fn heartbeat(&self, token: &LeaseToken) -> Result<bool> {
        let mut db = Database::open_control(&self.db_path)?;
        let now = Utc::now();
        let expires = now + duration_delta(self.lease_duration);
        let transaction = db
            .connection_mut()
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE pool_leases SET heartbeat_at=?6,expires_at=?7 WHERE pool_id=?1 AND request_id=?2 AND owner_session=?3 AND generation=?4 AND fence=?5",
            params![token.pool_id, token.request_id, token.owner_session, integer(token.generation)?, integer(token.fence)?, stamp(now), stamp(expires)],
        )?;
        if changed == 1 {
            let request_changed = transaction.execute(
                "UPDATE admission_requests SET heartbeat_at=?4,expires_at=?5 WHERE id=?1 AND owner_session=?2 AND generation=?3 AND status='admitted'",
                params![token.request_id, token.owner_session, integer(token.generation)?, stamp(now), stamp(expires)],
            )?;
            anyhow::ensure!(
                request_changed == 1,
                "lease heartbeat lost its request ownership"
            );
        }
        transaction.commit()?;
        Ok(changed == 1)
    }

    /// The final, one-shot spending boundary. It rechecks the newest shared
    /// evidence and atomically changes explicit launch knowledge before the OS
    /// spawn call is attempted.
    pub fn authorize_launch(&self, token: &LeaseToken) -> Result<()> {
        let route: serde_json::Value = serde_json::from_str(&token.route_snapshot_json)?;
        if let Some(path) = route["resources_path"].as_str() {
            let resources = crate::config::ResourceConfig::load(
                std::path::Path::new(path)
                    .parent()
                    .context("invalid resource config path")?,
            )?;
            anyhow::ensure!(
                resources.allocation_enabled
                    && resources.capacity.admission
                    && resources.profiles.iter().any(|p| p.enabled
                        && serde_json::to_value(p).ok().as_ref()
                            == Some(&route["resource_profile"])),
                "resource configuration changed after admission; stale launch refused"
            );
        }
        let configuration_revision = format!(
            "sha256:{}",
            hex::encode(Sha256::digest(token.route_snapshot_json.as_bytes()))
        );
        anyhow::ensure!(
            configuration_revision == token.configuration_revision,
            "immutable launch route does not match its configuration revision"
        );
        let owner = ProcessIdentity::current();
        let mut db = Database::open_control(&self.db_path)?;
        let transaction = db
            .connection_mut()
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = transaction
            .query_row(
                "SELECT a.status,a.evidence_json,a.authorization_revision,p.funding_source,o.payload_json,l.launch_lifecycle,a.route_revision,l.configuration_revision FROM pool_leases l JOIN admission_requests r ON r.id=l.request_id AND r.owner_session=l.owner_session AND r.generation=l.generation AND r.attempt_id=l.attempt_id AND r.route_snapshot_json=l.route_snapshot_json AND r.configuration_revision=l.configuration_revision AND r.authorization_revision=l.authorization_revision AND r.authorization_id=l.authorization_id AND r.canonical_pool_identity=l.canonical_pool_identity AND r.status='admitted' JOIN attempts t ON t.id=l.attempt_id AND t.run_id=r.run_id JOIN capacity_authorizations a ON a.id=l.authorization_id AND a.authorization_revision=l.authorization_revision AND a.pool_id=l.pool_id AND a.route_revision=l.configuration_revision JOIN resource_pools p ON p.id=l.pool_id AND p.canonical_identity=l.canonical_pool_identity JOIN capacity_observations o ON o.pool_id=l.pool_id WHERE l.pool_id=?1 AND l.request_id=?2 AND l.owner_session=?3 AND l.generation=?4 AND l.fence=?5 AND l.attempt_id=?6 AND l.owner_pid=?7 AND l.owner_start_identity IS ?8 AND l.owner_boot_identity IS ?9 AND l.state='start_intent' AND l.expires_at>?10 AND l.authorization_id=?11 AND l.authorization_revision=?12 AND l.route_snapshot_json=?13 AND l.configuration_revision=?14 AND l.canonical_pool_identity=?15 ORDER BY o.sampled_at DESC,o.id DESC LIMIT 1",
                params![token.pool_id, token.request_id, token.owner_session, integer(token.generation)?, integer(token.fence)?, token.attempt_id, owner.pid, owner.start, owner.boot, stamp(Utc::now()), token.authorization_id, integer(token.authorization_revision)?, token.route_snapshot_json, token.configuration_revision, token.canonical_pool_identity],
                |row| Ok((
                    row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?, row.get::<_, String>(7)?,
                )),
            )
            .optional()?;
        let Some((
            status,
            authorized_json,
            authorization_revision,
            funding_source,
            newest_json,
            lifecycle,
            route_revision,
            configuration_revision,
        )) = row
        else {
            transaction.commit()?;
            anyhow::bail!("lease ownership or immutable launch binding was lost before spawn");
        };
        if status != "authorized" || lifecycle != "launch_intent_committed" {
            transaction.commit()?;
            anyhow::bail!("lease is not authorized for a first launch");
        }
        anyhow::ensure!(
            route_revision == configuration_revision,
            "authorization does not match the immutable route configuration"
        );
        let bound_authorized: crate::CapacityObservation = serde_json::from_str(&authorized_json)?;
        let newest: crate::CapacityObservation = serde_json::from_str(&newest_json)?;
        let identity_conflict = crate::capacity::retained_authorization_conflict(
            &transaction,
            &token.pool_id,
            authorization_revision,
            &newest,
            &funding_source,
        )?;
        if let Some(reason) = identity_conflict {
            transaction.execute(
                "INSERT INTO capacity_authorizations(id,pool_id,authorization_revision,observation_id,route_revision,evidence_json,status,reason,created_at,valid_until) VALUES(?1,?2,?3,?4,?5,?6,'rejected',?7,?8,?9)",
                params![Ulid::new().to_string(), token.pool_id, authorization_revision, newest.id, route_revision, newest_json, reason, stamp(Utc::now()), newest.valid_until.to_rfc3339()],
            )?;
            transaction.commit()?;
            anyhow::bail!("subscription-only funding revalidation failed before launch: {reason}");
        }
        let mut launch_evidence = newest.clone();
        launch_evidence.scarcity =
            crate::capacity::resolved_scarcity(&transaction, &token.pool_id, Utc::now())?;
        if let Some(reason) = crate::capacity::launch_block(
            &bound_authorized,
            &launch_evidence,
            &funding_source,
            self.request_priority(&transaction, token)?,
        ) {
            transaction.commit()?;
            anyhow::bail!("launch deferred: {reason}");
        }
        crate::planning::fence(&transaction, &token.attempt_id)?;
        let changed = transaction.execute(
            "UPDATE pool_leases SET launch_lifecycle='spawn_may_have_occurred',launch_observation_id=?6 WHERE pool_id=?1 AND request_id=?2 AND owner_session=?3 AND generation=?4 AND fence=?5 AND launch_lifecycle='launch_intent_committed'",
            params![token.pool_id, token.request_id, token.owner_session, integer(token.generation)?, integer(token.fence)?, newest.id],
        )?;
        anyhow::ensure!(changed == 1, "lease was superseded at the launch boundary");
        transaction.commit()?;
        Ok(())
    }

    fn request_priority(
        &self,
        transaction: &rusqlite::Transaction<'_>,
        token: &LeaseToken,
    ) -> Result<i32> {
        Ok(transaction.query_row(
            "SELECT priority FROM admission_requests WHERE id=?1 AND owner_session=?2 AND generation=?3",
            params![token.request_id, token.owner_session, integer(token.generation)?],
            |row| row.get(0),
        )?)
    }

    pub fn spawn_failed(&self, token: &LeaseToken) -> Result<()> {
        let db = Database::open_control(&self.db_path)?;
        let changed = db.connection().execute(
            "UPDATE pool_leases SET launch_lifecycle='launch_not_started' WHERE pool_id=?1 AND request_id=?2 AND owner_session=?3 AND generation=?4 AND fence=?5 AND launch_lifecycle='spawn_may_have_occurred'",
            params![token.pool_id, token.request_id, token.owner_session, integer(token.generation)?, integer(token.fence)?],
        )?;
        anyhow::ensure!(
            changed == 1,
            "spawn-failure knowledge lost its fenced lease"
        );
        Ok(())
    }

    pub fn child_spawned(
        &self,
        token: &LeaseToken,
        child: &ProcessIdentity,
        backend_identity: Option<&str>,
    ) -> Result<()> {
        let db = Database::open_control(&self.db_path)?;
        let changed = db.connection().execute(
            "UPDATE pool_leases SET state='running',launch_lifecycle='child_recorded',child_pid=?6,child_start_identity=?7,child_boot_identity=?8,child_process_group=?9,backend_identity=?10 WHERE pool_id=?1 AND request_id=?2 AND owner_session=?3 AND generation=?4 AND fence=?5 AND launch_lifecycle='spawn_may_have_occurred'",
            params![token.pool_id, token.request_id, token.owner_session, integer(token.generation)?, integer(token.fence)?, child.pid, child.start, child.boot, child.process_group, backend_identity],
        )?;
        anyhow::ensure!(
            changed == 1,
            "lease was superseded before child start was recorded"
        );
        Ok(())
    }

    pub fn cleanup_confirmed(&self, token: &LeaseToken, child: &ProcessIdentity) -> Result<()> {
        let db = Database::open_control(&self.db_path)?;
        let changed = db.connection().execute(
            "UPDATE pool_leases SET launch_lifecycle='cleanup_confirmed',cleanup_confirmed_at=?10 WHERE pool_id=?1 AND request_id=?2 AND owner_session=?3 AND generation=?4 AND fence=?5 AND child_pid=?6 AND child_start_identity IS ?7 AND child_boot_identity IS ?8 AND child_process_group IS ?9 AND launch_lifecycle='child_recorded'",
            params![token.pool_id, token.request_id, token.owner_session, integer(token.generation)?, integer(token.fence)?, child.pid, child.start, child.boot, child.process_group, stamp(Utc::now())],
        )?;
        anyhow::ensure!(
            changed == 1,
            "lease was superseded before child cleanup was recorded"
        );
        Ok(())
    }

    pub fn cleanup_after_unrecorded_spawn(
        &self,
        token: &LeaseToken,
        child: &ProcessIdentity,
        backend_identity: Option<&str>,
        confirmed: bool,
    ) -> Result<()> {
        let mut db = Database::open_control(&self.db_path)?;
        let transaction = db
            .connection_mut()
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let lifecycle = if confirmed {
            "cleanup_confirmed"
        } else {
            "cleanup_uncertain"
        };
        let changed = transaction.execute(
            "UPDATE pool_leases SET state=CASE WHEN ?10 THEN state ELSE 'reconciliation' END,launch_lifecycle=?6,child_pid=?7,child_start_identity=?8,child_boot_identity=?9,child_process_group=?11,backend_identity=?12,cleanup_confirmed_at=CASE WHEN ?10 THEN ?13 ELSE NULL END WHERE pool_id=?1 AND request_id=?2 AND owner_session=?3 AND generation=?4 AND fence=?5 AND launch_lifecycle IN ('spawn_may_have_occurred','child_recorded')",
            params![token.pool_id, token.request_id, token.owner_session, integer(token.generation)?, integer(token.fence)?, lifecycle, child.pid, child.start, child.boot, confirmed, child.process_group, backend_identity, stamp(Utc::now())],
        )?;
        anyhow::ensure!(changed == 1, "cleanup knowledge lost its fenced lease");
        if !confirmed {
            let changed = transaction.execute(
                "UPDATE admission_requests SET status='reconciliation' WHERE id=?1 AND pool_id=?2 AND owner_session=?3 AND generation=?4 AND fence=?5 AND status IN ('admitted','reconciliation')",
                params![token.request_id,token.pool_id,token.owner_session,integer(token.generation)?,integer(token.fence)?],
            )?;
            anyhow::ensure!(
                changed == 1,
                "uncertain cleanup lost its admission ownership"
            );
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn release(&self, token: &LeaseToken) -> Result<bool> {
        let mut db = Database::open_control(&self.db_path)?;
        let transaction = db
            .connection_mut()
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = Utc::now();
        let deleted = transaction.execute(
            "DELETE FROM pool_leases WHERE pool_id=?1 AND request_id=?2 AND owner_session=?3 AND generation=?4 AND fence=?5 AND launch_lifecycle IN ('launch_intent_committed','launch_not_started','cleanup_confirmed')",
            params![token.pool_id, token.request_id, token.owner_session, integer(token.generation)?, integer(token.fence)?],
        )?;
        if deleted == 1 {
            let changed = transaction.execute(
                "UPDATE admission_requests SET status='released',released_at=?2 WHERE id=?1 AND owner_session=?3 AND generation=?4 AND status IN ('admitted','reconciliation')",
                params![token.request_id, stamp(now), token.owner_session, integer(token.generation)?],
            )?;
            anyhow::ensure!(changed == 1, "released lease lost its request ownership");
        }
        transaction.commit()?;
        Ok(deleted == 1)
    }

    pub fn release_not_launched(&self, token: &LeaseToken) -> Result<bool> {
        let db = Database::open_control(&self.db_path)?;
        let lifecycle: Option<String> = db
            .connection()
            .query_row(
                "SELECT launch_lifecycle FROM pool_leases WHERE pool_id=?1 AND request_id=?2 AND owner_session=?3 AND generation=?4 AND fence=?5",
                params![token.pool_id, token.request_id, token.owner_session, integer(token.generation)?, integer(token.fence)?],
                |row| row.get(0),
            )
            .optional()?;
        if matches!(
            lifecycle.as_deref(),
            Some("launch_intent_committed" | "launch_not_started")
        ) {
            self.release(token)
        } else {
            Ok(false)
        }
    }

    pub fn cancel_queued(&self, request: &AdmissionSummary) -> Result<()> {
        let db = Database::open_control(&self.db_path)?;
        let changed = db.connection().execute(
            "UPDATE admission_requests SET status='cancelled',released_at=?4 WHERE id=?1 AND owner_session=?2 AND generation=?3 AND status='queued'",
            params![request.request_id, request.owner_session, integer(request.generation)?, stamp(Utc::now())],
        )?;
        anyhow::ensure!(
            changed == 1,
            "queued admission ownership was lost before cancellation"
        );
        Ok(())
    }

    fn heartbeat_request(&self, request: &AdmissionSummary) -> Result<bool> {
        let db = Database::open_control(&self.db_path)?;
        let now = Utc::now();
        let expires = now + duration_delta(self.lease_duration);
        let changed = db.connection().execute(
            "UPDATE admission_requests SET heartbeat_at=?4,expires_at=?5 WHERE id=?1 AND owner_session=?2 AND generation=?3 AND status='queued'",
            params![request.request_id, request.owner_session, integer(request.generation)?, stamp(now), stamp(expires)],
        )?;
        Ok(changed == 1)
    }

    fn reconcile_expired_requests(&self, pool_id: &str) -> Result<()> {
        let db = Database::open_control(&self.db_path)?;
        let mut statement = db.connection().prepare(
            "SELECT id,owner_session,generation,expires_at,owner_pid,owner_start_identity,owner_boot_identity FROM admission_requests WHERE pool_id=?1 AND status='queued' AND expires_at<=?2 ORDER BY enqueued_at,id",
        )?;
        let expired = statement
            .query_map(params![pool_id, stamp(Utc::now())], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    identity_from_row(row.get(4)?, row.get(5)?, row.get(6)?, None),
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        for (id, owner_session, generation, expires_at, owner) in expired {
            let state = owner
                .as_ref()
                .map(identity_state)
                .unwrap_or(IdentityState::Unknown);
            if matches!(state, IdentityState::Gone | IdentityState::Reused) {
                db.connection().execute(
                    "UPDATE admission_requests SET status='abandoned',released_at=?5 WHERE id=?1 AND owner_session=?2 AND generation=?3 AND status='queued' AND expires_at=?4",
                    params![id, owner_session, generation, expires_at, stamp(Utc::now())],
                )?;
            }
        }
        Ok(())
    }

    pub(crate) fn reconcile_expired(&self, pool_id: &str) -> Result<()> {
        let db = Database::open_control(&self.db_path)?;
        let lease = db.connection().query_row(
            "SELECT request_id,owner_session,generation,fence,expires_at,owner_pid,owner_start_identity,owner_boot_identity,child_pid,child_start_identity,child_boot_identity,child_process_group,backend_identity,cleanup_confirmed_at,launch_lifecycle FROM pool_leases WHERE pool_id=?1",
            [pool_id],
            |row| Ok(LeaseSnapshot {
                request_id: row.get(0)?, owner_session: row.get(1)?, generation: row.get(2)?, fence: row.get(3)?, expires_at: row.get(4)?,
                owner: identity_from_row(row.get(5)?, row.get(6)?, row.get(7)?, None),
                child: identity_from_row(row.get(8)?, row.get(9)?, row.get(10)?, row.get(11)?),
                backend_identity: row.get(12)?,
                cleanup_confirmed_at: row.get(13)?,
                launch_lifecycle: row.get(14)?,
            }),
        ).optional()?;
        let Some(lease) = lease else {
            return Ok(());
        };
        let expires = DateTime::parse_from_rfc3339(&lease.expires_at)?.with_timezone(&Utc);
        if expires > Utc::now() {
            return Ok(());
        }
        let owner_state = lease
            .owner
            .as_ref()
            .map(identity_state)
            .unwrap_or(IdentityState::Unknown);
        let child_state = lease.child.as_ref().map(identity_state);
        let group = lease
            .child
            .as_ref()
            .and_then(|child| process_group_exists(child.process_group));
        let definitely_not_launched = matches!(
            lease.launch_lifecycle.as_str(),
            "launch_intent_committed" | "launch_not_started"
        );
        let child_stopped = lease.cleanup_confirmed_at.is_some()
            || (lease.backend_identity.is_none()
                && lease.child.is_some()
                && matches!(
                    child_state,
                    Some(IdentityState::Gone | IdentityState::Reused)
                )
                && group == Some(false));
        let safe = matches!(owner_state, IdentityState::Gone | IdentityState::Reused)
            && (definitely_not_launched || child_stopped);
        let db = Database::open_control(&self.db_path)?;
        if safe {
            let mut db = db;
            let transaction = db
                .connection_mut()
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let deleted = transaction.execute(
                "DELETE FROM pool_leases WHERE pool_id=?1 AND request_id=?2 AND owner_session=?3 AND generation=?4 AND fence=?5 AND expires_at=?6",
                params![pool_id, lease.request_id, lease.owner_session, lease.generation, lease.fence, lease.expires_at],
            )?;
            if deleted == 1 {
                let changed = transaction.execute(
                    "UPDATE admission_requests SET status='abandoned',released_at=?5 WHERE id=?1 AND owner_session=?2 AND generation=?3 AND status IN ('admitted','reconciliation') AND pool_id=?4",
                    params![lease.request_id, lease.owner_session, lease.generation, pool_id, stamp(Utc::now())],
                )?;
                anyhow::ensure!(changed == 1, "reclaimed lease lost its request ownership");
            }
            transaction.commit()?;
        } else {
            let mut db = db;
            let transaction = db
                .connection_mut()
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let changed = transaction.execute(
                "UPDATE pool_leases SET state='reconciliation' WHERE pool_id=?1 AND request_id=?2 AND generation=?3 AND fence=?4 AND expires_at=?5",
                params![pool_id, lease.request_id, lease.generation, lease.fence, lease.expires_at],
            )?;
            if changed == 1 {
                let request_changed = transaction.execute(
                    "UPDATE admission_requests SET status='reconciliation' WHERE id=?1 AND owner_session=?2 AND generation=?3 AND pool_id=?4 AND status IN ('admitted','reconciliation')",
                    params![lease.request_id, lease.owner_session, lease.generation, pool_id],
                )?;
                anyhow::ensure!(
                    request_changed == 1,
                    "lease/request reconciliation state diverged"
                );
            }
            transaction.commit()?;
        }
        Ok(())
    }
}

struct LeaseSnapshot {
    request_id: String,
    owner_session: String,
    generation: i64,
    fence: i64,
    expires_at: String,
    owner: Option<ProcessIdentity>,
    child: Option<ProcessIdentity>,
    backend_identity: Option<String>,
    cleanup_confirmed_at: Option<String>,
    launch_lifecycle: String,
}

pub(crate) fn identity_from_row(
    pid: Option<u32>,
    start: Option<String>,
    boot: Option<String>,
    group: Option<i32>,
) -> Option<ProcessIdentity> {
    pid.map(|pid| ProcessIdentity {
        pid,
        start,
        boot,
        process_group: group,
    })
}

fn duration_delta(duration: Duration) -> TimeDelta {
    TimeDelta::seconds(i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
}

fn integer(value: u64) -> Result<i64> {
    i64::try_from(value).context("integer exceeds SQLite range")
}

fn stamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capacity::{authorize_observation, unknown_observation};
    use crate::{
        config::ExecutionConfig,
        executor::{CommandSpec, ExecutionObserver, ExecutionRequest, Executor},
    };

    fn fixture() -> Result<(tempfile::TempDir, PathBuf, AdmissionCoordinator)> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("dispatch.db");
        let db = Database::open(&path)?;
        db.connection().execute(
            "INSERT INTO sources(id,path,kind,fingerprint,created_at) VALUES(1,'/fixture','directory','fixture',?1)",
            [stamp(Utc::now())],
        )?;
        for id in ["holder", "first", "second", "third"] {
            db.connection().execute(
                "INSERT INTO runs(id,source_id,task,exact_prompt,baseline_path,baseline_commit,status,created_at,dispatch_version,os,architecture,execution_backend,timeout_secs,cpus,memory,max_parallel,outcome_json) VALUES(?1,1,'task','prompt','/baseline','baseline','running',?2,'test','test','test','local',30,1.0,'1g',1,'{}')",
                params![id, stamp(Utc::now())],
            )?;
        }
        let coordinator =
            AdmissionCoordinator::new(&path, Duration::from_secs(20), Duration::from_secs(1));
        coordinator.register_pool("pool", "openai", "chatgpt-plus", &["codex".into()])?;
        Ok((temp, path, coordinator))
    }

    fn acquire(result: AcquireResult) -> LeaseToken {
        match result {
            AcquireResult::Acquired(token) => *token,
            other => panic!("expected acquired lease, got {other:?}"),
        }
    }

    fn mark_spawn_may_have_occurred(path: &PathBuf, token: &LeaseToken) -> Result<()> {
        let db = Database::open(path)?;
        db.connection().execute(
            "UPDATE pool_leases SET launch_lifecycle='spawn_may_have_occurred' WHERE pool_id=?1 AND request_id=?2 AND fence=?3",
            params![token.pool_id, token.request_id, integer(token.fence)?],
        )?;
        Ok(())
    }

    fn bound_request(
        path: &PathBuf,
        coordinator: &AdmissionCoordinator,
        run_id: &str,
        owner: &str,
        priority: i32,
    ) -> Result<AdmissionSummary> {
        let db = Database::open(path)?;
        let attempt_id = format!("attempt-{run_id}");
        db.connection().execute(
            "INSERT OR IGNORE INTO attempts(id,run_id,candidate_id,role,ordinal,generation,harness_id,started_at,outcome,raw_telemetry_path) VALUES(?1,?2,?3,'executor',1,1,'codex',?4,'preparing','fixture')",
            params![attempt_id, run_id, format!("candidate-{run_id}"), stamp(Utc::now())],
        )?;
        let observation = unknown_observation("pool", "default", Utc::now(), 300, "fixture");
        db.append_capacity_observation(&observation)?;
        let route_snapshot_json = "{}".to_owned();
        let configuration_revision = format!(
            "sha256:{}",
            hex::encode(Sha256::digest(route_snapshot_json.as_bytes()))
        );
        let authorization = authorize_observation(
            &db,
            "pool",
            &observation,
            &configuration_revision,
            1,
            "chatgpt-plus",
        )?;
        coordinator.enqueue_bound(
            run_id,
            "pool",
            owner,
            1,
            priority,
            &AdmissionBinding {
                attempt_id,
                route_snapshot_json,
                configuration_revision,
                canonical_pool_identity: canonical_pool_identity(
                    "openai",
                    "chatgpt-plus",
                    &["codex".into()],
                ),
                authorization_id: authorization.id,
                authorization_revision: 1,
            },
        )
    }

    #[test]
    fn current_process_identity_is_exact_when_supported() {
        let identity = ProcessIdentity::current();
        if identity.start.is_some() && identity.boot.is_some() {
            assert_eq!(identity_state(&identity), IdentityState::ExactLive);
        }
    }

    #[test]
    fn concurrent_admission_grants_exactly_one_pool_lease() -> Result<()> {
        use std::sync::{Arc, Barrier};

        let (_temp, _path, coordinator) = fixture()?;
        let first = coordinator.enqueue("first", "pool", "one", 1, 0)?;
        let second = coordinator.enqueue("second", "pool", "two", 1, 0)?;
        let barrier = Arc::new(Barrier::new(3));
        let handles = [(coordinator.clone(), first), (coordinator.clone(), second)]
            .into_iter()
            .map(|(coordinator, request)| {
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    coordinator.try_acquire(&request).unwrap()
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        let results = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, AcquireResult::Acquired(_)))
                .count(),
            1
        );
        Ok(())
    }

    #[test]
    fn bounded_aging_promotes_old_fifo_work() -> Result<()> {
        let (_temp, path, coordinator) = fixture()?;
        let holder = coordinator.enqueue("holder", "pool", "holder", 1, 1)?;
        let holder_token = acquire(coordinator.try_acquire(&holder)?);
        let background = coordinator.enqueue("first", "pool", "background", 1, -1)?;
        let urgent = coordinator.enqueue("second", "pool", "urgent", 1, 1)?;
        let db = Database::open(&path)?;
        db.connection().execute(
            "UPDATE admission_requests SET enqueued_at=?2 WHERE id=?1",
            params![
                background.request_id,
                stamp(Utc::now() - TimeDelta::seconds(3))
            ],
        )?;
        db.connection().execute(
            "UPDATE admission_requests SET enqueued_at=?2 WHERE id=?1",
            params![urgent.request_id, stamp(Utc::now() - TimeDelta::seconds(2))],
        )?;
        assert!(coordinator.release(&holder_token)?);
        assert_eq!(coordinator.try_acquire(&urgent)?, AcquireResult::Waiting);
        assert!(matches!(
            coordinator.try_acquire(&background)?,
            AcquireResult::Acquired(_)
        ));
        Ok(())
    }

    #[test]
    fn stale_generation_cannot_acquire_a_lease() -> Result<()> {
        let (_temp, path, coordinator) = fixture()?;
        let request = coordinator.enqueue("first", "pool", "owner", 1, 0)?;
        let db = Database::open(&path)?;
        db.connection().execute(
            "UPDATE admission_requests SET generation=2 WHERE id=?1",
            [&request.request_id],
        )?;
        assert!(coordinator.try_acquire(&request).is_err());
        let leases: i64 =
            db.connection()
                .query_row("SELECT COUNT(*) FROM pool_leases", [], |row| row.get(0))?;
        assert_eq!(leases, 0);
        Ok(())
    }

    #[test]
    fn lost_reconciliation_cas_does_not_mutate_request() -> Result<()> {
        let (_temp, path, coordinator) = fixture()?;
        let request = coordinator.enqueue("first", "pool", "owner", 1, 0)?;
        let token = acquire(coordinator.try_acquire(&request)?);
        let db = Database::open(&path)?;
        db.connection().execute(
            "UPDATE pool_leases SET launch_lifecycle='spawn_may_have_occurred',expires_at=?2 WHERE pool_id=?1",
            params![token.pool_id, stamp(Utc::now() - TimeDelta::seconds(1))],
        )?;
        db.connection().execute_batch(
            "CREATE TRIGGER lose_reconciliation BEFORE UPDATE OF state ON pool_leases WHEN NEW.state='reconciliation' BEGIN SELECT RAISE(IGNORE); END;",
        )?;
        coordinator.reconcile_expired("pool")?;
        let status: String = db.connection().query_row(
            "SELECT status FROM admission_requests WHERE id=?1",
            [&request.request_id],
            |row| row.get(0),
        )?;
        assert_eq!(status, "admitted");
        Ok(())
    }

    #[test]
    fn cancellation_before_spawn_releases_only_affirmatively_unlaunched_lease() -> Result<()> {
        let (_temp, _path, coordinator) = fixture()?;
        let first = coordinator.enqueue("first", "pool", "owner", 1, 0)?;
        let token = acquire(coordinator.try_acquire(&first)?);
        assert!(coordinator.release_not_launched(&token)?);
        let second = coordinator.enqueue("second", "pool", "next", 1, 0)?;
        assert!(matches!(
            coordinator.try_acquire(&second)?,
            AcquireResult::Acquired(_)
        ));
        Ok(())
    }

    #[test]
    fn cleanup_uncertainty_after_possible_spawn_never_releases() -> Result<()> {
        let (_temp, path, coordinator) = fixture()?;
        let request = coordinator.enqueue("first", "pool", "owner", 1, 0)?;
        let token = acquire(coordinator.try_acquire(&request)?);
        mark_spawn_may_have_occurred(&path, &token)?;
        coordinator.cleanup_after_unrecorded_spawn(
            &token,
            &ProcessIdentity::current(),
            None,
            false,
        )?;
        assert!(!coordinator.release(&token)?);
        assert!(!coordinator.release_not_launched(&token)?);
        Ok(())
    }

    #[derive(Debug)]
    struct FailingChildRecordObserver {
        inner: AdmissionLeaseObserver,
    }

    impl ExecutionObserver for FailingChildRecordObserver {
        fn authorize_launch(&self) -> Result<()> {
            self.inner.authorize_launch()
        }

        fn spawn_failed(&self) -> Result<()> {
            self.inner.spawn_failed()
        }

        fn child_spawned(
            &self,
            _identity: &ProcessIdentity,
            _backend_identity: Option<&str>,
        ) -> Result<()> {
            anyhow::bail!("injected child identity persistence failure")
        }

        fn cleanup_confirmed(&self, identity: &ProcessIdentity) -> Result<()> {
            self.inner.cleanup_confirmed(identity)
        }

        fn cleanup_after_unrecorded_spawn(
            &self,
            identity: &ProcessIdentity,
            backend_identity: Option<&str>,
            _confirmed: bool,
        ) -> Result<()> {
            self.inner
                .cleanup_after_unrecorded_spawn(identity, backend_identity, false)
        }
    }

    #[tokio::test]
    async fn spawned_child_persistence_and_artifact_failure_with_uncertain_cleanup_stays_fenced()
    -> Result<()> {
        let (temp, path, coordinator) = fixture()?;
        let request = bound_request(&path, &coordinator, "first", "owner", 1)?;
        let token = acquire(coordinator.try_acquire(&request)?);
        let invalid_log_path = temp.path().join("log-directory");
        std::fs::create_dir(&invalid_log_path)?;
        let execution = ExecutionRequest::new(
            CommandSpec::new("/bin/sh").args(["-c", "exit 0"]),
            temp.path(),
            &invalid_log_path,
            temp.path().join("stderr"),
        )
        .with_observer(std::sync::Arc::new(FailingChildRecordObserver {
            inner: AdmissionLeaseObserver::new(coordinator.clone(), token.clone()),
        }));
        assert!(
            Executor::new(ExecutionConfig::default())
                .execute(execution)
                .await
                .is_err()
        );
        assert!(!coordinator.release(&token)?);
        let db = Database::open(&path)?;
        let lifecycle: String = db.connection().query_row(
            "SELECT launch_lifecycle FROM pool_leases WHERE pool_id='pool'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(lifecycle, "cleanup_uncertain");
        Ok(())
    }

    #[test]
    fn ownership_loss_before_spawn_denies_launch() -> Result<()> {
        let (_temp, path, coordinator) = fixture()?;
        let request = bound_request(&path, &coordinator, "first", "owner", 1)?;
        let token = acquire(coordinator.try_acquire(&request)?);
        let db = Database::open(&path)?;
        db.connection().execute(
            "UPDATE pool_leases SET owner_session='superseded' WHERE pool_id='pool'",
            [],
        )?;
        assert!(coordinator.authorize_launch(&token).is_err());
        let lifecycle: String = db.connection().query_row(
            "SELECT launch_lifecycle FROM pool_leases WHERE pool_id='pool'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(lifecycle, "launch_intent_committed");
        Ok(())
    }

    #[derive(Debug)]
    struct CancelAtAuthorization {
        inner: AdmissionLeaseObserver,
        cancellation: crate::executor::CancellationToken,
        spawned: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    impl ExecutionObserver for CancelAtAuthorization {
        fn authorize_launch(&self) -> Result<()> {
            self.inner.authorize_launch()?;
            self.cancellation.cancel();
            Ok(())
        }
        fn spawn_failed(&self) -> Result<()> {
            self.inner.spawn_failed()
        }
        fn child_spawned(&self, identity: &ProcessIdentity, backend: Option<&str>) -> Result<()> {
            self.spawned
                .store(true, std::sync::atomic::Ordering::SeqCst);
            self.inner.child_spawned(identity, backend)
        }
        fn cleanup_confirmed(&self, identity: &ProcessIdentity) -> Result<()> {
            self.inner.cleanup_confirmed(identity)
        }
        fn cleanup_after_unrecorded_spawn(
            &self,
            identity: &ProcessIdentity,
            backend: Option<&str>,
            confirmed: bool,
        ) -> Result<()> {
            self.inner
                .cleanup_after_unrecorded_spawn(identity, backend, confirmed)
        }
    }

    #[tokio::test]
    async fn cancellation_during_final_authorization_spawns_no_child_and_disposes_lease()
    -> Result<()> {
        let (temp, path, coordinator) = fixture()?;
        let request = bound_request(&path, &coordinator, "first", "owner", 0)?;
        let token = acquire(coordinator.try_acquire(&request)?);
        let guard = AcquiredLeaseGuard::new(coordinator.clone(), token.clone());
        let cancellation = crate::executor::CancellationToken::new();
        let spawned = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let request = ExecutionRequest::new(
            CommandSpec::new("/bin/sh").args(["-c", "exit 0"]),
            temp.path(),
            temp.path().join("stdout"),
            temp.path().join("stderr"),
        )
        .with_observer(std::sync::Arc::new(CancelAtAuthorization {
            inner: AdmissionLeaseObserver::new(coordinator.clone(), token),
            cancellation: cancellation.clone(),
            spawned: spawned.clone(),
        }));
        let result = Executor::new(ExecutionConfig::default())
            .execute_with_cancel(request, cancellation)
            .await?;
        assert_eq!(result.status, crate::executor::ExecutionStatus::Cancelled);
        assert!(!spawned.load(std::sync::atomic::Ordering::SeqCst));
        drop(guard);
        assert_eq!(
            Database::open(&path)?
                .admission_summary_for_run("first")?
                .unwrap()
                .state,
            crate::AdmissionState::Released
        );
        Ok(())
    }

    // Use distinct sample times, independent of wall-clock/ULID generation order.
    fn funding_history_scenario(
        blocker: &str,
        buckets: &[&str],
        revision: u64,
        late_blocker: bool,
    ) -> Result<()> {
        let (_temp, path, coordinator) = fixture()?;
        let db = Database::open(&path)?;
        let sampled = Utc::now() - TimeDelta::seconds(10);
        let route = "{}";
        let route_revision = format!("sha256:{}", hex::encode(Sha256::digest(route.as_bytes())));
        let mut a = unknown_observation("pool", "default", sampled, 300, "baseline");
        a.id = "funding-a".into();
        a.auth_mode = crate::CapacityValue::Reported {
            value: "chatgpt".into(),
        };
        a.funding_identity = crate::CapacityValue::Reported {
            value: "account-a".into(),
        };
        a.plan_type = crate::CapacityValue::Reported {
            value: "plus".into(),
        };
        a.credits_available = crate::CapacityValue::Reported { value: false };
        a.service_tier = crate::CapacityValue::Reported {
            value: "standard".into(),
        };
        a.mapping = crate::CapacityMapping::Mapped;
        a.constraints[0].provider_bucket_id = Some("codex".into());
        a.constraints[0].window_id = Some("primary".into());
        db.append_capacity_observation(&a)?;
        authorize_observation(&db, "pool", &a, &route_revision, 1, "chatgpt-plus")?;
        let mut b = a.clone();
        b.id = "funding-b".into();
        b.sampled_at = sampled + TimeDelta::seconds(1);
        if blocker == "account" {
            b.funding_identity = crate::CapacityValue::Reported {
                value: "account-b".into(),
            };
        } else if blocker != "none" {
            b.credits_available = crate::CapacityValue::Reported { value: true };
        }
        let persist_blocker = || -> Result<()> {
            db.append_capacity_observation(&b)?;
            if blocker == "rejected" {
                assert!(
                    authorize_observation(&db, "pool", &b, &route_revision, 1, "chatgpt-plus")
                        .is_err()
                );
            }
            Ok(())
        };
        if !late_blocker {
            persist_blocker()?;
        }
        let buckets: Vec<String> = buckets.iter().map(|value| (*value).into()).collect();
        let pool = coordinator.register_pool("changed", "openai", "chatgpt-plus", &buckets)?;
        let mut c = unknown_observation(
            &pool,
            "default",
            sampled + TimeDelta::seconds(2),
            300,
            "failed probe",
        );
        c.id = "funding-c".into();
        db.append_capacity_observation(&c)?;
        let overlaps = buckets.is_empty() || buckets.iter().any(|bucket| bucket == "codex");
        let blocked = blocker != "none" && overlaps && revision == 1;
        if revision > 1 || blocker == "none" {
            if revision > 1 {
                // A mapping edit and Unknown do not clear the old epoch.
                assert!(
                    authorize_observation(&db, &pool, &c, &route_revision, 1, "chatgpt-plus")
                        .is_err()
                );
            }
            // Explicit revalidation reports safe funding facts and the new
            // mapping's own window set, which must not be conflated with A's.
            c = a.clone();
            c.id = "funding-d".into();
            c.pool_id = pool.clone();
            c.sampled_at = sampled + TimeDelta::seconds(3);
            c.constraints = buckets
                .iter()
                .map(|bucket| {
                    let mut window = a.constraints[0].clone();
                    window.provider_bucket_id = Some(bucket.clone());
                    window
                })
                .collect();
            db.append_capacity_observation(&c)?;
        }
        let authorization =
            authorize_observation(&db, &pool, &c, &route_revision, revision, "chatgpt-plus");
        if blocked && !late_blocker {
            let error = authorization.unwrap_err().to_string();
            assert!(error.contains("funding authorization revision"), "{error}");
            return Ok(());
        }
        let authorization = authorization?;
        db.connection().execute(
            "INSERT INTO attempts(id,run_id,candidate_id,role,ordinal,generation,harness_id,started_at,outcome,raw_telemetry_path) VALUES('funding-attempt','first','candidate-first','executor',1,1,'codex',?1,'preparing','fixture')",
            [stamp(sampled)],
        )?;
        let request = coordinator.enqueue_bound(
            "first",
            &pool,
            "owner",
            1,
            0,
            &AdmissionBinding {
                attempt_id: "funding-attempt".into(),
                route_snapshot_json: route.into(),
                configuration_revision: route_revision.clone(),
                canonical_pool_identity: canonical_pool_identity(
                    "openai",
                    "chatgpt-plus",
                    &buckets,
                ),
                authorization_id: authorization.id,
                authorization_revision: revision,
            },
        )?;
        let token = acquire(coordinator.try_acquire(&request)?);
        if late_blocker {
            persist_blocker()?;
        }
        let result = coordinator.authorize_launch(&token);
        if blocked {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("funding revalidation failed")
            );
            let lifecycle: String = db.connection().query_row(
                "SELECT launch_lifecycle FROM pool_leases WHERE pool_id=?1",
                [&pool],
                |row| row.get(0),
            )?;
            assert_eq!(lifecycle, "launch_intent_committed");
        } else {
            result?;
            coordinator.spawn_failed(&token)?;
        }
        assert!(coordinator.release_not_launched(&token)?);
        Ok(())
    }

    #[test]
    fn overlapping_funding_history_retains_raw_account_conflict() -> Result<()> {
        for late in [false, true] {
            funding_history_scenario("account", &["codex", "additional"], 1, late)?;
        }
        Ok(())
    }

    #[test]
    fn overlapping_funding_history_retains_raw_paid_credit_conflict() -> Result<()> {
        for late in [false, true] {
            funding_history_scenario("paid", &["codex", "additional"], 1, late)?;
        }
        Ok(())
    }

    #[test]
    fn overlapping_funding_history_retains_persisted_rejection() -> Result<()> {
        for late in [false, true] {
            funding_history_scenario("rejected", &["codex", "additional"], 1, late)?;
        }
        Ok(())
    }

    #[test]
    fn disjoint_funding_history_does_not_inherit_blockers() -> Result<()> {
        for blocker in ["account", "paid", "rejected"] {
            funding_history_scenario(blocker, &["independent-other-pool"], 1, false)?;
        }
        Ok(())
    }

    #[test]
    fn overlapping_funding_history_allows_explicit_revalidation() -> Result<()> {
        for blocker in ["account", "paid", "rejected"] {
            funding_history_scenario(blocker, &["codex", "additional"], 2, false)?;
        }
        Ok(())
    }

    #[test]
    fn unknown_mapping_retains_funding_history() -> Result<()> {
        for late in [false, true] {
            funding_history_scenario("account", &[], 1, late)?;
        }
        Ok(())
    }

    #[test]
    fn overlapping_funding_history_does_not_conflate_window_sets() -> Result<()> {
        funding_history_scenario("none", &["codex", "additional"], 1, false)
    }

    #[test]
    fn raw_conflict_then_unknown_cannot_cross_final_launch_fence() -> Result<()> {
        let (_temp, path, coordinator) = fixture()?;
        let request = bound_request(&path, &coordinator, "first", "owner", 1)?;
        let token = acquire(coordinator.try_acquire(&request)?);
        let db = Database::open(&path)?;
        let mut a = unknown_observation("pool", "default", Utc::now(), 300, "account-a");
        a.funding_identity = crate::CapacityValue::Reported {
            value: "account-a".into(),
        };
        db.append_capacity_observation(&a)?;
        authorize_observation(
            &db,
            "pool",
            &a,
            &token.configuration_revision,
            1,
            "chatgpt-plus",
        )?;
        let mut b = a.clone();
        b.id = Ulid::new().to_string();
        b.sampled_at = Utc::now();
        b.funding_identity = crate::CapacityValue::Reported {
            value: "account-b".into(),
        };
        db.append_capacity_observation(&b)?; // Writer dies here; no rejected authorization exists.
        let c = unknown_observation("pool", "default", Utc::now(), 300, "failed probe");
        db.append_capacity_observation(&c)?;
        assert!(coordinator.authorize_launch(&token).is_err());
        assert!(
            authorize_observation(
                &db,
                "pool",
                &c,
                &token.configuration_revision,
                1,
                "chatgpt-plus"
            )
            .is_err()
        );
        assert!(coordinator.release_not_launched(&token)?);
        Ok(())
    }

    #[test]
    fn final_fence_retains_rejection_until_same_constraints_are_reported_available() -> Result<()> {
        let (_temp, path, coordinator) = fixture()?;
        let request = bound_request(&path, &coordinator, "first", "owner", 0)?;
        let token = acquire(coordinator.try_acquire(&request)?);
        let db = Database::open(&path)?;
        let mut rejected = unknown_observation("pool", "default", Utc::now(), 300, "reported");
        rejected.mapping = crate::CapacityMapping::Mapped;
        rejected.scarcity = crate::ScarcityState::Exhausted;
        let mut window = rejected.constraints[0].clone();
        window.provider_bucket_id = Some("codex".into());
        window.window_id = Some("primary".into());
        window.scope = crate::CapacityValue::Reported {
            value: "pool".into(),
        };
        window.remaining = crate::CapacityValue::Reported { value: 0.0 };
        let mut secondary = window.clone();
        secondary.window_id = Some("secondary".into());
        secondary.remaining = crate::CapacityValue::Reported { value: 90.0 };
        let mut rejection = window.clone();
        rejection.kind = "provider_rejection".into();
        rejection.window_id = None;
        rejected.constraints = vec![window, secondary, rejection];
        db.append_capacity_observation(&rejected)?;
        let unknown = unknown_observation("pool", "default", Utc::now(), 300, "failed probe");
        db.append_capacity_observation(&unknown)?;
        assert!(coordinator.authorize_launch(&token).is_err());
        let mut available = rejected.clone();
        available.id = Ulid::new().to_string();
        available.sampled_at = Utc::now();
        available.scarcity = crate::ScarcityState::Available;
        available.constraints.pop();
        available.constraints[0].remaining = crate::CapacityValue::Reported { value: 90.0 };
        db.append_capacity_observation(&available)?;
        coordinator.authorize_launch(&token)?;
        coordinator.spawn_failed(&token)?;
        assert!(coordinator.release_not_launched(&token)?);
        Ok(())
    }

    #[test]
    fn newer_shared_exhaustion_is_rechecked_at_launch_boundary() -> Result<()> {
        let (_temp, path, coordinator) = fixture()?;
        let request = bound_request(&path, &coordinator, "first", "owner", 1)?;
        let token = acquire(coordinator.try_acquire(&request)?);
        let db = Database::open(&path)?;
        let mut exhausted = unknown_observation(
            "pool",
            "default",
            Utc::now() + TimeDelta::milliseconds(1),
            300,
            "independent foreground process",
        );
        exhausted.scarcity = crate::ScarcityState::Exhausted;
        db.append_capacity_observation(&exhausted)?;
        assert!(coordinator.authorize_launch(&token).is_err());
        let lifecycle: String = db.connection().query_row(
            "SELECT launch_lifecycle FROM pool_leases WHERE pool_id='pool'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(lifecycle, "launch_intent_committed");
        Ok(())
    }

    #[test]
    fn overlapping_mappings_exclude_but_disjoint_and_unrelated_pools_do_not() -> Result<()> {
        let (_temp, _path, coordinator) = fixture()?;
        let holder = coordinator.enqueue("holder", "pool", "old-configuration", 1, 0)?;
        let token = acquire(coordinator.try_acquire(&holder)?);
        let expanded = coordinator.register_pool(
            "expanded",
            "openai",
            "chatgpt-plus",
            &["codex".into(), "additional".into()],
        )?;
        let request = coordinator.enqueue("first", &expanded, "new-configuration", 1, 0)?;
        assert_eq!(coordinator.try_acquire(&request)?, AcquireResult::Waiting);
        let disjoint = coordinator.register_pool(
            "disjoint",
            "openai",
            "chatgpt-plus",
            &["independent".into()],
        )?;
        let independent = coordinator.enqueue("second", &disjoint, "independent", 1, 0)?;
        assert!(matches!(
            coordinator.try_acquire(&independent)?,
            AcquireResult::Acquired(_)
        ));
        let unknown = coordinator.register_pool("unknown", "openai", "chatgpt-plus", &[])?;
        let unknown_request = coordinator.enqueue("third", &unknown, "unknown", 1, 0)?;
        assert_eq!(
            coordinator.try_acquire(&unknown_request)?,
            AcquireResult::Waiting
        );
        assert!(coordinator.release(&token)?);
        assert!(matches!(
            coordinator.try_acquire(&request)?,
            AcquireResult::Acquired(_)
        ));
        let unrelated = coordinator.register_pool(
            "unrelated",
            "another-provider",
            "another-funding-source",
            &[],
        )?;
        let other = coordinator.enqueue("holder", &unrelated, "unrelated", 1, 0)?;
        assert!(matches!(
            coordinator.try_acquire(&other)?,
            AcquireResult::Acquired(_)
        ));
        Ok(())
    }

    #[test]
    fn stale_pre_upgrade_binding_cannot_launch() -> Result<()> {
        let (_temp, path, coordinator) = fixture()?;
        let previous = bound_request(&path, &coordinator, "first", "owner", 0)?;
        coordinator.cancel_queued(&previous)?;
        coordinator.register_pool("pool", "openai", "chatgpt-prolite", &["codex".into()])?;
        // Simulate a process paused after registering/probing the old allowance,
        // which only enqueues after another process commits the upgrade.
        let delayed = coordinator.enqueue_bound(
            "first",
            "pool",
            "owner",
            2,
            0,
            &AdmissionBinding {
                attempt_id: previous.attempt_id,
                route_snapshot_json: previous.route_snapshot_json,
                configuration_revision: previous.configuration_revision,
                canonical_pool_identity: previous.canonical_pool_identity,
                authorization_id: previous.authorization_id,
                authorization_revision: previous.authorization_revision,
            },
        )?;
        let token = acquire(coordinator.try_acquire(&delayed)?);
        assert!(coordinator.authorize_launch(&token).is_err());
        assert!(coordinator.release(&token)?);
        Ok(())
    }

    #[test]
    fn funding_upgrade_preserves_pool_history_and_fences() -> Result<()> {
        let (_temp, path, coordinator) = fixture()?;
        let request = coordinator.enqueue("first", "pool", "owner", 1, 0)?;
        let old = acquire(coordinator.try_acquire(&request)?);
        assert!(coordinator.release(&old)?);
        let db = Database::open(&path)?;
        let mut observation =
            unknown_observation("pool", "default", Utc::now(), 300, "before upgrade");
        observation.plan_type = crate::CapacityValue::Reported {
            value: "prolite".into(),
        };
        db.append_capacity_observation(&observation)?;
        assert!(
            authorize_observation(&db, "pool", &observation, "route", 1, "chatgpt-plus").is_err()
        );

        let upgraded =
            coordinator.register_pool("pool", "openai", "chatgpt-prolite", &["codex".into()])?;
        assert_eq!(upgraded, "pool");
        let history: i64 = db.connection().query_row(
            "SELECT count(*) FROM capacity_observations WHERE pool_id='pool'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(history, 1);
        // Updating the mapping cannot itself renew an invalid funding epoch.
        assert!(
            authorize_observation(&db, "pool", &observation, "route", 1, "chatgpt-prolite")
                .is_err()
        );
        authorize_observation(&db, "pool", &observation, "route", 2, "chatgpt-prolite")?;
        let next = coordinator.enqueue("second", &upgraded, "next", 1, 0)?;
        let new = acquire(coordinator.try_acquire(&next)?);
        assert!(new.fence > old.fence);
        assert!(!coordinator.release(&old)?);
        assert!(coordinator.release(&new)?);
        Ok(())
    }

    #[test]
    fn funding_upgrade_refuses_queued_owned_and_uncertain_overlapping_work() -> Result<()> {
        let (_temp, path, coordinator) = fixture()?;
        let overlap = coordinator.register_pool("overlap", "openai", "chatgpt-plus", &[])?;
        let request = coordinator.enqueue("first", &overlap, "owner", 1, 0)?;
        let upgrade =
            || coordinator.register_pool("pool", "openai", "chatgpt-prolite", &["codex".into()]);
        assert!(
            upgrade()
                .unwrap_err()
                .to_string()
                .contains("still has queued or owned work")
        );
        let token = acquire(coordinator.try_acquire(&request)?);
        assert!(upgrade().is_err());
        let db = Database::open(&path)?;
        db.connection()
            .execute("UPDATE pool_leases SET state='reconciliation'", [])?;
        assert!(upgrade().is_err());
        let funding: String = db.connection().query_row(
            "SELECT funding_source FROM resource_pools WHERE id='pool'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(funding, "chatgpt-plus");
        assert!(coordinator.release(&token)?);
        assert_eq!(upgrade()?, "pool");
        Ok(())
    }

    #[test]
    fn renamed_pool_resolves_to_the_owned_canonical_allowance() -> Result<()> {
        let (_temp, _path, coordinator) = fixture()?;
        let old =
            coordinator.register_pool("old-name", "openai", "chatgpt-plus", &["codex".into()])?;
        let holder = coordinator.enqueue("holder", &old, "owner", 1, 0)?;
        let _token = acquire(coordinator.try_acquire(&holder)?);
        let renamed =
            coordinator.register_pool("new-name", "openai", "chatgpt-plus", &["codex".into()])?;
        assert_eq!(renamed, old);
        let waiter = coordinator.enqueue("second", &renamed, "next", 1, 0)?;
        assert_eq!(coordinator.try_acquire(&waiter)?, AcquireResult::Waiting);
        Ok(())
    }

    #[test]
    fn stale_generation_cannot_release_newer_lease() -> Result<()> {
        let (_temp, _path, coordinator) = fixture()?;
        let first = coordinator.enqueue("first", "pool", "owner", 1, 0)?;
        let old = acquire(coordinator.try_acquire(&first)?);
        assert!(coordinator.release(&old)?);
        let second = coordinator.enqueue("second", "pool", "owner", 2, 0)?;
        let new = acquire(coordinator.try_acquire(&second)?);
        assert!(new.fence > old.fence);
        assert!(!coordinator.release(&old)?);
        assert!(coordinator.heartbeat(&new)?);
        Ok(())
    }

    #[test]
    fn a_recorded_child_requires_fenced_cleanup_before_release() -> Result<()> {
        let (_temp, path, coordinator) = fixture()?;
        let request = coordinator.enqueue("first", "pool", "owner", 1, 0)?;
        let token = acquire(coordinator.try_acquire(&request)?);
        mark_spawn_may_have_occurred(&path, &token)?;
        let child = ProcessIdentity::current();
        coordinator.child_spawned(&token, &child, None)?;
        assert!(!coordinator.release(&token)?);
        coordinator.cleanup_confirmed(&token, &child)?;
        assert!(coordinator.release(&token)?);
        Ok(())
    }

    #[test]
    fn owner_death_around_spawn_and_uncertain_child_both_block_replacement() -> Result<()> {
        let (_temp, path, coordinator) = fixture()?;
        let first = coordinator.enqueue("first", "pool", "owner", 1, 0)?;
        let token = acquire(coordinator.try_acquire(&first)?);
        let db = Database::open(&path)?;
        db.connection().execute(
            "UPDATE pool_leases SET launch_lifecycle='spawn_may_have_occurred',expires_at=?2,owner_pid=4294967295,owner_start_identity='dead',owner_boot_identity='dead' WHERE pool_id=?1",
            params![token.pool_id, stamp(Utc::now() - TimeDelta::seconds(1))],
        )?;
        coordinator.reconcile_expired("pool")?;
        let state: String = db.connection().query_row(
            "SELECT state FROM pool_leases WHERE pool_id='pool'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(state, "reconciliation");
        db.connection().execute(
            "UPDATE pool_leases SET expires_at=?2,owner_pid=4294967295,owner_start_identity='dead',owner_boot_identity='dead',child_pid=4294967294,child_start_identity=NULL,child_boot_identity=NULL,child_process_group=NULL WHERE pool_id=?1",
            params![token.pool_id, stamp(Utc::now() - TimeDelta::seconds(1))],
        )?;
        coordinator.reconcile_expired("pool")?;
        let state: String = db.connection().query_row(
            "SELECT state FROM pool_leases WHERE pool_id='pool'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(state, "reconciliation");
        Ok(())
    }

    #[test]
    fn dead_queued_owner_does_not_block_fifo_forever() -> Result<()> {
        let (_temp, path, coordinator) = fixture()?;
        let first = coordinator.enqueue("first", "pool", "dead", 1, 0)?;
        let second = coordinator.enqueue("second", "pool", "live", 1, 0)?;
        let db = Database::open(&path)?;
        db.connection().execute(
            "UPDATE admission_requests SET expires_at=?2,owner_pid=4294967295,owner_start_identity='dead',owner_boot_identity='dead' WHERE id=?1",
            params![first.request_id, stamp(Utc::now() - TimeDelta::seconds(1))],
        )?;
        assert!(matches!(
            coordinator.try_acquire(&second)?,
            AcquireResult::Acquired(_)
        ));
        let status: String = db.connection().query_row(
            "SELECT status FROM admission_requests WHERE id=?1",
            [first.request_id],
            |row| row.get(0),
        )?;
        assert_eq!(status, "abandoned");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn live_orphan_and_reused_pid_with_uncertain_group_block_replacement() -> Result<()> {
        use std::os::unix::process::CommandExt;

        let (_temp, path, coordinator) = fixture()?;
        let first = coordinator.enqueue("first", "pool", "owner", 1, 0)?;
        let token = acquire(coordinator.try_acquire(&first)?);
        mark_spawn_may_have_occurred(&path, &token)?;
        let mut command = std::process::Command::new("/bin/sleep");
        command.arg("5").process_group(0);
        let mut child = command.spawn()?;
        coordinator.child_spawned(&token, &process_identity(child.id()), None)?;
        let db = Database::open(&path)?;
        db.connection().execute(
            "UPDATE pool_leases SET expires_at=?2,owner_pid=4294967295,owner_start_identity='dead',owner_boot_identity='dead' WHERE pool_id=?1",
            params![token.pool_id, stamp(Utc::now() - TimeDelta::seconds(1))],
        )?;
        coordinator.reconcile_expired("pool")?;
        let second = coordinator.enqueue("second", "pool", "next", 1, 0)?;
        assert_eq!(
            coordinator.try_acquire(&second)?,
            AcquireResult::Reconciliation
        );
        let _ = child.kill();
        let _ = child.wait();
        coordinator.reconcile_expired("pool")?;
        let second_token = acquire(coordinator.try_acquire(&second)?);
        mark_spawn_may_have_occurred(&path, &second_token)?;

        let current = ProcessIdentity::current();
        let reused = ProcessIdentity {
            start: Some("different-start".into()),
            ..current.clone()
        };
        if current.start.is_some() && current.boot.is_some() {
            assert_eq!(identity_state(&reused), IdentityState::Reused);
        }
        let db = Database::open(&path)?;
        db.connection().execute(
            "UPDATE pool_leases SET expires_at=?1,owner_pid=4294967295,owner_start_identity='dead',owner_boot_identity='dead',child_pid=?2,child_start_identity='different-start',child_boot_identity=?3,child_process_group=NULL",
            params![
                stamp(Utc::now() - TimeDelta::seconds(1)),
                current.pid,
                current.boot,
            ],
        )?;
        coordinator.reconcile_expired("pool")?;
        let count: i64 = db.connection().query_row(
            "SELECT COUNT(*) FROM pool_leases WHERE pool_id='pool'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(count, 1, "PID reuse without group proof stays conservative");
        let state: String = db.connection().query_row(
            "SELECT state FROM pool_leases WHERE pool_id='pool'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(state, "reconciliation");
        Ok(())
    }
}
