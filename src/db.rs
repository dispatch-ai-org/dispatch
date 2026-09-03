use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::models::{
    BenchmarkPrior, CandidateRecord, CandidateStatus, CheckResult, EvaluationOutcome,
    EvaluationRecord, EventRecord, RoutingHumanEvaluation, RoutingObservation, RunRecord,
    TaskFeatures,
};

const MIGRATIONS: &[(i64, &str, &str)] = &[
    (
        1,
        "initial_schema",
        r#"
CREATE TABLE sources (
    id                  INTEGER PRIMARY KEY,
    path                TEXT NOT NULL,
    kind                TEXT NOT NULL,
    git_head            TEXT,
    fingerprint         TEXT NOT NULL,
    created_at          TEXT NOT NULL
);

CREATE TABLE runs (
    id                  TEXT PRIMARY KEY,
    source_id           INTEGER NOT NULL REFERENCES sources(id),
    task                TEXT NOT NULL,
    exact_prompt        TEXT NOT NULL,
    baseline_path       TEXT NOT NULL,
    baseline_commit     TEXT NOT NULL,
    status              TEXT NOT NULL,
    created_at          TEXT NOT NULL,
    completed_at        TEXT,
    dispatch_version    TEXT NOT NULL,
    os                  TEXT NOT NULL,
    architecture        TEXT NOT NULL,
    execution_backend   TEXT NOT NULL,
    timeout_secs        INTEGER NOT NULL,
    cpus                REAL NOT NULL,
    memory              TEXT NOT NULL,
    max_parallel        INTEGER NOT NULL,
    applied_candidate   TEXT
);

CREATE TABLE candidates (
    id                  TEXT PRIMARY KEY,
    run_id              TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    label               TEXT NOT NULL,
    harness_id          TEXT NOT NULL,
    harness_version     TEXT,
    model               TEXT,
    status              TEXT NOT NULL,
    workspace_path      TEXT NOT NULL,
    prompt_path         TEXT NOT NULL,
    stdout_path         TEXT NOT NULL,
    stderr_path         TEXT NOT NULL,
    diff_path           TEXT NOT NULL,
    duration_ms         INTEGER NOT NULL,
    exit_code           INTEGER,
    timed_out           INTEGER NOT NULL,
    tokens              INTEGER,
    cost_usd            REAL,
    error               TEXT,
    files_changed       INTEGER NOT NULL,
    lines_added         INTEGER NOT NULL,
    lines_removed       INTEGER NOT NULL,
    untracked_files_json TEXT NOT NULL,
    UNIQUE (run_id, label),
    UNIQUE (run_id, id)
);

CREATE TABLE checks (
    id                  INTEGER PRIMARY KEY,
    run_id              TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    candidate_id        TEXT REFERENCES candidates(id) ON DELETE CASCADE,
    name                TEXT NOT NULL,
    phase               TEXT NOT NULL,
    command             TEXT NOT NULL,
    status              TEXT NOT NULL,
    exit_code           INTEGER,
    duration_ms         INTEGER NOT NULL,
    stdout_path         TEXT NOT NULL,
    stderr_path         TEXT NOT NULL
);

CREATE INDEX checks_run_idx ON checks(run_id);
CREATE INDEX checks_candidate_idx ON checks(candidate_id);

CREATE TABLE artifacts (
    id                  INTEGER PRIMARY KEY,
    run_id              TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    candidate_id        TEXT REFERENCES candidates(id) ON DELETE CASCADE,
    kind                TEXT NOT NULL,
    name                TEXT,
    path                TEXT NOT NULL
);

CREATE INDEX artifacts_run_idx ON artifacts(run_id);
CREATE INDEX artifacts_candidate_idx ON artifacts(candidate_id);

CREATE TABLE events (
    id                  INTEGER PRIMARY KEY,
    run_id              TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    candidate_label     TEXT,
    event_type          TEXT NOT NULL,
    timestamp           TEXT NOT NULL,
    payload_json        TEXT NOT NULL
);

CREATE INDEX events_run_timestamp_idx ON events(run_id, timestamp);

CREATE TABLE evaluations (
    id                  INTEGER PRIMARY KEY,
    run_id              TEXT NOT NULL UNIQUE REFERENCES runs(id) ON DELETE CASCADE,
    outcome             TEXT NOT NULL,
    selected_candidate  TEXT,
    selected_candidate_id TEXT,
    explanation         TEXT,
    created_at          TEXT NOT NULL,
    blind               INTEGER NOT NULL,
    FOREIGN KEY (run_id, selected_candidate_id)
        REFERENCES candidates(run_id, id),
    CHECK (
        (outcome = 'candidate' AND selected_candidate IS NOT NULL AND selected_candidate_id IS NOT NULL)
        OR (outcome IN ('tie', 'neither') AND selected_candidate IS NULL AND selected_candidate_id IS NULL)
    )
);

CREATE TABLE evaluation_reasons (
    evaluation_id       INTEGER NOT NULL REFERENCES evaluations(id) ON DELETE CASCADE,
    position            INTEGER NOT NULL,
    reason              TEXT NOT NULL,
    PRIMARY KEY (evaluation_id, position)
);
"#,
    ),
    (
        2,
        "environment_provenance",
        r#"
ALTER TABLE runs ADD COLUMN docker_image TEXT;
ALTER TABLE runs ADD COLUMN resource_limits_enforced INTEGER NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN unsafe_local INTEGER NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN forwarded_env_json TEXT NOT NULL DEFAULT '[]';
"#,
    ),
    (
        3,
        "token_semantics",
        "ALTER TABLE candidates ADD COLUMN token_semantics TEXT;",
    ),
    (
        4,
        "evaluation_sync",
        r#"
ALTER TABLE evaluations ADD COLUMN public_id TEXT;
UPDATE evaluations SET public_id = 'evaluation-' || run_id WHERE public_id IS NULL;
CREATE UNIQUE INDEX evaluations_public_id_idx ON evaluations(public_id);

CREATE TABLE sync_settings (
    id                  INTEGER PRIMARY KEY CHECK (id = 1),
    enabled             INTEGER NOT NULL DEFAULT 0,
    contributor_id      TEXT,
    enabled_at          TEXT
);
INSERT INTO sync_settings(id, enabled) VALUES (1, 0);

CREATE TABLE sync_outbox (
    evaluation_id       TEXT PRIMARY KEY,
    run_id              TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    payload_json        TEXT NOT NULL,
    status              TEXT NOT NULL CHECK (status IN ('pending', 'failed', 'synced')),
    last_attempt        TEXT,
    last_error          TEXT,
    synced_at           TEXT
);
CREATE INDEX sync_outbox_status_idx ON sync_outbox(status);
"#,
    ),
    (
        5,
        "cloud_ingestion_token",
        "ALTER TABLE sync_settings ADD COLUMN ingestion_token TEXT;",
    ),
    (
        6,
        "benchmark_priors",
        r#"
CREATE TABLE benchmark_priors (
    id                  INTEGER PRIMARY KEY,
    source              TEXT NOT NULL,
    dataset             TEXT NOT NULL,
    dataset_version     TEXT NOT NULL,
    harness             TEXT NOT NULL,
    model               TEXT,
    language            TEXT,
    task_kind           TEXT NOT NULL CHECK (task_kind IN ('bug_fix', 'feature', 'refactor', 'tests', 'unknown')),
    scope               TEXT NOT NULL CHECK (scope IN ('localized', 'multi_file', 'broad', 'unknown')),
    successes           INTEGER NOT NULL CHECK (successes >= 0),
    attempts            INTEGER NOT NULL CHECK (attempts >= 0 AND successes <= attempts),
    updated_at          TEXT NOT NULL
);

CREATE UNIQUE INDEX benchmark_priors_identity_idx ON benchmark_priors(
    source,
    dataset,
    dataset_version,
    harness,
    COALESCE(model, X''),
    COALESCE(language, X''),
    task_kind,
    scope
);
CREATE INDEX benchmark_priors_lookup_idx
    ON benchmark_priors(harness, language, task_kind, scope);
"#,
    ),
    (
        7,
        "run_routing_decision",
        "ALTER TABLE runs ADD COLUMN routing_decision_json TEXT;",
    ),
    (
        8,
        "routing_observations",
        r#"
CREATE TABLE routing_observations (
    id                  TEXT PRIMARY KEY,
    run_id              TEXT NOT NULL UNIQUE REFERENCES runs(id) ON DELETE CASCADE,
    candidate_id        TEXT NOT NULL,
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL,
    observation_json    TEXT NOT NULL
);
"#,
    ),
    (
        9,
        "routing_observation_sync",
        r#"
ALTER TABLE sync_settings ADD COLUMN consent_version INTEGER NOT NULL DEFAULT 1
    CHECK (consent_version >= 1);
ALTER TABLE sync_settings ADD COLUMN routing_observation_enabled_at TEXT;

CREATE TABLE sync_outbox_v2 (
    record_id           TEXT PRIMARY KEY,
    run_id              TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    record_type         TEXT NOT NULL CHECK (record_type IN ('evaluation-v1', 'routing-observation-v1')),
    payload_json        TEXT NOT NULL,
    status              TEXT NOT NULL CHECK (status IN ('pending', 'failed', 'conflict', 'synced')),
    last_attempt        TEXT,
    last_error          TEXT,
    synced_at           TEXT
);
INSERT INTO sync_outbox_v2(
    record_id, run_id, record_type, payload_json, status, last_attempt, last_error, synced_at
)
SELECT evaluation_id, run_id, 'evaluation-v1', payload_json, status, last_attempt, last_error, synced_at
FROM sync_outbox;
DROP TABLE sync_outbox;
ALTER TABLE sync_outbox_v2 RENAME TO sync_outbox;
CREATE INDEX sync_outbox_status_idx ON sync_outbox(status);
CREATE INDEX sync_outbox_type_status_idx ON sync_outbox(record_type, status);
"#,
    ),
];

/// The compact row used by `dispatch history`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSummary {
    pub id: String,
    pub task: String,
    pub source_path: PathBuf,
    pub status: String,
    pub created_at: String,
    pub candidate_count: usize,
    pub evaluated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncSettings {
    pub enabled: bool,
    pub contributor_id: Option<String>,
    pub enabled_at: Option<String>,
    pub consent_version: u32,
    pub routing_observation_enabled_at: Option<String>,
}

impl SyncSettings {
    pub fn routing_observations_enabled(&self) -> bool {
        self.enabled
            && self.consent_version >= ROUTING_OBSERVATION_CONSENT_VERSION
            && self.routing_observation_enabled_at.is_some()
    }
}

pub const ROUTING_OBSERVATION_CONSENT_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncRecordType {
    EvaluationV1,
    RoutingObservationV1,
}

impl SyncRecordType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EvaluationV1 => "evaluation-v1",
            Self::RoutingObservationV1 => "routing-observation-v1",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "evaluation-v1" => Ok(Self::EvaluationV1),
            "routing-observation-v1" => Ok(Self::RoutingObservationV1),
            _ => anyhow::bail!("unknown sync record type {value:?}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncOutboxRecord {
    pub record_id: String,
    pub run_id: String,
    pub record_type: SyncRecordType,
    pub payload_json: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncOutboxCounts {
    pub pending: usize,
    pub failed: usize,
    pub conflict: usize,
    pub synced: usize,
}

/// Dispatch's durable structured store. Large artifacts remain on disk and are
/// referenced by rows in the `artifacts` table.
pub struct Database {
    connection: Connection,
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create database directory {}", parent.display())
            })?;
        }

        let connection = Connection::open(path)
            .with_context(|| format!("failed to open database {}", path.display()))?;
        let mut database = Self { connection };
        database.configure(false)?;
        database.migrate()?;
        Ok(database)
    }

    pub fn open_in_memory() -> Result<Self> {
        let connection =
            Connection::open_in_memory().context("failed to open in-memory database")?;
        let mut database = Self { connection };
        database.configure(true)?;
        database.migrate()?;
        Ok(database)
    }

    fn configure(&self, in_memory: bool) -> Result<()> {
        self.connection
            .execute_batch("PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")
            .context("failed to configure SQLite")?;
        self.connection
            .busy_timeout(Duration::from_secs(5))
            .context("failed to configure SQLite busy timeout")?;
        if !in_memory {
            // WAL lets read-only history/show commands coexist with an active run.
            self.connection
                .pragma_update(None, "journal_mode", "WAL")
                .context("failed to enable SQLite WAL mode")?;
        }
        Ok(())
    }

    fn migrate(&mut self) -> Result<()> {
        self.connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS schema_migrations (\
                    version INTEGER PRIMARY KEY, \
                    name TEXT NOT NULL, \
                    applied_at TEXT NOT NULL\
                );",
            )
            .context("failed to initialize schema migrations")?;

        for (version, name, sql) in MIGRATIONS {
            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let applied = transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = ?1)",
                    [version],
                    |row| row.get::<_, bool>(0),
                )
                .context("failed to read schema migration state")?;
            if applied {
                transaction.commit()?;
                continue;
            }

            transaction.execute_batch(sql).with_context(|| {
                format!("failed to apply database migration {version} ({name})")
            })?;
            transaction.execute(
                "INSERT INTO schema_migrations(version, name, applied_at) VALUES (?1, ?2, ?3)",
                params![version, name, timestamp(Utc::now())],
            )?;
            transaction.commit()?;
        }
        Ok(())
    }

    pub fn upsert_benchmark_prior(&self, prior: &BenchmarkPrior) -> Result<()> {
        insert_benchmark_prior(&self.connection, prior)
    }

    pub fn replace_benchmark_prior(&mut self, prior: &BenchmarkPrior) -> Result<()> {
        let transaction = self.connection.transaction()?;
        transaction.execute(
            r#"DELETE FROM benchmark_priors
               WHERE source = ?1
                 AND dataset = ?2
                 AND harness = ?3
                 AND model IS ?4
                 AND language IS ?5
                 AND task_kind = ?6
                 AND scope = ?7"#,
            params![
                prior.source,
                prior.dataset,
                prior.harness,
                prior.model,
                prior.language,
                prior.task_kind.as_str(),
                prior.scope.as_str(),
            ],
        )?;
        insert_benchmark_prior(&transaction, prior)?;
        transaction
            .commit()
            .context("failed to replace benchmark prior")
    }

    pub fn matching_benchmark_priors(
        &self,
        features: &TaskFeatures,
        harness: &str,
    ) -> Result<Vec<BenchmarkPrior>> {
        let mut statement = self.connection.prepare(
            r#"SELECT source, dataset, dataset_version, harness, model, language,
                      successes, attempts, updated_at
               FROM benchmark_priors
               WHERE harness = ?1
                 AND language IS ?2
                 AND task_kind = ?3
                 AND scope = ?4
               ORDER BY source, dataset, dataset_version, model"#,
        )?;
        let rows = statement.query_map(
            params![
                harness,
                features.language,
                features.task_kind.as_str(),
                features.scope.as_str(),
            ],
            |row| {
                let updated_at = timestamp_from_sql(8, row.get(8)?)?;
                Ok(BenchmarkPrior {
                    source: row.get(0)?,
                    dataset: row.get(1)?,
                    dataset_version: row.get(2)?,
                    harness: row.get(3)?,
                    model: row.get(4)?,
                    language: row.get(5)?,
                    task_kind: features.task_kind.clone(),
                    scope: features.scope.clone(),
                    successes: row.get(6)?,
                    attempts: row.get(7)?,
                    updated_at,
                })
            },
        )?;
        rows.collect::<rusqlite::Result<_>>()
            .context("failed to read matching benchmark priors")
    }

    /// Replace all structured state for a run in one transaction. Events are an
    /// append-only log and are deliberately not replaced by this operation.
    pub fn sync_run(&mut self, run: &RunRecord) -> Result<()> {
        let routing_decision_json = run
            .routing
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("failed to serialize routing decision")?;
        let transaction = self.connection.transaction()?;

        let existing_source_id = transaction
            .query_row(
                "SELECT source_id FROM runs WHERE id = ?1",
                [&run.id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let source_id = match existing_source_id {
            Some(source_id) => {
                transaction.execute(
                    "UPDATE sources SET path = ?1, kind = ?2, git_head = ?3, fingerprint = ?4 \
                     WHERE id = ?5",
                    params![
                        path_text(&run.source_path),
                        run.source_kind.as_str(),
                        run.source_git_head,
                        run.source_fingerprint,
                        source_id,
                    ],
                )?;
                source_id
            }
            None => {
                transaction.execute(
                    "INSERT INTO sources(path, kind, git_head, fingerprint, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        path_text(&run.source_path),
                        run.source_kind.as_str(),
                        run.source_git_head,
                        run.source_fingerprint,
                        timestamp(run.created_at),
                    ],
                )?;
                transaction.last_insert_rowid()
            }
        };

        transaction.execute(
            r#"INSERT INTO runs(
                    id, source_id, task, exact_prompt, baseline_path, baseline_commit, status,
                    created_at, completed_at, dispatch_version, os, architecture,
                    execution_backend, timeout_secs, cpus, memory, max_parallel,
                    docker_image, resource_limits_enforced, unsafe_local,
                    forwarded_env_json, applied_candidate, routing_decision_json
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                    ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23
                )
                ON CONFLICT(id) DO UPDATE SET
                    source_id = excluded.source_id,
                    task = excluded.task,
                    exact_prompt = excluded.exact_prompt,
                    baseline_path = excluded.baseline_path,
                    baseline_commit = excluded.baseline_commit,
                    status = excluded.status,
                    created_at = excluded.created_at,
                    completed_at = excluded.completed_at,
                    dispatch_version = excluded.dispatch_version,
                    os = excluded.os,
                    architecture = excluded.architecture,
                    execution_backend = excluded.execution_backend,
                    timeout_secs = excluded.timeout_secs,
                    cpus = excluded.cpus,
                    memory = excluded.memory,
                    max_parallel = excluded.max_parallel,
                    docker_image = excluded.docker_image,
                    resource_limits_enforced = excluded.resource_limits_enforced,
                    unsafe_local = excluded.unsafe_local,
                    forwarded_env_json = excluded.forwarded_env_json,
                    applied_candidate = excluded.applied_candidate,
                    routing_decision_json = excluded.routing_decision_json"#,
            params![
                run.id,
                source_id,
                run.task,
                run.exact_prompt,
                path_text(&run.baseline_path),
                run.baseline_commit,
                run.status.as_str(),
                timestamp(run.created_at),
                run.completed_at.map(timestamp),
                run.environment.dispatch_version,
                run.environment.os,
                run.environment.architecture,
                run.environment.execution_backend,
                unsigned(run.environment.timeout_secs, "environment timeout")?,
                run.environment.cpus,
                run.environment.memory,
                usize_integer(run.environment.max_parallel, "environment max_parallel")?,
                run.environment.docker_image,
                run.environment.resource_limits_enforced,
                run.environment.unsafe_local,
                serde_json::to_string(&run.environment.forwarded_env)
                    .context("failed to serialize forwarded environment names")?,
                run.applied_candidate,
                routing_decision_json,
            ],
        )?;

        // Evaluations reference candidates, so remove the old evaluation before
        // replacing a run's candidate set. Reasons cascade from the evaluation.
        transaction.execute("DELETE FROM evaluations WHERE run_id = ?1", [&run.id])?;
        transaction.execute("DELETE FROM checks WHERE run_id = ?1", [&run.id])?;
        transaction.execute("DELETE FROM artifacts WHERE run_id = ?1", [&run.id])?;
        transaction.execute("DELETE FROM candidates WHERE run_id = ?1", [&run.id])?;

        for candidate in &run.candidates {
            insert_candidate(&transaction, &run.id, candidate)?;
            insert_candidate_artifacts(&transaction, &run.id, candidate)?;
            for check in &candidate.checks {
                insert_check(&transaction, &run.id, Some(&candidate.id), check)?;
                insert_check_artifacts(&transaction, &run.id, Some(&candidate.id), check)?;
            }
        }

        insert_artifact(
            &transaction,
            &run.id,
            None,
            "baseline",
            None,
            &run.baseline_path,
        )?;
        if let Some(run_dir) = run.baseline_path.parent() {
            for (kind, name) in [
                ("task", "task.md"),
                ("config", "config.snapshot.yml"),
                ("metadata", "metadata.json"),
                ("events", "events.jsonl"),
            ] {
                insert_artifact(&transaction, &run.id, None, kind, None, &run_dir.join(name))?;
            }
        }
        for check in &run.baseline_checks {
            insert_check(&transaction, &run.id, None, check)?;
            insert_check_artifacts(&transaction, &run.id, None, check)?;
        }

        if let Some(evaluation) = &run.evaluation {
            insert_evaluation(&transaction, &run.id, evaluation, false)?;
        }
        sync_routing_observation(&transaction, run)?;

        transaction.commit().context("failed to commit run sync")
    }

    pub fn record_event(&self, event: &EventRecord) -> Result<()> {
        let payload =
            serde_json::to_string(&event.payload).context("failed to serialize event payload")?;
        self.connection
            .execute(
                "INSERT INTO events(run_id, candidate_label, event_type, timestamp, payload_json) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    event.run_id,
                    event.candidate_label,
                    event.event_type,
                    timestamp(event.timestamp),
                    payload,
                ],
            )
            .with_context(|| format!("failed to record {} event", event.event_type))?;
        Ok(())
    }

    pub fn save_evaluation(&mut self, run_id: &str, evaluation: &EvaluationRecord) -> Result<()> {
        let transaction = self.connection.transaction()?;
        insert_evaluation(&transaction, run_id, evaluation, true)?;
        transaction.commit().context("failed to commit evaluation")
    }

    pub fn routing_observation(&self, id: &str) -> Result<Option<RoutingObservation>> {
        self.read_routing_observation(
            "SELECT observation_json FROM routing_observations WHERE id = ?1",
            id,
        )
    }

    pub fn routing_observation_for_run(&self, run_id: &str) -> Result<Option<RoutingObservation>> {
        self.read_routing_observation(
            "SELECT observation_json FROM routing_observations WHERE run_id = ?1",
            run_id,
        )
    }

    pub fn routing_observations(&self) -> Result<Vec<RoutingObservation>> {
        let mut statement = self
            .connection
            .prepare("SELECT observation_json FROM routing_observations ORDER BY id")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|row| deserialize_routing_observation(row?))
            .collect()
    }

    pub fn save_routing_human_evaluation(
        &mut self,
        run_id: &str,
        evaluation: &RoutingHumanEvaluation,
    ) -> Result<RoutingObservation> {
        let transaction = self.connection.transaction()?;
        let json = transaction
            .query_row(
                "SELECT observation_json FROM routing_observations WHERE run_id = ?1",
                [run_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .with_context(|| format!("run {run_id} has no routing observation"))?;
        let mut observation = deserialize_routing_observation(json)?;
        if observation
            .human_evaluation
            .as_ref()
            .is_some_and(|existing| same_routing_human_evaluation(existing, evaluation))
        {
            transaction.commit()?;
            return Ok(observation);
        }

        observation.updated_at = evaluation.evaluated_at;
        observation.human_evaluation = Some(evaluation.clone());
        let json = serde_json::to_string(&observation)
            .context("failed to serialize routing observation")?;
        transaction.execute(
            "UPDATE routing_observations SET updated_at = ?1, observation_json = ?2 \
             WHERE run_id = ?3",
            params![timestamp(observation.updated_at), json, run_id],
        )?;
        transaction
            .commit()
            .context("failed to commit routing evaluation")?;
        Ok(observation)
    }

    fn read_routing_observation(
        &self,
        query: &str,
        identity: &str,
    ) -> Result<Option<RoutingObservation>> {
        self.connection
            .query_row(query, [identity], |row| row.get::<_, String>(0))
            .optional()?
            .map(deserialize_routing_observation)
            .transpose()
    }

    pub fn sync_settings(&self) -> Result<SyncSettings> {
        let (enabled, contributor_id, enabled_at, consent_version, routing_enabled_at) = self
            .connection
            .query_row(
                "SELECT enabled, contributor_id, enabled_at, consent_version, \
                 routing_observation_enabled_at FROM sync_settings WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get::<_, i64>(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .context("failed to read sync settings")?;
        Ok(SyncSettings {
            enabled,
            contributor_id,
            enabled_at,
            consent_version: u32::try_from(consent_version)
                .context("sync consent version is invalid")?,
            routing_observation_enabled_at: routing_enabled_at,
        })
    }

    pub fn enable_sync(
        &mut self,
        contributor_id: &str,
        enabled_at: DateTime<Utc>,
    ) -> Result<SyncSettings> {
        self.connection.execute(
            "UPDATE sync_settings SET enabled = 1, \
             contributor_id = COALESCE(contributor_id, ?1), \
             enabled_at = COALESCE(enabled_at, ?2), \
             consent_version = MAX(consent_version, ?3), \
             routing_observation_enabled_at = COALESCE(routing_observation_enabled_at, ?2) \
             WHERE id = 1",
            params![
                contributor_id,
                timestamp(enabled_at),
                ROUTING_OBSERVATION_CONSENT_VERSION
            ],
        )?;
        self.sync_settings()
    }

    pub fn disable_sync(&self) -> Result<()> {
        self.connection
            .execute("UPDATE sync_settings SET enabled = 0 WHERE id = 1", [])?;
        Ok(())
    }

    pub fn set_sync_token(&self, token: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE sync_settings SET ingestion_token = ?1 WHERE id = 1",
            [token],
        )?;
        Ok(())
    }

    pub fn sync_token(&self) -> Result<Option<String>> {
        self.connection
            .query_row(
                "SELECT ingestion_token FROM sync_settings WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .context("failed to read Dispatch Cloud ingestion token")
    }

    pub fn clear_sync_token(&self) -> Result<()> {
        self.connection.execute(
            "UPDATE sync_settings SET ingestion_token = NULL WHERE id = 1",
            [],
        )?;
        Ok(())
    }

    pub fn evaluation_id(&self, run_id: &str) -> Result<Option<String>> {
        self.connection
            .query_row(
                "SELECT public_id FROM evaluations WHERE run_id = ?1",
                [run_id],
                |row| row.get(0),
            )
            .optional()
            .context("failed to read evaluation identity")
    }

    pub fn evaluation_count(&self) -> Result<usize> {
        let count: i64 =
            self.connection
                .query_row("SELECT COUNT(*) FROM evaluations", [], |row| row.get(0))?;
        usize::try_from(count).context("evaluation count is invalid")
    }

    pub fn enqueue_sync(
        &self,
        record_type: SyncRecordType,
        record_id: &str,
        run_id: &str,
        payload_json: &str,
    ) -> Result<bool> {
        let source_exists = match record_type {
            SyncRecordType::EvaluationV1 => self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM evaluations WHERE public_id = ?1 AND run_id = ?2)",
                params![record_id, run_id],
                |row| row.get::<_, bool>(0),
            )?,
            SyncRecordType::RoutingObservationV1 => self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM routing_observations WHERE id = ?1 AND run_id = ?2)",
                params![record_id, run_id],
                |row| row.get::<_, bool>(0),
            )?,
        };
        anyhow::ensure!(
            source_exists,
            "run {run_id} has no persisted {} record {record_id}",
            record_type.as_str()
        );

        let transaction = self.connection.unchecked_transaction()?;
        let existing = transaction
            .query_row(
                "SELECT record_type, payload_json, status FROM sync_outbox WHERE record_id = ?1",
                [record_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        let changed = if let Some((stored_type, stored_payload, status)) = existing {
            anyhow::ensure!(
                stored_type == record_type.as_str(),
                "sync record {record_id} already has type {stored_type}"
            );
            if stored_payload == payload_json {
                false
            } else if record_type == SyncRecordType::EvaluationV1 {
                // Preserve the original evaluation outbox's first-snapshot behavior.
                false
            } else {
                anyhow::ensure!(
                    !matches!(status.as_str(), "synced" | "conflict"),
                    "{} {record_id} is already {status}; its Cloud payload is immutable",
                    record_type.as_str()
                );
                transaction.execute(
                    "UPDATE sync_outbox SET payload_json = ?2, status = 'pending', \
                     last_attempt = NULL, last_error = NULL, synced_at = NULL \
                     WHERE record_id = ?1",
                    params![record_id, payload_json],
                )?;
                true
            }
        } else {
            transaction.execute(
                "INSERT INTO sync_outbox(\
                     record_id, run_id, record_type, payload_json, status\
                 ) VALUES (?1, ?2, ?3, ?4, 'pending')",
                params![record_id, run_id, record_type.as_str(), payload_json],
            )?;
            true
        };
        transaction.commit()?;
        Ok(changed)
    }

    pub fn sync_payload_for_run(
        &self,
        run_id: &str,
        record_type: SyncRecordType,
    ) -> Result<Option<String>> {
        self.connection
            .query_row(
                "SELECT payload_json FROM sync_outbox WHERE run_id = ?1 AND record_type = ?2",
                params![run_id, record_type.as_str()],
                |row| row.get(0),
            )
            .optional()
            .context("failed to read sync preview")
    }

    pub fn pending_sync_records(&self) -> Result<Vec<SyncOutboxRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT record_id, run_id, record_type, payload_json FROM sync_outbox \
             WHERE status IN ('pending', 'failed') ORDER BY record_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let rows = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read sync outbox")?;
        rows.into_iter()
            .map(|(record_id, run_id, record_type, payload_json)| {
                Ok(SyncOutboxRecord {
                    record_id,
                    run_id,
                    record_type: SyncRecordType::parse(&record_type)?,
                    payload_json,
                })
            })
            .collect()
    }

    pub fn mark_sync_failed(
        &self,
        record_id: &str,
        attempted_at: DateTime<Utc>,
        error: &str,
    ) -> Result<()> {
        self.connection.execute(
            "UPDATE sync_outbox SET status = 'failed', last_attempt = ?2, \
             last_error = ?3, synced_at = NULL WHERE record_id = ?1",
            params![record_id, timestamp(attempted_at), error],
        )?;
        Ok(())
    }

    pub fn mark_sync_conflict(
        &self,
        record_id: &str,
        attempted_at: DateTime<Utc>,
        error: &str,
    ) -> Result<()> {
        self.connection.execute(
            "UPDATE sync_outbox SET status = 'conflict', last_attempt = ?2, \
             last_error = ?3, synced_at = NULL WHERE record_id = ?1",
            params![record_id, timestamp(attempted_at), error],
        )?;
        Ok(())
    }

    pub fn mark_sync_succeeded(&self, record_id: &str, attempted_at: DateTime<Utc>) -> Result<()> {
        let at = timestamp(attempted_at);
        self.connection.execute(
            "UPDATE sync_outbox SET status = 'synced', last_attempt = ?2, \
             last_error = NULL, synced_at = ?2 WHERE record_id = ?1",
            params![record_id, at],
        )?;
        Ok(())
    }

    pub fn sync_outbox_counts(&self) -> Result<SyncOutboxCounts> {
        self.read_sync_outbox_counts(None)
    }

    pub fn sync_outbox_counts_for_type(
        &self,
        record_type: SyncRecordType,
    ) -> Result<SyncOutboxCounts> {
        self.read_sync_outbox_counts(Some(record_type))
    }

    fn read_sync_outbox_counts(
        &self,
        record_type: Option<SyncRecordType>,
    ) -> Result<SyncOutboxCounts> {
        let mut counts = SyncOutboxCounts::default();
        let mut statement = self.connection.prepare(
            "SELECT status, COUNT(*) FROM sync_outbox \
             WHERE ?1 IS NULL OR record_type = ?1 GROUP BY status",
        )?;
        let record_type = record_type.map(SyncRecordType::as_str);
        let rows = statement.query_map([record_type], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (status, count) = row?;
            let count = usize::try_from(count).context("sync outbox count is invalid")?;
            match status.as_str() {
                "pending" => counts.pending = count,
                "failed" => counts.failed = count,
                "conflict" => counts.conflict = count,
                "synced" => counts.synced = count,
                _ => {}
            }
        }
        Ok(counts)
    }

    pub fn list_runs(&self, limit: usize) -> Result<Vec<RunSummary>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = usize_integer(limit, "history limit")?;
        let mut statement = self.connection.prepare(
            r#"SELECT r.id, r.task, s.path, r.status, r.created_at,
                      COUNT(c.id), e.id IS NOT NULL
               FROM runs r
               JOIN sources s ON s.id = r.source_id
               LEFT JOIN candidates c ON c.run_id = r.id
               LEFT JOIN evaluations e ON e.run_id = r.id
               GROUP BY r.id
               ORDER BY r.created_at DESC, r.id DESC
               LIMIT ?1"#,
        )?;
        let rows = statement.query_map([limit], |row| {
            let count: i64 = row.get(5)?;
            Ok(RunSummary {
                id: row.get(0)?,
                task: row.get(1)?,
                source_path: PathBuf::from(row.get::<_, String>(2)?),
                status: row.get(3)?,
                created_at: row.get(4)?,
                candidate_count: usize::try_from(count).unwrap_or(usize::MAX),
                evaluated: row.get(6)?,
            })
        })?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to list runs")
    }

    pub fn health_check(&self) -> Result<()> {
        self.connection
            .query_row("SELECT 1", [], |_| Ok(()))
            .context("SQLite health check failed")
    }

    pub fn schema_version(&self) -> Result<i64> {
        self.connection
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                [],
                |row| row.get(0),
            )
            .context("failed to read SQLite schema version")
    }
}

fn insert_candidate(
    transaction: &Transaction<'_>,
    run_id: &str,
    candidate: &CandidateRecord,
) -> Result<()> {
    let untracked_files = serde_json::to_string(&candidate.diff_stats.untracked_files)
        .context("failed to serialize untracked file list")?;
    transaction.execute(
        r#"INSERT INTO candidates(
                id, run_id, label, harness_id, harness_version, model, status,
                workspace_path, prompt_path, stdout_path, stderr_path, diff_path,
                duration_ms, exit_code, timed_out, tokens, token_semantics, cost_usd, error,
                files_changed, lines_added, lines_removed, untracked_files_json
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23
            )"#,
        params![
            candidate.id,
            run_id,
            candidate.label,
            candidate.harness_id,
            candidate.harness_version,
            candidate.model,
            candidate.status.as_str(),
            path_text(&candidate.workspace_path),
            path_text(&candidate.prompt_path),
            path_text(&candidate.stdout_path),
            path_text(&candidate.stderr_path),
            path_text(&candidate.diff_path),
            unsigned(candidate.duration_ms, "candidate duration")?,
            candidate.exit_code,
            candidate.timed_out,
            candidate
                .tokens
                .map(|value| unsigned(value, "candidate tokens"))
                .transpose()?,
            candidate.token_semantics,
            candidate.cost_usd,
            candidate.error,
            unsigned(candidate.diff_stats.files_changed, "changed file count")?,
            unsigned(candidate.diff_stats.lines_added, "added line count")?,
            unsigned(candidate.diff_stats.lines_removed, "removed line count")?,
            untracked_files,
        ],
    )?;
    Ok(())
}

fn insert_check(
    transaction: &Transaction<'_>,
    run_id: &str,
    candidate_id: Option<&str>,
    check: &CheckResult,
) -> Result<()> {
    let phase = serde_json::to_value(&check.phase)?
        .as_str()
        .context("check phase did not serialize as a string")?
        .to_owned();
    let status = serde_json::to_value(&check.status)?
        .as_str()
        .context("check status did not serialize as a string")?
        .to_owned();
    transaction.execute(
        r#"INSERT INTO checks(
                run_id, candidate_id, name, phase, command, status, exit_code,
                duration_ms, stdout_path, stderr_path
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"#,
        params![
            run_id,
            candidate_id,
            check.name,
            phase,
            check.command,
            status,
            check.exit_code,
            unsigned(check.duration_ms, "check duration")?,
            path_text(&check.stdout_path),
            path_text(&check.stderr_path),
        ],
    )?;
    Ok(())
}

fn insert_candidate_artifacts(
    transaction: &Transaction<'_>,
    run_id: &str,
    candidate: &CandidateRecord,
) -> Result<()> {
    for (kind, path) in [
        ("workspace", &candidate.workspace_path),
        ("prompt", &candidate.prompt_path),
        ("stdout", &candidate.stdout_path),
        ("stderr", &candidate.stderr_path),
        ("diff", &candidate.diff_path),
    ] {
        insert_artifact(transaction, run_id, Some(&candidate.id), kind, None, path)?;
    }
    if let Some(candidate_dir) = candidate.prompt_path.parent() {
        insert_artifact(
            transaction,
            run_id,
            Some(&candidate.id),
            "harness_events",
            None,
            &candidate_dir.join("harness.jsonl"),
        )?;
    }
    Ok(())
}

fn insert_check_artifacts(
    transaction: &Transaction<'_>,
    run_id: &str,
    candidate_id: Option<&str>,
    check: &CheckResult,
) -> Result<()> {
    insert_artifact(
        transaction,
        run_id,
        candidate_id,
        "check_stdout",
        Some(&check.name),
        &check.stdout_path,
    )?;
    insert_artifact(
        transaction,
        run_id,
        candidate_id,
        "check_stderr",
        Some(&check.name),
        &check.stderr_path,
    )
}

fn insert_artifact(
    transaction: &Transaction<'_>,
    run_id: &str,
    candidate_id: Option<&str>,
    kind: &str,
    name: Option<&str>,
    path: &Path,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO artifacts(run_id, candidate_id, kind, name, path) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![run_id, candidate_id, kind, name, path_text(path)],
    )?;
    Ok(())
}

fn sync_routing_observation(transaction: &Transaction<'_>, run: &RunRecord) -> Result<()> {
    let Some(prediction) = &run.routing else {
        return Ok(());
    };
    let [candidate] = run.candidates.as_slice() else {
        return Ok(());
    };
    if matches!(
        candidate.status,
        CandidateStatus::Preparing | CandidateStatus::Running | CandidateStatus::Verifying
    ) {
        return Ok(());
    }
    anyhow::ensure!(
        candidate.harness_id == prediction.selected_harness,
        "routed run {} selected {} but recorded candidate {}",
        run.id,
        prediction.selected_harness,
        candidate.harness_id
    );

    let id = format!("routing-observation-{}", run.id);
    let now = Utc::now();
    let existing = transaction
        .query_row(
            "SELECT observation_json FROM routing_observations WHERE run_id = ?1",
            [&run.id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(deserialize_routing_observation)
        .transpose()?;
    let created_at = existing
        .as_ref()
        .map_or(now, |observation| observation.created_at);
    let human_evaluation = existing.and_then(|observation| observation.human_evaluation);
    let observation = RoutingObservation {
        id: id.clone(),
        run_id: run.id.clone(),
        candidate_id: candidate.id.clone(),
        created_at,
        updated_at: now,
        prediction: prediction.clone(),
        harness_version: candidate.harness_version.clone(),
        model: candidate.model.clone(),
        candidate_status: candidate.status.clone(),
        exit_code: candidate.exit_code,
        timed_out: candidate.timed_out,
        verification: (!candidate.checks.is_empty()).then(|| {
            candidate
                .checks
                .iter()
                .map(|check| check.status.clone())
                .collect()
        }),
        human_evaluation,
    };
    let json =
        serde_json::to_string(&observation).context("failed to serialize routing observation")?;
    transaction.execute(
        r#"INSERT INTO routing_observations(
                id, run_id, candidate_id, created_at, updated_at, observation_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(run_id) DO UPDATE SET
                candidate_id = excluded.candidate_id,
                updated_at = excluded.updated_at,
                observation_json = excluded.observation_json"#,
        params![
            id,
            run.id,
            candidate.id,
            timestamp(observation.created_at),
            timestamp(observation.updated_at),
            json,
        ],
    )?;
    Ok(())
}

fn deserialize_routing_observation(json: String) -> Result<RoutingObservation> {
    serde_json::from_str(&json).context("failed to deserialize routing observation")
}

fn same_routing_human_evaluation(
    left: &RoutingHumanEvaluation,
    right: &RoutingHumanEvaluation,
) -> bool {
    left.outcome == right.outcome
        && left.reasons == right.reasons
        && left.explanation == right.explanation
}

fn insert_evaluation(
    transaction: &Transaction<'_>,
    run_id: &str,
    evaluation: &EvaluationRecord,
    mark_run_evaluated: bool,
) -> Result<()> {
    let (outcome, selected_candidate, selected_candidate_id) = match &evaluation.outcome {
        EvaluationOutcome::Candidate(selected) => {
            let candidate_id = transaction
                .query_row(
                    "SELECT id FROM candidates \
                     WHERE run_id = ?1 AND (id = ?2 OR label = ?2) \
                     ORDER BY CASE WHEN label = ?2 THEN 0 ELSE 1 END \
                     LIMIT 1",
                    params![run_id, selected],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
                .with_context(|| format!("run {run_id} has no candidate {selected}"))?;
            ("candidate", Some(selected.as_str()), Some(candidate_id))
        }
        EvaluationOutcome::Tie => ("tie", None, None),
        EvaluationOutcome::Neither => ("neither", None, None),
    };

    // This also makes re-submission atomic: old reasons disappear with the old
    // evaluation and are replaced in their original order.
    transaction.execute("DELETE FROM evaluations WHERE run_id = ?1", [run_id])?;
    let public_id = evaluation_public_id(run_id);
    transaction.execute(
        r#"INSERT INTO evaluations(
                run_id, outcome, selected_candidate, selected_candidate_id,
                explanation, created_at, blind, public_id
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
        params![
            run_id,
            outcome,
            selected_candidate,
            selected_candidate_id,
            evaluation.explanation,
            timestamp(evaluation.created_at),
            evaluation.blind,
            public_id,
        ],
    )?;
    let evaluation_id = transaction.last_insert_rowid();
    for (position, reason) in evaluation.reasons.iter().enumerate() {
        transaction.execute(
            "INSERT INTO evaluation_reasons(evaluation_id, position, reason) \
             VALUES (?1, ?2, ?3)",
            params![
                evaluation_id,
                usize_integer(position, "evaluation reason position")?,
                reason
            ],
        )?;
    }
    if mark_run_evaluated {
        transaction.execute(
            "UPDATE runs SET status = 'evaluated' WHERE id = ?1",
            [run_id],
        )?;
    }
    Ok(())
}

fn evaluation_public_id(run_id: &str) -> String {
    format!("evaluation-{run_id}")
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

fn insert_benchmark_prior(connection: &Connection, prior: &BenchmarkPrior) -> Result<()> {
    anyhow::ensure!(
        prior.successes <= prior.attempts,
        "benchmark prior successes cannot exceed attempts"
    );
    connection.execute(
        r#"INSERT INTO benchmark_priors(
                source, dataset, dataset_version, harness, model, language,
                task_kind, scope, successes, attempts, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            ON CONFLICT DO UPDATE SET
                successes = excluded.successes,
                attempts = excluded.attempts,
                updated_at = excluded.updated_at"#,
        params![
            prior.source,
            prior.dataset,
            prior.dataset_version,
            prior.harness,
            prior.model,
            prior.language,
            prior.task_kind.as_str(),
            prior.scope.as_str(),
            unsigned(prior.successes, "benchmark prior successes")?,
            unsigned(prior.attempts, "benchmark prior attempts")?,
            timestamp(prior.updated_at),
        ],
    )?;
    Ok(())
}

fn timestamp_from_sql(column: usize, value: String) -> rusqlite::Result<DateTime<Utc>> {
    value.parse().map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn unsigned(value: u64, field: &str) -> Result<i64> {
    i64::try_from(value).with_context(|| format!("{field} is too large for SQLite"))
}

fn usize_integer(value: usize, field: &str) -> Result<i64> {
    i64::try_from(value).with_context(|| format!("{field} is too large for SQLite"))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::{TimeZone, Utc};
    use rusqlite::OptionalExtension;
    use serde_json::json;

    use super::*;
    use crate::models::{
        CandidateStatus, CheckPhase, CheckStatus, DiffStats, EnvironmentRecord, RunStatus,
        SourceKind,
    };

    fn at(second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 21, 12, 0, second).unwrap()
    }

    fn check(name: &str, phase: CheckPhase) -> CheckResult {
        CheckResult {
            name: name.to_owned(),
            phase,
            command: format!("cargo {name}"),
            status: CheckStatus::Passed,
            exit_code: Some(0),
            duration_ms: 41,
            stdout_path: PathBuf::from(format!("/state/checks/{name}.stdout")),
            stderr_path: PathBuf::from(format!("/state/checks/{name}.stderr")),
        }
    }

    fn candidate(id: &str, label: &str, harness: &str) -> CandidateRecord {
        CandidateRecord {
            id: id.to_owned(),
            label: label.to_owned(),
            harness_id: harness.to_owned(),
            harness_version: Some("1.2.3".to_owned()),
            model: Some("test-model".to_owned()),
            status: CandidateStatus::Completed,
            workspace_path: PathBuf::from(format!("/state/{id}/workspace")),
            prompt_path: PathBuf::from(format!("/state/{id}/prompt.txt")),
            stdout_path: PathBuf::from(format!("/state/{id}/stdout.log")),
            stderr_path: PathBuf::from(format!("/state/{id}/stderr.log")),
            diff_path: PathBuf::from(format!("/state/{id}/diff.patch")),
            duration_ms: 2_500,
            exit_code: Some(0),
            timed_out: false,
            tokens: Some(12_345),
            token_semantics: Some("input+output".to_owned()),
            cost_usd: Some(0.125),
            error: None,
            diff_stats: DiffStats {
                files_changed: 2,
                lines_added: 17,
                lines_removed: 4,
                changed_files: vec!["src/lib.rs".to_owned(), "tests/regression.rs".to_owned()],
                untracked_files: vec!["tests/regression.rs".to_owned()],
            },
            checks: vec![check("test", CheckPhase::Verify)],
        }
    }

    fn run(id: &str) -> RunRecord {
        RunRecord {
            id: id.to_owned(),
            task: "Fix the retry race".to_owned(),
            exact_prompt: "Fix the retry race\n\nKeep the public API.".to_owned(),
            source_path: PathBuf::from("/code/worker"),
            source_kind: SourceKind::Directory,
            source_git_head: None,
            source_fingerprint: "sha256:abc".to_owned(),
            baseline_path: PathBuf::from(format!("/state/{id}/baseline")),
            baseline_commit: "0123456789abcdef".to_owned(),
            status: RunStatus::ReadyForEvaluation,
            created_at: at(1),
            completed_at: Some(at(4)),
            environment: EnvironmentRecord {
                dispatch_version: "0.1.0".to_owned(),
                os: "test-os".to_owned(),
                architecture: "test-arch".to_owned(),
                execution_backend: "fake".to_owned(),
                timeout_secs: 900,
                cpus: 2.0,
                memory: "4g".to_owned(),
                max_parallel: 3,
                docker_image: None,
                resource_limits_enforced: false,
                unsafe_local: false,
                forwarded_env: Vec::new(),
            },
            baseline_checks: vec![check("test", CheckPhase::Baseline)],
            candidates: vec![
                candidate("cand-a", "A", "codex"),
                candidate("cand-b", "B", "claude"),
            ],
            routing: None,
            evaluation: None,
            applied_candidate: None,
        }
    }

    #[test]
    fn applies_migration_and_enables_foreign_keys() -> Result<()> {
        let database = Database::open_in_memory()?;
        assert_eq!(database.schema_version()?, 9);
        let foreign_keys: i64 =
            database
                .connection
                .query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
        assert_eq!(foreign_keys, 1);

        for table in [
            "schema_migrations",
            "sources",
            "runs",
            "candidates",
            "checks",
            "artifacts",
            "events",
            "evaluations",
            "evaluation_reasons",
            "sync_settings",
            "sync_outbox",
            "benchmark_priors",
            "routing_observations",
        ] {
            let exists: bool = database.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                [table],
                |row| row.get(0),
            )?;
            assert!(exists, "missing {table} table");
        }
        database.health_check()
    }

    #[test]
    fn opens_file_database_and_creates_parent_directories() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("nested/state/dispatch.db");
        let database = Database::open(&path)?;
        assert!(path.is_file());
        assert_eq!(database.schema_version()?, 9);
        drop(database);

        // Opening an already-migrated database is idempotent.
        assert_eq!(Database::open(&path)?.schema_version()?, 9);
        Ok(())
    }

    #[test]
    fn migration_preserves_legacy_evaluation_consent_and_outbox_without_expanding_scope()
    -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("legacy.db");
        let connection = Connection::open(&path)?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;\
             CREATE TABLE schema_migrations (\
                 version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at TEXT NOT NULL\
             );",
        )?;
        for (version, name, sql) in MIGRATIONS.iter().take(8) {
            connection.execute_batch(sql)?;
            connection.execute(
                "INSERT INTO schema_migrations(version, name, applied_at) VALUES (?1, ?2, ?3)",
                params![version, name, timestamp(at(1))],
            )?;
        }
        let mut legacy = Database { connection };
        legacy.sync_run(&run("run-legacy-sync"))?;
        legacy.save_evaluation(
            "run-legacy-sync",
            &EvaluationRecord {
                outcome: EvaluationOutcome::Tie,
                reasons: vec![],
                explanation: None,
                created_at: at(5),
                blind: true,
            },
        )?;
        legacy.connection.execute(
            "UPDATE sync_settings SET enabled = 1, contributor_id = ?1, enabled_at = ?2",
            params!["legacy-contributor", timestamp(at(6))],
        )?;
        legacy.connection.execute(
            "INSERT INTO sync_outbox(evaluation_id, run_id, payload_json, status) \
             VALUES (?1, ?2, ?3, 'pending')",
            params![
                "evaluation-run-legacy-sync",
                "run-legacy-sync",
                "{\"schema_version\":1}"
            ],
        )?;
        drop(legacy);

        let database = Database::open(&path)?;
        let settings = database.sync_settings()?;
        assert!(settings.enabled);
        assert_eq!(settings.consent_version, 1);
        assert!(!settings.routing_observations_enabled());
        assert!(settings.routing_observation_enabled_at.is_none());
        let records = database.pending_sync_records()?;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].record_id, "evaluation-run-legacy-sync");
        assert_eq!(records[0].record_type, SyncRecordType::EvaluationV1);
        assert_eq!(records[0].payload_json, "{\"schema_version\":1}");
        Ok(())
    }

    #[test]
    fn syncs_run_candidates_checks_and_blind_harness_mapping() -> Result<()> {
        let mut database = Database::open_in_memory()?;
        let record = run("run-1");
        database.sync_run(&record)?;

        let summary = database.list_runs(10)?;
        assert_eq!(summary.len(), 1);
        assert_eq!(summary[0].id, "run-1");
        assert_eq!(summary[0].source_path, PathBuf::from("/code/worker"));
        assert_eq!(summary[0].candidate_count, 2);
        assert!(!summary[0].evaluated);

        let mapping: Vec<(String, String)> = {
            let mut statement = database.connection.prepare(
                "SELECT label, harness_id FROM candidates WHERE run_id = ?1 ORDER BY label",
            )?;
            statement
                .query_map(["run-1"], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?
        };
        assert_eq!(
            mapping,
            vec![("A".into(), "codex".into()), ("B".into(), "claude".into())]
        );

        let baseline_checks: i64 = database.connection.query_row(
            "SELECT COUNT(*) FROM checks WHERE run_id = ?1 AND candidate_id IS NULL",
            ["run-1"],
            |row| row.get(0),
        )?;
        let candidate_checks: i64 = database.connection.query_row(
            "SELECT COUNT(*) FROM checks WHERE run_id = ?1 AND candidate_id IS NOT NULL",
            ["run-1"],
            |row| row.get(0),
        )?;
        assert_eq!((baseline_checks, candidate_checks), (1, 2));

        let exact_prompt: String = database.connection.query_row(
            "SELECT exact_prompt FROM runs WHERE id = 'run-1'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(exact_prompt, record.exact_prompt);
        let token_semantics: String = database.connection.query_row(
            "SELECT token_semantics FROM candidates WHERE run_id = 'run-1' LIMIT 1",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(token_semantics, "input+output");

        // Re-sync replaces, rather than duplicates, child rows.
        database.sync_run(&record)?;
        assert_eq!(database.list_runs(10)?[0].candidate_count, 2);
        let checks: i64 = database.connection.query_row(
            "SELECT COUNT(*) FROM checks WHERE run_id = 'run-1'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(checks, 3);
        Ok(())
    }

    #[test]
    fn saves_candidate_evaluation_and_verbatim_freeform_text() -> Result<()> {
        let mut database = Database::open_in_memory()?;
        database.sync_run(&run("run-eval"))?;
        let explanation =
            "  B handled cancellation.\n\nKeep this indentation:\n    exact words  \n";
        let evaluation = EvaluationRecord {
            outcome: EvaluationOutcome::Candidate("B".to_owned()),
            reasons: vec!["correctness".to_owned(), "better architecture".to_owned()],
            explanation: Some(explanation.to_owned()),
            created_at: at(5),
            blind: true,
        };
        database.save_evaluation("run-eval", &evaluation)?;

        let stored: (String, String, String, bool) = database.connection.query_row(
            "SELECT outcome, selected_candidate, explanation, blind \
             FROM evaluations WHERE run_id = 'run-eval'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        assert_eq!(
            stored,
            ("candidate".into(), "B".into(), explanation.into(), true)
        );
        let reasons: Vec<String> = {
            let mut statement = database.connection.prepare(
                "SELECT er.reason FROM evaluation_reasons er \
                 JOIN evaluations e ON e.id = er.evaluation_id \
                 WHERE e.run_id = 'run-eval' ORDER BY er.position",
            )?;
            statement
                .query_map([], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        assert_eq!(reasons, evaluation.reasons);
        assert_eq!(database.list_runs(1)?[0].status, "evaluated");
        assert!(database.list_runs(1)?[0].evaluated);
        assert_eq!(
            database.evaluation_id("run-eval")?.as_deref(),
            Some("evaluation-run-eval")
        );
        Ok(())
    }

    #[test]
    fn sync_consent_is_off_by_default_and_identity_is_stable() -> Result<()> {
        let mut database = Database::open_in_memory()?;
        assert_eq!(
            database.sync_settings()?,
            SyncSettings {
                enabled: false,
                contributor_id: None,
                enabled_at: None,
                consent_version: 1,
                routing_observation_enabled_at: None,
            }
        );
        let first = database.enable_sync("install-first", at(1))?;
        assert!(first.enabled);
        assert!(first.routing_observations_enabled());
        assert_eq!(first.contributor_id.as_deref(), Some("install-first"));
        database.disable_sync()?;
        assert!(!database.sync_settings()?.enabled);
        let second = database.enable_sync("install-replacement", at(2))?;
        assert_eq!(second.contributor_id, first.contributor_id);
        assert_eq!(second.enabled_at, first.enabled_at);
        assert_eq!(
            second.routing_observation_enabled_at,
            first.routing_observation_enabled_at
        );

        database.sync_run(&run("run-sync"))?;
        database.save_evaluation(
            "run-sync",
            &EvaluationRecord {
                outcome: EvaluationOutcome::Tie,
                reasons: vec![],
                explanation: None,
                created_at: at(5),
                blind: true,
            },
        )?;
        let evaluation_id = database.evaluation_id("run-sync")?.unwrap();
        assert!(database.enqueue_sync(
            SyncRecordType::EvaluationV1,
            &evaluation_id,
            "run-sync",
            "{\"v\":1}"
        )?);
        assert!(!database.enqueue_sync(
            SyncRecordType::EvaluationV1,
            &evaluation_id,
            "run-sync",
            "{\"v\":1}"
        )?);
        assert!(!database.enqueue_sync(
            SyncRecordType::EvaluationV1,
            &evaluation_id,
            "run-sync",
            "different"
        )?);
        assert_eq!(
            database.pending_sync_records()?[0].payload_json,
            "{\"v\":1}"
        );
        database.mark_sync_failed(&evaluation_id, at(6), "offline")?;
        assert_eq!(database.sync_outbox_counts()?.failed, 1);
        database.mark_sync_succeeded(&evaluation_id, at(7))?;
        assert_eq!(database.sync_outbox_counts()?.synced, 1);
        assert!(!database.enqueue_sync(
            SyncRecordType::EvaluationV1,
            &evaluation_id,
            "run-sync",
            "changed-after-sync"
        )?);
        Ok(())
    }

    #[test]
    fn persists_tie_neither_and_embedded_evaluations() -> Result<()> {
        let mut database = Database::open_in_memory()?;
        let mut record = run("run-outcomes");
        record.status = RunStatus::Evaluated;
        record.evaluation = Some(EvaluationRecord {
            outcome: EvaluationOutcome::Tie,
            reasons: vec![],
            explanation: Some("Both are acceptable.\n".to_owned()),
            created_at: at(5),
            blind: true,
        });
        database.sync_run(&record)?;
        let outcome: String = database.connection.query_row(
            "SELECT outcome FROM evaluations WHERE run_id = 'run-outcomes'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(outcome, "tie");
        let public_id = database.evaluation_id("run-outcomes")?;

        // Later lifecycle transitions retain the evaluation without having the
        // embedded evaluation overwrite the authoritative run status.
        record.status = RunStatus::Applied;
        record.applied_candidate = Some("A".to_owned());
        database.sync_run(&record)?;
        assert_eq!(database.list_runs(1)?[0].status, "applied");
        assert_eq!(database.evaluation_id("run-outcomes")?, public_id);

        database.save_evaluation(
            "run-outcomes",
            &EvaluationRecord {
                outcome: EvaluationOutcome::Neither,
                reasons: vec!["other".to_owned()],
                explanation: Some("Neither preserves the public API.".to_owned()),
                created_at: at(6),
                blind: true,
            },
        )?;
        let outcome: String = database.connection.query_row(
            "SELECT outcome FROM evaluations WHERE run_id = 'run-outcomes'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(outcome, "neither");
        Ok(())
    }

    #[test]
    fn appends_structured_events_without_rewriting_them_on_sync() -> Result<()> {
        let mut database = Database::open_in_memory()?;
        let record = run("run-events");
        database.sync_run(&record)?;
        database.record_event(&EventRecord {
            run_id: record.id.clone(),
            candidate_label: Some("A".to_owned()),
            event_type: "candidate.completed".to_owned(),
            timestamp: at(5),
            payload: json!({"exit_code": 0, "message": "done"}),
        })?;
        database.sync_run(&record)?;

        let stored: (String, Option<String>, String) = database.connection.query_row(
            "SELECT event_type, candidate_label, payload_json FROM events WHERE run_id = ?1",
            [&record.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(stored.0, "candidate.completed");
        assert_eq!(stored.1.as_deref(), Some("A"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&stored.2)?,
            json!({"exit_code": 0, "message": "done"})
        );
        Ok(())
    }

    #[test]
    fn rejects_evaluation_for_unknown_candidate_without_losing_previous_one() -> Result<()> {
        let mut database = Database::open_in_memory()?;
        database.sync_run(&run("run-invalid"))?;
        database.save_evaluation(
            "run-invalid",
            &EvaluationRecord {
                outcome: EvaluationOutcome::Tie,
                reasons: vec![],
                explanation: None,
                created_at: at(5),
                blind: true,
            },
        )?;
        let result = database.save_evaluation(
            "run-invalid",
            &EvaluationRecord {
                outcome: EvaluationOutcome::Candidate("Z".to_owned()),
                reasons: vec![],
                explanation: None,
                created_at: at(6),
                blind: true,
            },
        );
        assert!(result.is_err());
        let existing = database
            .connection
            .query_row(
                "SELECT outcome FROM evaluations WHERE run_id = 'run-invalid'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        assert_eq!(existing.as_deref(), Some("tie"));
        Ok(())
    }
}
