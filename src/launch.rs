//! The durable record of each agent launch, and the rule crash repair uses to
//! decide whether a launched agent may still be alive.
//!
//! `LaunchObserver` is the executor's observer for a native attempt. It
//! commits `intent` before the process is spawned, the child's identity once
//! it is, and how cleanup ended. A supervisor killed at any point therefore
//! leaves enough behind for a later process to tell whether the agent can
//! still be writing.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use chrono::Utc;
use rusqlite::params;

use crate::{
    config::{ResourceConfig, ResourceProfile},
    db::Database,
    executor::ExecutionObserver,
    process::{IdentityState, ProcessIdentity, identity_state, process_group_exists},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchState {
    /// Committed before spawn: the process may have been started.
    Intent,
    SpawnFailed,
    Spawned,
    Cleaned,
    /// Started, but its cleanup could not be confirmed.
    Uncertain,
}

impl LaunchState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Intent => "intent",
            Self::SpawnFailed => "spawn_failed",
            Self::Spawned => "spawned",
            Self::Cleaned => "cleaned",
            Self::Uncertain => "uncertain",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        Ok(match value {
            "intent" => Self::Intent,
            "spawn_failed" => Self::SpawnFailed,
            "spawned" => Self::Spawned,
            "cleaned" => Self::Cleaned,
            "uncertain" => Self::Uncertain,
            _ => anyhow::bail!("unknown launch state {value:?}"),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRecord {
    pub attempt_id: String,
    pub state: LaunchState,
    pub child: Option<ProcessIdentity>,
    pub backend_identity: Option<String>,
}

impl LaunchRecord {
    /// Whether the launched agent may still be running. Anything short of
    /// proof that it has stopped counts as alive: an intent without a
    /// recorded child, a container, or a child or process group that cannot
    /// be shown gone.
    pub fn possibly_alive(&self) -> bool {
        match self.state {
            LaunchState::SpawnFailed | LaunchState::Cleaned => false,
            LaunchState::Intent => true,
            LaunchState::Spawned | LaunchState::Uncertain => {
                let Some(child) = &self.child else {
                    return true;
                };
                self.backend_identity.is_some()
                    || !matches!(
                        identity_state(child),
                        IdentityState::Gone | IdentityState::Reused
                    )
                    || process_group_exists(child.process_group) != Some(false)
            }
        }
    }
}

/// Every launch recorded for `run_id`.
pub fn launches_for_run(db: &Database, run_id: &str) -> Result<Vec<LaunchRecord>> {
    let mut statement = db.connection().prepare(
        "SELECT attempt_id, state, child_json, backend_identity FROM attempt_launches WHERE run_id = ?1",
    )?;
    let rows = statement.query_map([run_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    })?;
    rows.map(|row| {
        let (attempt_id, state, child, backend_identity) = row?;
        Ok(LaunchRecord {
            attempt_id,
            state: LaunchState::parse(&state)?,
            child: child.map(|json| serde_json::from_str(&json)).transpose()?,
            backend_identity,
        })
    })
    .collect()
}

/// Checked again at the launch boundary, after the adapter preflight and
/// immediately before spawn: the bound profile must still be configured
/// exactly as selected, and its funding must not have been refused.
#[derive(Debug, Clone)]
pub struct LaunchGuard {
    pub state_root: PathBuf,
    pub profile: ResourceProfile,
}

impl LaunchGuard {
    fn check(&self, db: &Database) -> Result<()> {
        let current = ResourceConfig::load(&self.state_root)?;
        let bound = serde_json::to_value(&self.profile).ok();
        if !current
            .profiles
            .iter()
            .any(|p| p.enabled && serde_json::to_value(p).ok() == bound)
        {
            return Err(LaunchRefused(
                "resource configuration changed after selection; stale launch refused".into(),
            )
            .into());
        }
        if let Some(reason) = db.funding_refusal(
            &self.profile.funding_key(),
            self.profile.authorization_revision,
        )? {
            return Err(
                LaunchRefused(format!("funding was refused before launch: {reason}")).into(),
            );
        }
        Ok(())
    }
}

/// The launch guard refused at the spawn boundary; nothing was spawned.
#[derive(Debug)]
pub struct LaunchRefused(pub String);

impl std::fmt::Display for LaunchRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LaunchRefused {}

/// Records one attempt's launch; an optional inner observer is consulted
/// first at every step.
#[derive(Debug)]
pub struct LaunchObserver {
    db_path: PathBuf,
    run_id: String,
    attempt_id: String,
    guard: Option<LaunchGuard>,
    inner: Option<Arc<dyn ExecutionObserver>>,
}

impl LaunchObserver {
    pub fn new(
        db_path: &Path,
        run_id: &str,
        attempt_id: &str,
        guard: Option<LaunchGuard>,
        inner: Option<Arc<dyn ExecutionObserver>>,
    ) -> Self {
        Self {
            db_path: db_path.to_owned(),
            run_id: run_id.to_owned(),
            attempt_id: attempt_id.to_owned(),
            guard,
            inner,
        }
    }

    fn record(
        &self,
        state: LaunchState,
        child: Option<&ProcessIdentity>,
        backend_identity: Option<&str>,
    ) -> Result<()> {
        let db = Database::open(&self.db_path)?;
        let child = child.map(serde_json::to_string).transpose()?;
        db.connection().execute(
            "INSERT INTO attempt_launches(attempt_id, run_id, state, child_json, backend_identity, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(attempt_id) DO UPDATE SET state = excluded.state, \
                 child_json = COALESCE(excluded.child_json, attempt_launches.child_json), \
                 backend_identity = COALESCE(excluded.backend_identity, attempt_launches.backend_identity), \
                 updated_at = excluded.updated_at",
            params![
                self.attempt_id,
                self.run_id,
                state.as_str(),
                child,
                backend_identity,
                Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }
}

impl ExecutionObserver for LaunchObserver {
    fn provider_failure(&self, failure: crate::FailureKind) -> Result<()> {
        match &self.inner {
            Some(inner) => inner.provider_failure(failure),
            None => Ok(()),
        }
    }

    fn preflight_failed(&self) -> Result<()> {
        match &self.inner {
            Some(inner) => inner.preflight_failed(),
            None => Ok(()),
        }
    }

    fn authorize_launch(&self) -> Result<()> {
        if let Some(guard) = &self.guard {
            guard.check(&Database::open(&self.db_path)?)?;
        }
        if let Some(inner) = &self.inner {
            inner.authorize_launch()?;
        }
        // Nothing may be spawned unless this is durable first.
        self.record(LaunchState::Intent, None, None)
    }

    fn spawn_failed(&self) -> Result<()> {
        self.record(LaunchState::SpawnFailed, None, None)?;
        match &self.inner {
            Some(inner) => inner.spawn_failed(),
            None => Ok(()),
        }
    }

    fn child_spawned(
        &self,
        identity: &ProcessIdentity,
        backend_identity: Option<&str>,
    ) -> Result<()> {
        self.record(LaunchState::Spawned, Some(identity), backend_identity)?;
        match &self.inner {
            Some(inner) => inner.child_spawned(identity, backend_identity),
            None => Ok(()),
        }
    }

    fn cleanup_confirmed(&self, identity: &ProcessIdentity) -> Result<()> {
        self.record(LaunchState::Cleaned, Some(identity), None)?;
        match &self.inner {
            Some(inner) => inner.cleanup_confirmed(identity),
            None => Ok(()),
        }
    }

    fn cleanup_after_unrecorded_spawn(
        &self,
        identity: &ProcessIdentity,
        backend_identity: Option<&str>,
        confirmed: bool,
    ) -> Result<()> {
        let state = if confirmed {
            LaunchState::Cleaned
        } else {
            LaunchState::Uncertain
        };
        self.record(state, Some(identity), backend_identity)?;
        match &self.inner {
            Some(inner) => {
                inner.cleanup_after_unrecorded_spawn(identity, backend_identity, confirmed)
            }
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        os::unix::process::CommandExt,
        process::{Child, Command},
        sync::atomic::{AtomicBool, Ordering},
    };

    use super::*;
    use crate::{
        config::ExecutionConfig,
        executor::{CancellationToken, CommandSpec, ExecutionRequest, ExecutionStatus, Executor},
        process::process_identity,
    };

    fn record(state: LaunchState, child: Option<ProcessIdentity>) -> LaunchRecord {
        LaunchRecord {
            attempt_id: "attempt".into(),
            state,
            child,
            backend_identity: None,
        }
    }

    /// A process that leads its own process group, as the executor starts agents.
    fn group_leader(script: &str) -> Child {
        Command::new("/bin/sh")
            .args(["-c", script])
            .process_group(0)
            .spawn()
            .unwrap()
    }

    fn kill_group(child: &Child) {
        // SAFETY: signals only the process group this test created.
        unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
    }

    fn wait_until_group_gone(identity: &ProcessIdentity) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while process_group_exists(identity.process_group) != Some(false) {
            assert!(
                std::time::Instant::now() < deadline,
                "process group did not exit"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn launches_that_never_started_or_were_cleaned_up_are_not_alive() {
        assert!(!record(LaunchState::SpawnFailed, None).possibly_alive());
        assert!(!record(LaunchState::Cleaned, None).possibly_alive());
    }

    #[test]
    fn an_intent_without_a_recorded_child_may_have_spawned() {
        assert!(record(LaunchState::Intent, None).possibly_alive());
        assert!(record(LaunchState::Spawned, None).possibly_alive());
    }

    #[test]
    fn a_running_child_is_alive_until_it_and_its_group_are_gone() {
        let mut child = group_leader("sleep 30");
        let identity = process_identity(child.id());
        assert!(record(LaunchState::Spawned, Some(identity.clone())).possibly_alive());
        kill_group(&child);
        child.wait().unwrap();
        wait_until_group_gone(&identity);
        assert!(!record(LaunchState::Spawned, Some(identity.clone())).possibly_alive());
        assert!(!record(LaunchState::Uncertain, Some(identity)).possibly_alive());
    }

    #[test]
    fn a_gone_child_whose_group_lives_on_is_still_alive() {
        // The leader exits at once; a background process stays in its group.
        let mut child = group_leader("sleep 30 & exit 0");
        let identity = process_identity(child.id());
        child.wait().unwrap();
        assert!(record(LaunchState::Uncertain, Some(identity.clone())).possibly_alive());
        kill_group(&child);
        wait_until_group_gone(&identity);
        assert!(!record(LaunchState::Uncertain, Some(identity)).possibly_alive());
    }

    #[test]
    fn a_container_launch_is_alive_even_when_its_host_process_is_gone() {
        let mut child = group_leader("exit 0");
        let identity = process_identity(child.id());
        child.wait().unwrap();
        wait_until_group_gone(&identity);
        let mut launch = record(LaunchState::Spawned, Some(identity));
        launch.backend_identity = Some("dispatch-container".into());
        assert!(launch.possibly_alive());
    }

    fn database() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("dispatch.db");
        Database::open(&path).unwrap();
        (temp, path)
    }

    fn launches(path: &Path) -> Vec<LaunchRecord> {
        launches_for_run(&Database::open(path).unwrap(), "run").unwrap()
    }

    #[test]
    fn intent_is_durable_before_anything_is_spawned() {
        let (_temp, path) = database();
        let observer = LaunchObserver::new(&path, "run", "attempt", None, None);
        observer.authorize_launch().unwrap();
        assert_eq!(launches(&path), vec![record(LaunchState::Intent, None)]);
    }

    #[tokio::test]
    async fn a_completed_launch_records_its_child_and_confirmed_cleanup() {
        let (temp, path) = database();
        let request = ExecutionRequest::new(
            CommandSpec::new("/bin/sh").args(["-c", "exit 0"]),
            temp.path(),
            temp.path().join("stdout"),
            temp.path().join("stderr"),
        )
        .with_observer(Arc::new(LaunchObserver::new(
            &path, "run", "attempt", None, None,
        )));
        let result = Executor::new(ExecutionConfig::default())
            .execute_with_cancel(request, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(result.status, ExecutionStatus::Succeeded);
        let [launch] = launches(&path).try_into().unwrap();
        assert_eq!(launch.state, LaunchState::Cleaned);
        assert!(launch.child.is_some());
        assert!(!launch.possibly_alive());
    }

    #[derive(Debug)]
    struct CancelAtAuthorization {
        cancellation: CancellationToken,
        spawned: Arc<AtomicBool>,
    }

    impl ExecutionObserver for CancelAtAuthorization {
        fn authorize_launch(&self) -> Result<()> {
            self.cancellation.cancel();
            Ok(())
        }
        fn spawn_failed(&self) -> Result<()> {
            Ok(())
        }
        fn child_spawned(&self, _: &ProcessIdentity, _: Option<&str>) -> Result<()> {
            self.spawned.store(true, Ordering::SeqCst);
            Ok(())
        }
        fn cleanup_confirmed(&self, _: &ProcessIdentity) -> Result<()> {
            Ok(())
        }
        fn cleanup_after_unrecorded_spawn(
            &self,
            _: &ProcessIdentity,
            _: Option<&str>,
            _: bool,
        ) -> Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn a_launch_cancelled_after_intent_records_that_nothing_spawned() {
        let (temp, path) = database();
        let cancellation = CancellationToken::new();
        let spawned = Arc::new(AtomicBool::new(false));
        let inner = Arc::new(CancelAtAuthorization {
            cancellation: cancellation.clone(),
            spawned: spawned.clone(),
        });
        let request = ExecutionRequest::new(
            CommandSpec::new("/bin/sh").args(["-c", "exit 0"]),
            temp.path(),
            temp.path().join("stdout"),
            temp.path().join("stderr"),
        )
        .with_observer(Arc::new(LaunchObserver::new(
            &path,
            "run",
            "attempt",
            None,
            Some(inner),
        )));
        let result = Executor::new(ExecutionConfig::default())
            .execute_with_cancel(request, cancellation)
            .await
            .unwrap();
        assert_eq!(result.status, ExecutionStatus::Cancelled);
        assert!(!spawned.load(Ordering::SeqCst));
        assert_eq!(
            launches(&path),
            vec![record(LaunchState::SpawnFailed, None)]
        );
    }

    #[test]
    fn an_unrecorded_spawn_keeps_its_identity_and_whether_cleanup_was_confirmed() {
        let (_temp, path) = database();
        let identity = ProcessIdentity::current();
        let observer = LaunchObserver::new(&path, "run", "attempt", None, None);
        observer.authorize_launch().unwrap();
        observer
            .cleanup_after_unrecorded_spawn(&identity, None, false)
            .unwrap();
        let [launch] = launches(&path).try_into().unwrap();
        assert_eq!(launch.state, LaunchState::Uncertain);
        assert_eq!(launch.child, Some(identity.clone()));
        assert!(launch.possibly_alive(), "this test process is alive");
        observer
            .cleanup_after_unrecorded_spawn(&identity, None, true)
            .unwrap();
        assert_eq!(launches(&path)[0].state, LaunchState::Cleaned);
    }

    fn profile() -> ResourceProfile {
        serde_yaml::from_str(
            "provider: openai\nfunding_source: chatgpt-plus\nharness: codex\nmodel: m\neffort: low\n\
             service_mode: standard\nruntime: local\npool: p\ntier: light\nincluded: true\n\
             no_overage_verified: true\n",
        )
        .unwrap()
    }

    fn write_resources(root: &Path, profile: &ResourceProfile) {
        let config = format!(
            "version: 1\nallocation_enabled: true\nprofiles:\n{}",
            serde_yaml::to_string(&vec![profile]).unwrap()
        );
        std::fs::write(root.join("resources.yml"), config).unwrap();
    }

    #[test]
    fn the_guard_refuses_a_changed_profile_or_refused_funding_before_intent() {
        let (temp, path) = database();
        let bound = profile();
        write_resources(temp.path(), &bound);
        let guard = LaunchGuard {
            state_root: temp.path().to_owned(),
            profile: bound.clone(),
        };
        let observer = LaunchObserver::new(&path, "run", "attempt", Some(guard), None);
        observer.authorize_launch().unwrap();

        let mut changed = bound.clone();
        changed.model = "other".into();
        write_resources(temp.path(), &changed);
        let error = observer.authorize_launch().unwrap_err();
        assert!(error.downcast_ref::<LaunchRefused>().is_some(), "{error:#}");

        write_resources(temp.path(), &bound);
        Database::open(&path)
            .unwrap()
            .record_funding_refusal(
                &bound.funding_key(),
                bound.authorization_revision,
                "fixture",
            )
            .unwrap();
        let other = LaunchObserver::new(
            &path,
            "run",
            "second",
            Some(LaunchGuard {
                state_root: temp.path().to_owned(),
                profile: bound,
            }),
            None,
        );
        let error = other.authorize_launch().unwrap_err();
        assert!(
            error.to_string().contains("funding was refused"),
            "{error:#}"
        );
        assert!(
            launches(&path)
                .iter()
                .all(|launch| launch.attempt_id == "attempt"),
            "a refused launch records no intent"
        );
    }
}
