use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{
    Connection, ErrorCode, OptionalExtension, Transaction, TransactionBehavior, params,
};

use crate::models::{
    AdmissionState, AdmissionSummary, AttemptRecord, BenchmarkPrior, CandidateRecord,
    CapacityObservation, CheckResult, EvaluationOutcome, EvaluationRecord, EventRecord,
    GoalFeedbackRevision, RoutingHumanEvaluation, RoutingObservation, RunMode, RunRecord,
    TaskFeatures,
};
use crate::public_priors::PublicPriorSnapshotV1;
use crate::sync::{RoutingFeedbackV1, RoutingObservationV1};
use crate::{RoutingHumanOutcome, evidence::LocalRoutingEvidence};

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
    (
        10,
        "routing_feedback_events",
        r#"
CREATE TABLE routing_feedback_events (
    id              TEXT PRIMARY KEY,
    observation_id  TEXT NOT NULL REFERENCES routing_observations(id),
    run_id          TEXT NOT NULL REFERENCES runs(id),
    revision        INTEGER NOT NULL CHECK (revision BETWEEN 1 AND 4294967295),
    created_at      TEXT NOT NULL,
    payload_json    TEXT NOT NULL,
    UNIQUE (observation_id, revision)
);
CREATE TABLE sync_outbox_v3 (
    record_id       TEXT PRIMARY KEY,
    run_id          TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    record_type     TEXT NOT NULL CHECK (record_type IN ('evaluation-v1', 'routing-observation-v1', 'routing-feedback-v1')),
    payload_json    TEXT NOT NULL,
    status          TEXT NOT NULL CHECK (status IN ('pending', 'failed', 'conflict', 'synced')),
    last_attempt    TEXT,
    last_error      TEXT,
    synced_at       TEXT
);
INSERT INTO sync_outbox_v3 SELECT * FROM sync_outbox;
DROP TABLE sync_outbox;
ALTER TABLE sync_outbox_v3 RENAME TO sync_outbox;
CREATE INDEX sync_outbox_status_idx ON sync_outbox(status);
CREATE INDEX sync_outbox_type_status_idx ON sync_outbox(record_type, status);
"#,
    ),
    (
        11,
        "distributed_public_priors",
        r#"
ALTER TABLE benchmark_priors ADD COLUMN origin TEXT NOT NULL DEFAULT 'manual'
    CHECK (origin IN ('manual', 'distributed'));
DROP INDEX benchmark_priors_identity_idx;
CREATE UNIQUE INDEX benchmark_priors_identity_idx ON benchmark_priors(
    source,
    dataset,
    dataset_version,
    harness,
    COALESCE(model, X''),
    COALESCE(language, X''),
    task_kind,
    scope,
    origin
);
CREATE TABLE distributed_public_prior_snapshot (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    snapshot_id     TEXT NOT NULL,
    generated_at    TEXT NOT NULL,
    installed_at    TEXT NOT NULL,
    installed_from  TEXT NOT NULL CHECK (installed_from IN ('bundled', 'downloaded'))
);
"#,
    ),
    (
        12,
        "truthful_attempt_outcomes",
        r#"
ALTER TABLE runs ADD COLUMN run_mode TEXT NOT NULL DEFAULT 'legacy'
    CHECK (run_mode IN ('legacy', 'routed', 'comparison'));
ALTER TABLE runs ADD COLUMN state_revision INTEGER NOT NULL DEFAULT 0 CHECK (state_revision >= 0);
ALTER TABLE runs ADD COLUMN outcome_json TEXT NOT NULL DEFAULT
    '{"version":1,"lifecycle":"finished","work_result":"pending","verification":"inconclusive","review":"not_requested","application":"not_applied","phase":"finished","waiting_on":"none"}';
ALTER TABLE runs ADD COLUMN run_projection_json TEXT;

CREATE TABLE attempts (
    id                  TEXT PRIMARY KEY,
    run_id              TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    candidate_id        TEXT NOT NULL,
    role                TEXT NOT NULL,
    ordinal             INTEGER NOT NULL CHECK (ordinal >= 1),
    generation          INTEGER NOT NULL DEFAULT 1 CHECK (generation >= 1),
    harness_id          TEXT NOT NULL,
    harness_version     TEXT,
    requested_model     TEXT,
    resolved_model      TEXT,
    observed_model      TEXT,
    requested_effort    TEXT,
    resolved_effort     TEXT,
    observed_effort     TEXT,
    started_at          TEXT NOT NULL,
    completed_at        TEXT,
    outcome             TEXT NOT NULL,
    raw_telemetry_path  TEXT NOT NULL,
    identity_provenance TEXT NOT NULL DEFAULT 'observed'
        CHECK (identity_provenance IN ('observed', 'legacy')),
    UNIQUE (run_id, ordinal),
    UNIQUE (run_id, candidate_id)
);
CREATE INDEX attempts_run_idx ON attempts(run_id, ordinal);

INSERT INTO attempts(
    id, run_id, candidate_id, role, ordinal, generation, harness_id,
    harness_version, observed_model, started_at, completed_at, outcome,
    raw_telemetry_path, identity_provenance
)
SELECT 'legacy-' || c.id, c.run_id, c.id, 'executor',
       (SELECT COUNT(*) FROM candidates earlier
        WHERE earlier.run_id = c.run_id AND earlier.label <= c.label),
       1, c.harness_id, c.harness_version, c.model, r.created_at, r.completed_at,
       c.status, '', 'legacy'
FROM candidates c JOIN runs r ON r.id = c.run_id;

ALTER TABLE events ADD COLUMN protocol_version INTEGER NOT NULL DEFAULT 0;
ALTER TABLE events ADD COLUMN sequence INTEGER;
ALTER TABLE events ADD COLUMN attempt_id TEXT;
ALTER TABLE events ADD COLUMN generation INTEGER NOT NULL DEFAULT 0;
ALTER TABLE events ADD COLUMN actor TEXT NOT NULL DEFAULT 'legacy';
UPDATE events
SET sequence = (
    SELECT COUNT(*) FROM events earlier
    WHERE earlier.run_id = events.run_id AND earlier.id <= events.id
);
CREATE UNIQUE INDEX events_run_sequence_idx ON events(run_id, sequence);
"#,
    ),
    (
        13,
        "resource_allocation",
        r#"
PRAGMA legacy_alter_table = ON;
ALTER TABLE runs RENAME TO runs_v12;
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
    applied_candidate   TEXT,
    docker_image        TEXT,
    resource_limits_enforced INTEGER NOT NULL DEFAULT 0,
    unsafe_local        INTEGER NOT NULL DEFAULT 0,
    forwarded_env_json  TEXT NOT NULL DEFAULT '[]',
    routing_decision_json TEXT,
    run_mode            TEXT NOT NULL DEFAULT 'legacy'
        CHECK (run_mode IN ('legacy', 'routed', 'allocation', 'comparison')),
    state_revision      INTEGER NOT NULL DEFAULT 0 CHECK (state_revision >= 0),
    outcome_json        TEXT NOT NULL,
    run_projection_json TEXT
);
INSERT INTO runs(
    id, source_id, task, exact_prompt, baseline_path, baseline_commit, status,
    created_at, completed_at, dispatch_version, os, architecture,
    execution_backend, timeout_secs, cpus, memory, max_parallel,
    applied_candidate, docker_image, resource_limits_enforced, unsafe_local,
    forwarded_env_json, routing_decision_json, run_mode, state_revision,
    outcome_json, run_projection_json
)
SELECT id, source_id, task, exact_prompt, baseline_path, baseline_commit, status,
       created_at, completed_at, dispatch_version, os, architecture,
       execution_backend, timeout_secs, cpus, memory, max_parallel,
       applied_candidate, docker_image, resource_limits_enforced, unsafe_local,
       forwarded_env_json, routing_decision_json, run_mode, state_revision,
       outcome_json, run_projection_json
FROM runs_v12;
DROP TABLE runs_v12;
PRAGMA legacy_alter_table = OFF;

ALTER TABLE attempts ADD COLUMN resource_snapshot_json TEXT;

CREATE TABLE allocation_decisions (
    run_id          TEXT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
    decision_json   TEXT NOT NULL,
    created_at      TEXT NOT NULL
);

CREATE TABLE goal_feedback_revisions (
    id              TEXT PRIMARY KEY,
    run_id          TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    revision        INTEGER NOT NULL CHECK (revision >= 1),
    outcome         TEXT NOT NULL CHECK (outcome IN ('accepted', 'rejected')),
    reasons_json    TEXT NOT NULL,
    explanation     TEXT,
    created_at      TEXT NOT NULL,
    UNIQUE (run_id, revision)
);
CREATE INDEX goal_feedback_run_idx ON goal_feedback_revisions(run_id, revision);
"#,
    ),
    (
        14,
        "capacity_and_admission",
        r#"
CREATE TABLE resource_pools (
    id                      TEXT PRIMARY KEY,
    provider                TEXT NOT NULL,
    funding_source          TEXT NOT NULL,
    provider_buckets_json   TEXT NOT NULL,
    max_active              INTEGER NOT NULL CHECK (max_active = 1),
    next_fence              INTEGER NOT NULL DEFAULT 0 CHECK (next_fence >= 0),
    created_at              TEXT NOT NULL,
    updated_at              TEXT NOT NULL
);

CREATE TABLE capacity_observations (
    id                      TEXT PRIMARY KEY,
    pool_id                 TEXT NOT NULL REFERENCES resource_pools(id),
    source                  TEXT NOT NULL,
    source_version          TEXT NOT NULL,
    provider_bucket_id      TEXT,
    window_identity         TEXT,
    sampled_at              TEXT NOT NULL,
    valid_until             TEXT NOT NULL,
    payload_json            TEXT NOT NULL
);
CREATE INDEX capacity_observations_pool_time_idx
    ON capacity_observations(pool_id, sampled_at, id);

CREATE TABLE admission_requests (
    id                      TEXT PRIMARY KEY,
    run_id                  TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    pool_id                 TEXT NOT NULL REFERENCES resource_pools(id),
    owner_session           TEXT NOT NULL,
    generation              INTEGER NOT NULL CHECK (generation >= 1),
    priority                INTEGER NOT NULL,
    enqueued_at             TEXT NOT NULL,
    heartbeat_at            TEXT NOT NULL,
    expires_at              TEXT NOT NULL,
    owner_pid               INTEGER,
    owner_start_identity    TEXT,
    owner_boot_identity     TEXT,
    status                  TEXT NOT NULL CHECK (status IN ('queued','admitted','reconciliation','released','abandoned','cancelled')),
    start_intent_at         TEXT,
    admitted_at             TEXT,
    released_at             TEXT
);
CREATE UNIQUE INDEX admission_one_runnable_per_goal_idx
    ON admission_requests(run_id) WHERE status IN ('queued','admitted','reconciliation');
CREATE INDEX admission_queue_idx
    ON admission_requests(pool_id, status, priority, enqueued_at, id);

CREATE TABLE pool_leases (
    pool_id                 TEXT PRIMARY KEY REFERENCES resource_pools(id),
    request_id              TEXT NOT NULL UNIQUE REFERENCES admission_requests(id),
    owner_session           TEXT NOT NULL,
    generation              INTEGER NOT NULL CHECK (generation >= 1),
    fence                   INTEGER NOT NULL CHECK (fence >= 1),
    state                   TEXT NOT NULL CHECK (state IN ('start_intent','running','reconciliation')),
    start_intent_at         TEXT NOT NULL,
    heartbeat_at            TEXT NOT NULL,
    expires_at              TEXT NOT NULL,
    owner_pid               INTEGER,
    owner_start_identity    TEXT,
    owner_boot_identity     TEXT,
    child_pid               INTEGER,
    child_start_identity    TEXT,
    child_boot_identity     TEXT,
    child_process_group     INTEGER,
    backend_identity        TEXT,
    cleanup_confirmed_at    TEXT
);
"#,
    ),
    (
        15,
        "phase2_safety_correction",
        r#"
ALTER TABLE resource_pools ADD COLUMN canonical_identity TEXT;
UPDATE resource_pools SET canonical_identity=id WHERE canonical_identity IS NULL;
CREATE UNIQUE INDEX resource_pools_canonical_identity_idx
    ON resource_pools(canonical_identity);

CREATE TABLE capacity_authorizations (
    id                      TEXT PRIMARY KEY,
    pool_id                 TEXT NOT NULL REFERENCES resource_pools(id),
    authorization_revision  INTEGER NOT NULL CHECK (authorization_revision >= 1),
    observation_id          TEXT NOT NULL REFERENCES capacity_observations(id),
    route_revision          TEXT NOT NULL,
    evidence_json           TEXT NOT NULL,
    status                  TEXT NOT NULL CHECK (status IN ('authorized','rejected')),
    reason                  TEXT,
    created_at              TEXT NOT NULL,
    valid_until             TEXT NOT NULL
);
CREATE INDEX capacity_authorizations_pool_revision_idx
    ON capacity_authorizations(pool_id,authorization_revision,created_at,id);

CREATE TABLE capacity_observation_constraints (
    observation_id          TEXT NOT NULL REFERENCES capacity_observations(id) ON DELETE CASCADE,
    ordinal                 INTEGER NOT NULL,
    provider_bucket_id      TEXT,
    window_identity         TEXT,
    constraint_json         TEXT NOT NULL,
    PRIMARY KEY (observation_id,ordinal)
);

ALTER TABLE admission_requests ADD COLUMN attempt_id TEXT REFERENCES attempts(id);
ALTER TABLE admission_requests ADD COLUMN route_snapshot_json TEXT;
ALTER TABLE admission_requests ADD COLUMN configuration_revision TEXT;
ALTER TABLE admission_requests ADD COLUMN authorization_revision INTEGER;
ALTER TABLE admission_requests ADD COLUMN authorization_id TEXT REFERENCES capacity_authorizations(id);
ALTER TABLE admission_requests ADD COLUMN canonical_pool_identity TEXT;
ALTER TABLE admission_requests ADD COLUMN fence INTEGER;

ALTER TABLE pool_leases ADD COLUMN attempt_id TEXT REFERENCES attempts(id);
ALTER TABLE pool_leases ADD COLUMN route_snapshot_json TEXT;
ALTER TABLE pool_leases ADD COLUMN configuration_revision TEXT;
ALTER TABLE pool_leases ADD COLUMN authorization_revision INTEGER;
ALTER TABLE pool_leases ADD COLUMN authorization_id TEXT REFERENCES capacity_authorizations(id);
ALTER TABLE pool_leases ADD COLUMN canonical_pool_identity TEXT;
ALTER TABLE pool_leases ADD COLUMN launch_lifecycle TEXT NOT NULL DEFAULT 'launch_intent_committed';
ALTER TABLE pool_leases ADD COLUMN launch_observation_id TEXT REFERENCES capacity_observations(id);
"#,
    ),
    (
        16,
        "repair_existing_lease_knowledge",
        r#"
-- Migration 15's default is not evidence that an existing lease never spawned.
-- Repair already-upgraded databases too; only recorded cleanup is affirmative.
UPDATE pool_leases
SET launch_lifecycle=CASE WHEN cleanup_confirmed_at IS NOT NULL
                         THEN 'cleanup_confirmed' ELSE 'cleanup_uncertain' END,
    state='reconciliation';
UPDATE admission_requests SET status='reconciliation'
WHERE id IN (SELECT request_id FROM pool_leases)
  AND status IN ('admitted','reconciliation');
"#,
    ),
    (
        17,
        "bounded_attempts_and_clarifications",
        r#"
ALTER TABLE attempts ADD COLUMN details_json TEXT;
ALTER TABLE runs ADD COLUMN delivery_attempt_id TEXT REFERENCES attempts(id);
CREATE TABLE clarifications (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id),
    attempt_id TEXT NOT NULL REFERENCES attempts(id),
    generation INTEGER NOT NULL,
    revision INTEGER NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('pending','answered','cancelled')),
    question_json TEXT NOT NULL
);
CREATE UNIQUE INDEX clarification_one_pending ON clarifications(run_id) WHERE state='pending';
"#,
    ),
    (
        18,
        "foreground_control_authority_and_receipts",
        r#"
CREATE TABLE control_grants (
    id TEXT PRIMARY KEY,
    secret_hash TEXT NOT NULL UNIQUE,
    scope_json TEXT NOT NULL
);
CREATE TABLE control_runs (
    run_id TEXT PRIMARY KEY REFERENCES runs(id),
    principal TEXT NOT NULL REFERENCES control_grants(id),
    session_id TEXT NOT NULL,
    cancelled INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE command_receipts (
    principal TEXT NOT NULL REFERENCES control_grants(id),
    request_id TEXT NOT NULL,
    payload_digest TEXT NOT NULL,
    run_id TEXT NOT NULL REFERENCES runs(id),
    reply_json TEXT NOT NULL,
    PRIMARY KEY(principal, request_id)
);
"#,
    ),
    (
        19,
        "private_outcome_annotations_and_policy",
        r#"
ALTER TABLE admission_requests ADD COLUMN launch_knowledge TEXT NOT NULL DEFAULT 'unknown';
CREATE TRIGGER private_launch_insert AFTER INSERT ON pool_leases BEGIN
 UPDATE admission_requests SET launch_knowledge=NEW.launch_lifecycle WHERE id=NEW.request_id;
END;
CREATE TRIGGER private_launch_update AFTER UPDATE OF launch_lifecycle ON pool_leases BEGIN
 UPDATE admission_requests SET launch_knowledge=NEW.launch_lifecycle WHERE id=NEW.request_id;
END;
CREATE INDEX private_attempt_admission ON admission_requests(attempt_id);
CREATE TABLE private_annotations (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL REFERENCES runs(id),
    feedback_id TEXT REFERENCES goal_feedback_revisions(id),
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TRIGGER private_annotations_append_only_update BEFORE UPDATE ON private_annotations
BEGIN SELECT RAISE(ABORT,'provenance annotations are append-only'); END;
CREATE TRIGGER private_annotations_append_only_delete BEFORE DELETE ON private_annotations
BEGIN SELECT RAISE(ABORT,'provenance annotations are append-only'); END;
CREATE TRIGGER private_decision_immutable BEFORE UPDATE OF run_projection_json ON runs
WHEN json_extract(OLD.run_projection_json,'$.allocation.private_evidence') IS NOT NULL
 AND json_extract(NEW.run_projection_json,'$.allocation.private_evidence') IS NOT json_extract(OLD.run_projection_json,'$.allocation.private_evidence')
BEGIN SELECT RAISE(ABORT,'private decision snapshot is immutable'); END;
CREATE INDEX private_annotation_run ON private_annotations(run_id, sequence);
CREATE INDEX private_source_path ON sources(path);
CREATE INDEX private_runs_source_window ON runs(source_id, created_at, id);
CREATE TABLE private_proposals (
    id TEXT PRIMARY KEY,
    source_key TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('proposed','active','stale','superseded','revoked'))
);
CREATE TABLE private_policy_transitions (
    source_key TEXT NOT NULL,
    revision INTEGER NOT NULL,
    previous_revision INTEGER NOT NULL,
    proposal_id TEXT REFERENCES private_proposals(id),
    actor TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY(source_key,revision)
);
CREATE TABLE private_fixture_domain (singleton INTEGER PRIMARY KEY CHECK(singleton=1));
CREATE TRIGGER private_fixture_only_empty BEFORE INSERT ON private_fixture_domain
WHEN EXISTS(SELECT 1 FROM runs) BEGIN SELECT RAISE(ABORT,'fixture domain requires empty state'); END;
CREATE TRIGGER private_fixture_no_delete BEFORE DELETE ON private_fixture_domain
BEGIN SELECT RAISE(ABORT,'fixture domain is permanent'); END;
CREATE TRIGGER private_fixture_no_update BEFORE UPDATE ON private_fixture_domain
BEGIN SELECT RAISE(ABORT,'fixture domain is permanent'); END;
"#,
    ),
    (
        20,
        "bounded_planning",
        r#"
CREATE TABLE planned_goals(run_id TEXT PRIMARY KEY REFERENCES runs(id), revision TEXT NOT NULL UNIQUE, policy_json TEXT NOT NULL, plan_json TEXT);
CREATE TABLE planned_tasks(run_id TEXT NOT NULL REFERENCES planned_goals(run_id), id TEXT NOT NULL, state TEXT NOT NULL, task_json TEXT NOT NULL, PRIMARY KEY(run_id,id));
CREATE TABLE planned_artifacts(run_id TEXT NOT NULL REFERENCES planned_goals(run_id), id TEXT NOT NULL, payload_json TEXT NOT NULL, PRIMARY KEY(run_id,id));
CREATE TABLE planned_snapshots(run_id TEXT NOT NULL REFERENCES planned_goals(run_id), id TEXT NOT NULL, payload_json TEXT NOT NULL, PRIMARY KEY(run_id,id));
CREATE TABLE planned_invocations(attempt_id TEXT PRIMARY KEY REFERENCES attempts(id), run_id TEXT NOT NULL REFERENCES planned_goals(run_id), slot TEXT NOT NULL, knowledge TEXT NOT NULL, UNIQUE(run_id,slot));
CREATE TRIGGER planned_launch_update AFTER UPDATE OF launch_lifecycle ON pool_leases BEGIN
 UPDATE planned_invocations SET knowledge=NEW.launch_lifecycle WHERE attempt_id=NEW.attempt_id;
END;
"#,
    ),
    (
        21,
        "attached_work_mode",
        r#"
PRAGMA legacy_alter_table = ON;
ALTER TABLE runs RENAME TO runs_v20;
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
    applied_candidate   TEXT,
    docker_image        TEXT,
    resource_limits_enforced INTEGER NOT NULL DEFAULT 0,
    unsafe_local        INTEGER NOT NULL DEFAULT 0,
    forwarded_env_json  TEXT NOT NULL DEFAULT '[]',
    routing_decision_json TEXT,
    run_mode            TEXT NOT NULL DEFAULT 'legacy'
        CHECK (run_mode IN ('legacy', 'routed', 'allocation', 'comparison', 'attached')),
    state_revision      INTEGER NOT NULL DEFAULT 0 CHECK (state_revision >= 0),
    outcome_json        TEXT NOT NULL,
    run_projection_json TEXT,
    delivery_attempt_id TEXT REFERENCES attempts(id)
);
INSERT INTO runs(
    id, source_id, task, exact_prompt, baseline_path, baseline_commit, status,
    created_at, completed_at, dispatch_version, os, architecture,
    execution_backend, timeout_secs, cpus, memory, max_parallel,
    applied_candidate, docker_image, resource_limits_enforced, unsafe_local,
    forwarded_env_json, routing_decision_json, run_mode, state_revision,
    outcome_json, run_projection_json, delivery_attempt_id
)
SELECT id, source_id, task, exact_prompt, baseline_path, baseline_commit, status,
       created_at, completed_at, dispatch_version, os, architecture,
       execution_backend, timeout_secs, cpus, memory, max_parallel,
       applied_candidate, docker_image, resource_limits_enforced, unsafe_local,
       forwarded_env_json, routing_decision_json, run_mode, state_revision,
       outcome_json, run_projection_json, delivery_attempt_id
FROM runs_v20;
DROP TABLE runs_v20;
PRAGMA legacy_alter_table = OFF;

-- legacy_alter_table leaves objects still bound to runs_v20 (not renamed
-- along with it), so DROP TABLE runs_v20 takes the index and trigger below
-- with it; migration 13 needed neither because they did not exist until
-- migration 19. Recreate them verbatim on the rebuilt table.
CREATE INDEX private_runs_source_window ON runs(source_id, created_at, id);
CREATE TRIGGER private_decision_immutable BEFORE UPDATE OF run_projection_json ON runs
WHEN json_extract(OLD.run_projection_json,'$.allocation.private_evidence') IS NOT NULL
 AND json_extract(NEW.run_projection_json,'$.allocation.private_evidence') IS NOT json_extract(OLD.run_projection_json,'$.allocation.private_evidence')
BEGIN SELECT RAISE(ABORT,'private decision snapshot is immutable'); END;
"#,
    ),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistributedPublicPriorSnapshot {
    pub snapshot_id: String,
    pub generated_at: DateTime<Utc>,
    pub installed_at: DateTime<Utc>,
    pub installed_from: String,
}

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
    RoutingFeedbackV1,
}

impl SyncRecordType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EvaluationV1 => "evaluation-v1",
            Self::RoutingObservationV1 => "routing-observation-v1",
            Self::RoutingFeedbackV1 => "routing-feedback-v1",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "evaluation-v1" => Ok(Self::EvaluationV1),
            "routing-observation-v1" => Ok(Self::RoutingObservationV1),
            "routing-feedback-v1" => Ok(Self::RoutingFeedbackV1),
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

fn sqlite_busy(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(details, _)
            if matches!(details.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    )
}

/// A missing mapping overlaps only within the same provider/funding identity.
/// Bucket sets need not be equal: any shared allowance implies exclusion.
pub(crate) fn overlapping_pools(connection: &Connection, pool_id: &str) -> Result<Vec<String>> {
    let mut statement = connection.prepare(
        "SELECT p.id FROM resource_pools p JOIN resource_pools q ON q.id=?1
         WHERE p.provider=q.provider AND p.funding_source=q.funding_source
         AND (json_array_length(p.provider_buckets_json)=0 OR json_array_length(q.provider_buckets_json)=0
              OR EXISTS(SELECT 1 FROM json_each(p.provider_buckets_json) a
                        JOIN json_each(q.provider_buckets_json) b ON a.value=b.value)) ORDER BY p.id",
    )?;
    Ok(statement
        .query_map([pool_id], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}

pub(crate) fn shared_capacity_history(
    connection: &Connection,
    pool_id: &str,
) -> Result<Vec<CapacityObservation>> {
    let mut history = Vec::new();
    for id in overlapping_pools(connection, pool_id)? {
        let mut statement = connection
            .prepare("SELECT payload_json FROM capacity_observations WHERE pool_id=?1")?;
        for row in statement.query_map([id], |row| row.get::<_, String>(0))? {
            history.push(serde_json::from_str::<CapacityObservation>(&row?)?);
        }
    }
    history.sort_by(|a, b| (a.sampled_at, &a.id).cmp(&(b.sampled_at, &b.id)));
    Ok(history)
}

fn authoritative_admission(
    connection: &Connection,
    run_id: &str,
) -> Result<Option<AdmissionSummary>> {
    connection
        .query_row(
            "SELECT r.id,r.pool_id,COALESCE(r.attempt_id,''),COALESCE(r.route_snapshot_json,''),COALESCE(r.configuration_revision,''),COALESCE(r.canonical_pool_identity,''),COALESCE(r.authorization_id,''),COALESCE(r.authorization_revision,0),r.owner_session,r.generation,r.priority,r.status,COALESCE(r.fence,l.fence),r.enqueued_at,r.released_at FROM admission_requests r LEFT JOIN pool_leases l ON l.request_id=r.id AND l.owner_session=r.owner_session AND l.generation=r.generation WHERE r.run_id=?1 ORDER BY r.enqueued_at DESC,r.id DESC LIMIT 1",
            [run_id],
            |row| {
                let status: String = row.get(11)?;
                let state = match status.as_str() {
                    "queued" => AdmissionState::Queued,
                    "admitted" => AdmissionState::Admitted,
                    "reconciliation" => AdmissionState::Reconciliation,
                    _ => AdmissionState::Released,
                };
                Ok(AdmissionSummary {
                    request_id: row.get(0)?,
                    pool_id: row.get(1)?,
                    attempt_id: row.get(2)?,
                    route_snapshot_json: row.get(3)?,
                    configuration_revision: row.get(4)?,
                    canonical_pool_identity: row.get(5)?,
                    authorization_id: row.get(6)?,
                    authorization_revision: u64::try_from(row.get::<_, i64>(7)?).unwrap_or(0),
                    owner_session: row.get(8)?,
                    generation: u64::try_from(row.get::<_, i64>(9)?).unwrap_or(0),
                    priority: row.get(10)?,
                    state,
                    fence: row
                        .get::<_, Option<i64>>(12)?
                        .and_then(|value| u64::try_from(value).ok()),
                    enqueued_at: timestamp_from_sql(13, row.get(13)?)?,
                    released_at: row
                        .get::<_, Option<String>>(14)?
                        .map(|value| timestamp_from_sql(14, value))
                        .transpose()?,
                })
            },
        )
        .optional()
        .context("failed to read authoritative admission state")
}

impl Database {
    /// Event followers never migrate, repair projections, or acquire execution authority.
    pub fn open_read_only(path: impl AsRef<Path>) -> Result<Self> {
        let connection =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        Ok(Self { connection })
    }

    /// Authoritative control and admission writes share the ordinary durable
    /// connection. Model execution never owns a database write transaction.
    pub fn open_control(path: impl AsRef<Path>) -> Result<Self> {
        Self::open(path)
    }

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
        database.backup_before_upgrade(path)?;
        database.configure(false)?;
        database
            .connection
            .pragma_update(None, "synchronous", "FULL")
            .context("failed to enable FULL SQLite durability")?;
        database.migrate()?;
        Ok(database)
    }

    /// Keep a transactionally consistent, private rollback copy before a real
    /// historical-schema upgrade. Opening current state creates no backup.
    fn backup_before_upgrade(&self, path: &Path) -> Result<()> {
        let initialized: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_migrations')", [], |r|r.get(0))?;
        if !initialized {
            return Ok(());
        }
        let version = self.schema_version()?;
        let latest = MIGRATIONS.last().context("missing schema migrations")?.0;
        ensure!(
            version <= latest,
            "state schema {version} is newer than this binary supports ({latest}); use the matching binary or restore a complete backed-up state directory"
        );
        if version == 0 || version == latest {
            return Ok(());
        }
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let backup = tempfile::Builder::new()
            .prefix(&format!("dispatch.schema-{version}-"))
            .suffix(".db")
            .tempfile_in(parent)?;
        self.connection
            .execute("VACUUM INTO ?1", [backup.path().to_string_lossy().as_ref()])
            .context("cannot preserve pre-upgrade database; state was not migrated")?;
        backup.as_file().sync_all()?;
        let (_, saved) = backup.keep()?;
        fs::File::open(parent)?
            .sync_all()
            .with_context(|| format!("cannot sync database backup {}", saved.display()))?;
        Ok(())
    }

    pub fn open_in_memory() -> Result<Self> {
        let connection =
            Connection::open_in_memory().context("failed to open in-memory database")?;
        let mut database = Self { connection };
        database.configure(true)?;
        database.migrate()?;
        Ok(database)
    }

    pub(crate) fn connection(&self) -> &Connection {
        &self.connection
    }

    pub(crate) fn connection_mut(&mut self) -> &mut Connection {
        &mut self.connection
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
            let mut last_busy = None;
            for _ in 0..200 {
                match self.connection.pragma_update(None, "journal_mode", "WAL") {
                    Ok(()) => {
                        last_busy = None;
                        break;
                    }
                    Err(error) if sqlite_busy(&error) => {
                        last_busy = Some(error);
                        std::thread::sleep(Duration::from_millis(25));
                    }
                    Err(error) => return Err(error).context("failed to enable SQLite WAL mode"),
                }
            }
            if let Some(error) = last_busy {
                return Err(error).context("failed to enable SQLite WAL mode after waiting");
            }
        }
        Ok(())
    }

    fn migrate(&mut self) -> Result<()> {
        let initialized = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_migrations')",
            [],
            |row| row.get::<_, bool>(0),
        )?;
        if !initialized {
            self.connection
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS schema_migrations (\
                    version INTEGER PRIMARY KEY, \
                    name TEXT NOT NULL, \
                    applied_at TEXT NOT NULL\
                );",
                )
                .context("failed to initialize schema migrations")?;
        }
        let applied = {
            let mut statement = self
                .connection
                .prepare("SELECT version FROM schema_migrations")?;
            statement
                .query_map([], |row| row.get::<_, i64>(0))?
                .collect::<rusqlite::Result<BTreeSet<_>>>()?
        };

        for (version, name, sql) in MIGRATIONS {
            if applied.contains(version) {
                continue;
            }
            // Migrations 13 and 21 rebuild `runs` (rename, recreate, copy,
            // drop). `PRAGMA foreign_keys` is a schema-level setting that is a
            // no-op inside a transaction, so it must be toggled here, on the
            // connection, before the migration's transaction begins: with it
            // left on, `DROP TABLE runs_v*` fails once any row in `attempts`,
            // `control_runs` or `planned_goals` references a run.
            let rebuilds_runs = *version == 13 || *version == 21;
            if rebuilds_runs {
                self.connection.pragma_update(None, "foreign_keys", false)?;
            }
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
                if rebuilds_runs {
                    self.connection.pragma_update(None, "foreign_keys", true)?;
                }
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
            if rebuilds_runs {
                self.connection.pragma_update(None, "foreign_keys", true)?;
            }
        }
        Ok(())
    }

    pub fn upsert_benchmark_prior(&self, prior: &BenchmarkPrior) -> Result<()> {
        insert_benchmark_prior(&self.connection, prior, "manual")
    }

    pub fn replace_benchmark_prior(&mut self, prior: &BenchmarkPrior) -> Result<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            r#"DELETE FROM benchmark_priors
               WHERE source = ?1
                 AND dataset = ?2
                 AND harness = ?3
                 AND model IS ?4
                 AND language IS ?5
                 AND task_kind = ?6
                 AND scope = ?7
                 AND origin = 'manual'"#,
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
        insert_benchmark_prior(&transaction, prior, "manual")?;
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

    pub fn replace_distributed_public_priors(
        &mut self,
        snapshot: &PublicPriorSnapshotV1,
        installed_from: &str,
    ) -> Result<()> {
        snapshot.validate()?;
        anyhow::ensure!(
            matches!(installed_from, "bundled" | "downloaded"),
            "invalid public prior installation source"
        );
        if self
            .distributed_public_prior_snapshot()?
            .is_some_and(|current| current.snapshot_id == snapshot.snapshot_id)
        {
            return Ok(());
        }

        let installed_at = Utc::now();
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "DELETE FROM benchmark_priors WHERE origin = 'distributed'",
            [],
        )?;
        for entry in &snapshot.entries {
            insert_benchmark_prior(
                &transaction,
                &entry.to_prior(snapshot.generated_at),
                "distributed",
            )?;
        }
        transaction.execute(
            r#"INSERT INTO distributed_public_prior_snapshot(
                    id, snapshot_id, generated_at, installed_at, installed_from
                ) VALUES (1, ?1, ?2, ?3, ?4)
                ON CONFLICT(id) DO UPDATE SET
                    snapshot_id = excluded.snapshot_id,
                    generated_at = excluded.generated_at,
                    installed_at = excluded.installed_at,
                    installed_from = excluded.installed_from"#,
            params![
                snapshot.snapshot_id,
                timestamp(snapshot.generated_at),
                timestamp(installed_at),
                installed_from,
            ],
        )?;
        transaction
            .commit()
            .context("failed to replace distributed public priors")
    }

    pub fn distributed_public_prior_snapshot(
        &self,
    ) -> Result<Option<DistributedPublicPriorSnapshot>> {
        self.connection
            .query_row(
                "SELECT snapshot_id, generated_at, installed_at, installed_from \
                 FROM distributed_public_prior_snapshot WHERE id = 1",
                [],
                |row| {
                    Ok(DistributedPublicPriorSnapshot {
                        snapshot_id: row.get(0)?,
                        generated_at: timestamp_from_sql(1, row.get(1)?)?,
                        installed_at: timestamp_from_sql(2, row.get(2)?)?,
                        installed_from: row.get(3)?,
                    })
                },
            )
            .optional()
            .context("failed to read distributed public prior snapshot")
    }

    pub fn distributed_public_prior_count(&self) -> Result<usize> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM benchmark_priors WHERE origin = 'distributed'",
            [],
            |row| row.get(0),
        )?;
        usize::try_from(count).context("distributed public prior count is invalid")
    }

    pub fn append_capacity_observation(&self, observation: &CapacityObservation) -> Result<()> {
        let payload = serde_json::to_string(observation)?;
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO capacity_observations(id,pool_id,source,source_version,provider_bucket_id,window_identity,sampled_at,valid_until,payload_json) VALUES(?1,?2,?3,?4,NULL,NULL,?5,?6,?7)",
            params![observation.id, observation.pool_id, observation.source, observation.source_version, timestamp(observation.sampled_at), timestamp(observation.valid_until), payload],
        )?;
        for (ordinal, constraint) in observation.constraints.iter().enumerate() {
            transaction.execute(
                "INSERT INTO capacity_observation_constraints(observation_id,ordinal,provider_bucket_id,window_identity,constraint_json) VALUES(?1,?2,?3,?4,?5)",
                params![observation.id, ordinal, constraint.provider_bucket_id, constraint.window_id, serde_json::to_string(constraint)?],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn latest_capacity_observation(
        &self,
        pool_id: &str,
    ) -> Result<Option<CapacityObservation>> {
        self.connection
            .query_row(
                "SELECT payload_json FROM capacity_observations WHERE pool_id=?1 ORDER BY sampled_at DESC,id DESC LIMIT 1",
                [pool_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|payload| serde_json::from_str(&payload).context("invalid capacity observation"))
            .transpose()
    }

    pub fn questions_for_run(&self, run_id: &str) -> Result<Vec<crate::Clarification>> {
        let mut statement = self
            .connection
            .prepare("SELECT question_json FROM clarifications WHERE run_id=?1 ORDER BY id")?;
        statement
            .query_map([run_id], |row| row.get::<_, String>(0))?
            .map(|row| Ok(serde_json::from_str(&row?)?))
            .collect()
    }

    pub fn admission_summary_for_run(&self, run_id: &str) -> Result<Option<AdmissionSummary>> {
        authoritative_admission(&self.connection, run_id)
    }

    pub fn manual_benchmark_priors(&self) -> Result<Vec<BenchmarkPrior>> {
        let mut statement = self.connection.prepare(
            "SELECT source, dataset, dataset_version, harness, model, language, \
                    task_kind, scope, successes, attempts, updated_at \
             FROM benchmark_priors WHERE origin = 'manual' \
             ORDER BY source, dataset, dataset_version, harness, model, language, task_kind, scope",
        )?;
        let rows = statement.query_map([], prior_from_row)?;
        rows.collect::<rusqlite::Result<_>>()
            .context("failed to read public benchmark priors")
    }

    /// Replace all structured state for a run in one transaction. Events are an
    /// append-only log and are deliberately not replaced by this operation.
    pub fn sync_run(&mut self, run: &RunRecord) -> Result<()> {
        self.sync_run_effect(run, |_| Ok(()))
    }

    /// Initial run, acceptance event, ownership and receipt share one commit.
    pub(crate) fn commit_run_transition(
        &mut self,
        run: &mut RunRecord,
        event: EventRecord,
    ) -> Result<EventRecord> {
        let mut projection = run.clone();
        let mut committed = None;
        self.sync_run_effect(run, |transaction| {
            committed = Some(Self::transition_on(transaction, &mut projection, event)?);
            Ok(())
        })?;
        *run = projection;
        Ok(committed.expect("created transition"))
    }

    fn sync_run_effect(
        &mut self,
        run: &RunRecord,
        effect: impl FnOnce(&Transaction<'_>) -> Result<()>,
    ) -> Result<()> {
        let routing_decision_json = run
            .routing
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("failed to serialize routing decision")?;
        let outcome_json =
            serde_json::to_string(&run.outcome).context("failed to serialize run outcome")?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Admission binds immutably to an attempt. The legacy projection
        // writer replaces attempt rows within this same transaction, so defer
        // that foreign-key check until the replacement is complete.
        transaction.execute_batch("PRAGMA defer_foreign_keys=ON;")?;

        let stored_revision = transaction
            .query_row(
                "SELECT state_revision FROM runs WHERE id=?1",
                [&run.id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if let Some(stored_revision) = stored_revision {
            anyhow::ensure!(
                u64::try_from(stored_revision)
                    .ok()
                    .is_some_and(|value| value <= run.state_revision),
                "refusing to overwrite a newer committed run projection"
            );
        }
        validate_terminal_write(&transaction, run, false)?;
        let mut projection = run.clone();
        if let Some(admission) = authoritative_admission(&transaction, &run.id)? {
            projection.admission = Some(admission);
        }
        let run_projection_json = serde_json::to_string(&projection)
            .context("failed to serialize authoritative run projection")?;

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
                    forwarded_env_json, applied_candidate, routing_decision_json,
                    run_mode, state_revision, outcome_json, run_projection_json
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                    ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23,
                    ?24, ?25, ?26, ?27
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
                    routing_decision_json = excluded.routing_decision_json,
                    run_mode = excluded.run_mode,
                    state_revision = excluded.state_revision,
                    outcome_json = excluded.outcome_json,
                    run_projection_json = excluded.run_projection_json"#,
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
                run.mode.as_str(),
                unsigned(run.state_revision, "run state revision")?,
                outcome_json,
                run_projection_json,
            ],
        )?;

        if let Some(decision) = &run.allocation {
            let decision_json = serde_json::to_string(decision)?;
            let existing = transaction
                .query_row(
                    "SELECT decision_json FROM allocation_decisions WHERE run_id = ?1",
                    [&run.id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            anyhow::ensure!(
                existing
                    .as_ref()
                    .is_none_or(|stored| stored == &decision_json),
                "allocation decision for run {} is immutable",
                run.id
            );
            transaction.execute(
                "INSERT OR IGNORE INTO allocation_decisions(run_id, decision_json, created_at) VALUES (?1, ?2, ?3)",
                params![run.id, decision_json, timestamp(run.created_at)],
            )?;
        }

        // Evaluations reference candidates, so remove the old evaluation before
        // replacing a run's candidate set. Reasons cascade from the evaluation.
        transaction.execute("DELETE FROM evaluations WHERE run_id = ?1", [&run.id])?;
        transaction.execute("DELETE FROM checks WHERE run_id = ?1", [&run.id])?;
        transaction.execute("DELETE FROM artifacts WHERE run_id = ?1", [&run.id])?;
        transaction.execute("DELETE FROM attempts WHERE run_id = ?1 AND (details_json IS NULL OR completed_at IS NULL)", [&run.id])?;
        transaction.execute("DELETE FROM candidates WHERE run_id = ?1", [&run.id])?;

        for candidate in &run.candidates {
            insert_candidate(&transaction, &run.id, candidate)?;
            insert_candidate_artifacts(&transaction, &run.id, candidate)?;
            for check in &candidate.checks {
                insert_check(&transaction, &run.id, Some(&candidate.id), check)?;
                insert_check_artifacts(&transaction, &run.id, Some(&candidate.id), check)?;
            }
        }
        for attempt in &run.attempts {
            insert_attempt(&transaction, attempt)?;
        }
        transaction.execute(
            "UPDATE runs SET delivery_attempt_id=?2 WHERE id=?1",
            params![
                run.id,
                run.phase3
                    .as_ref()
                    .and_then(|p| p.final_attempt_id.as_deref())
            ],
        )?;

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

        effect(&transaction)?;
        transaction.commit().context("failed to commit run sync")
    }

    pub fn save_goal_feedback(
        &mut self,
        run_id: &str,
        outcome: RoutingHumanOutcome,
        reasons: Vec<String>,
        explanation: Option<String>,
    ) -> Result<GoalFeedbackRevision> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let revision: u32 = transaction.query_row(
            "SELECT COALESCE(MAX(revision), 0) + 1 FROM goal_feedback_revisions WHERE run_id = ?1",
            [run_id],
            |row| row.get(0),
        )?;
        let feedback = GoalFeedbackRevision {
            id: format!("goal-feedback-{run_id}-{revision}"),
            run_id: run_id.to_owned(),
            revision,
            outcome,
            reasons,
            explanation,
            created_at: Utc::now(),
        };
        transaction.execute(
            "INSERT INTO goal_feedback_revisions(id, run_id, revision, outcome, reasons_json, explanation, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                feedback.id,
                feedback.run_id,
                feedback.revision,
                feedback.outcome.as_str(),
                serde_json::to_string(&feedback.reasons)?,
                feedback.explanation,
                timestamp(feedback.created_at),
            ],
        )?;
        transaction.commit()?;
        Ok(feedback)
    }

    pub fn latest_goal_feedback(&self, run_id: &str) -> Result<Option<GoalFeedbackRevision>> {
        self.connection
            .query_row(
                "SELECT id, revision, outcome, reasons_json, explanation, created_at FROM goal_feedback_revisions WHERE run_id = ?1 ORDER BY revision DESC LIMIT 1",
                [run_id],
                |row| {
                    let outcome = match row.get::<_, String>(2)?.as_str() {
                        "accepted" => RoutingHumanOutcome::Accepted,
                        "rejected" => RoutingHumanOutcome::Rejected,
                        value => return Err(rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, format!("invalid goal feedback outcome {value}").into())),
                    };
                    let reasons_json: String = row.get(3)?;
                    let reasons = serde_json::from_str(&reasons_json).map_err(|error| rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(error)))?;
                    Ok(GoalFeedbackRevision {
                        id: row.get(0)?,
                        run_id: run_id.to_owned(),
                        revision: row.get(1)?,
                        outcome,
                        reasons,
                        explanation: row.get(4)?,
                        created_at: timestamp_from_sql(5, row.get(5)?)?,
                    })
                },
            )
            .optional()
            .context("failed to read goal feedback")
    }

    pub fn record_event(&self, event: &EventRecord) -> Result<()> {
        let payload =
            serde_json::to_string(&event.payload).context("failed to serialize event payload")?;
        self.connection
            .execute(
                "INSERT INTO events(run_id, candidate_label, event_type, timestamp, payload_json, \
                                    protocol_version, sequence, attempt_id, generation, actor) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6,
                         COALESCE(NULLIF(?7, 0), (SELECT COALESCE(MAX(sequence), 0) + 1 FROM events WHERE run_id = ?1)),
                         ?8, ?9, ?10)",
                params![
                    event.run_id,
                    event.candidate_label,
                    event.event_type,
                    timestamp(event.timestamp),
                    payload,
                    event.protocol_version,
                    unsigned(event.sequence, "event sequence")?,
                    event.attempt_id,
                    event.generation,
                    event.actor,
                ],
            )
            .with_context(|| format!("failed to record {} event", event.event_type))?;
        Ok(())
    }

    /// Commit the semantic state revision and its event in one SQLite transaction.
    /// Files in the run directory are projections written only after this succeeds.
    pub fn commit_transition(
        &self,
        run: &mut RunRecord,
        event: EventRecord,
    ) -> Result<EventRecord> {
        let transaction =
            Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let event = Self::transition_on(&transaction, run, event)?;
        transaction.commit()?;
        Ok(event)
    }

    fn transition_on(
        transaction: &Transaction<'_>,
        run: &mut RunRecord,
        mut event: EventRecord,
    ) -> Result<EventRecord> {
        let stored_revision: i64 = transaction.query_row(
            "SELECT state_revision FROM runs WHERE id = ?1",
            [&run.id],
            |row| row.get(0),
        )?;
        anyhow::ensure!(
            u64::try_from(stored_revision).ok() == Some(run.state_revision),
            "run state revision changed concurrently"
        );
        let sequence: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(sequence), 0) + 1 FROM events WHERE run_id = ?1",
            [&run.id],
            |row| row.get(0),
        )?;
        run.state_revision = run
            .state_revision
            .checked_add(1)
            .context("run state revision overflow")?;
        event.protocol_version = 1;
        event.sequence = u64::try_from(sequence).context("event sequence is invalid")?;
        if let Some(actor) = crate::commands::actor() {
            event.actor = actor;
        }
        if event.actor.is_empty() {
            event.actor = "orchestrator".into();
        }
        if event.generation == 0 {
            event.generation = 1;
        }
        // Every committed event carries its semantic outcome so cursor followers
        // can observe even a waiting transition that has already been answered.
        if !event.payload.is_object() {
            event.payload = serde_json::json!({"data": event.payload});
        }
        event
            .payload
            .as_object_mut()
            .expect("normalized event object")
            .insert("outcome".into(), serde_json::to_value(&run.outcome)?);
        persist_questions(transaction, run)?;
        crate::planning::persist(transaction, run)?;
        let outcome_json = serde_json::to_string(&run.outcome)?;
        validate_terminal_write(transaction, run, event.event_type == "review.accepted")?;
        let mut projection = run.clone();
        if let Some(admission) = authoritative_admission(transaction, &run.id)? {
            projection.admission = Some(admission);
        }
        let projection_json = serde_json::to_string(&projection)?;
        transaction.execute(
            "UPDATE runs SET state_revision = ?2, outcome_json = ?3, run_projection_json = ?4, \
                             run_mode = ?5, status = ?6, completed_at = ?7, applied_candidate = ?8 \
             WHERE id = ?1",
            params![
                run.id,
                unsigned(run.state_revision, "run state revision")?,
                outcome_json,
                projection_json,
                run.mode.as_str(),
                run.status.as_str(),
                run.completed_at.map(timestamp),
                run.applied_candidate,
            ],
        )?;
        let payload = serde_json::to_string(&event.payload)?;
        transaction.execute(
            "INSERT INTO events(run_id, candidate_label, event_type, timestamp, payload_json, \
                                protocol_version, sequence, attempt_id, generation, actor) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                event.run_id,
                event.candidate_label,
                event.event_type,
                timestamp(event.timestamp),
                payload,
                event.protocol_version,
                sequence,
                event.attempt_id,
                event.generation,
                event.actor,
            ],
        )?;
        crate::commands::commit_receipt(transaction, run, &mut event)?;
        // Keep immediate JSON/results and the file projection consistent with
        // the same admission authority used by subsequent status reads.
        run.admission = projection.admission;
        Ok(event)
    }

    pub fn committed_run_projection(&self, run_id: &str) -> Result<Option<RunRecord>> {
        let json = self
            .connection
            .query_row(
                "SELECT run_projection_json FROM runs WHERE id = ?1",
                [run_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        json.map(|json| serde_json::from_str(&json).context("invalid committed run projection"))
            .transpose()
    }

    pub fn events_for_run(&self, run_id: &str) -> Result<Vec<EventRecord>> {
        self.events_after(run_id, 0, i64::MAX as u64)
    }

    pub fn event_page(
        &self,
        run_id: &str,
        after: u64,
        limit: u64,
    ) -> Result<(Vec<EventRecord>, Option<RunRecord>)> {
        let transaction = self.connection.unchecked_transaction()?;
        let latest: u64 = self.connection.query_row(
            "SELECT COALESCE(MAX(sequence), 0) FROM events WHERE run_id = ?1",
            [run_id],
            |row| row.get(0),
        )?;
        anyhow::ensure!(after <= latest, "cursor is ahead of the committed journal");
        let events = self.events_after(run_id, after, limit)?;
        let run = self.committed_run_projection(run_id)?;
        transaction.commit()?;
        Ok((events, run))
    }

    pub fn events_after(&self, run_id: &str, after: u64, limit: u64) -> Result<Vec<EventRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT protocol_version, sequence, attempt_id, generation, actor,
                    candidate_label, event_type, timestamp, payload_json
             FROM events WHERE run_id = ?1 AND sequence > ?2 ORDER BY sequence LIMIT ?3",
        )?;
        let rows = statement.query_map(params![run_id, after, limit], |row| {
            let payload_json: String = row.get(8)?;
            let payload = serde_json::from_str(&payload_json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    8,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            Ok(EventRecord {
                protocol_version: row.get(0)?,
                run_id: run_id.to_owned(),
                sequence: row.get::<_, u64>(1)?,
                attempt_id: row.get(2)?,
                generation: row.get(3)?,
                actor: row.get(4)?,
                candidate_label: row.get(5)?,
                event_type: row.get(6)?,
                timestamp: timestamp_from_sql(7, row.get(7)?)?,
                payload,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to replay run events")
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

    /// Derive source-local counts in one SQLite read snapshot. `source` is the
    /// canonical location already stored by run preparation, not a repository ID.
    /// No sync preparation, outbox writes, public priors or Cloud queries occur.
    pub fn local_routing_evidence(&self, source: &Path) -> Result<Vec<LocalRoutingEvidence>> {
        let mut statement = self.connection.prepare(
            "SELECT o.observation_json, f.id, f.run_id, f.revision, f.payload_json \
             FROM routing_observations o JOIN runs r ON r.id = o.run_id \
             JOIN sources s ON s.id = r.source_id \
             LEFT JOIN routing_feedback_events f ON f.id = (\
                 SELECT id FROM routing_feedback_events WHERE observation_id = o.id \
                 ORDER BY revision DESC LIMIT 1) \
             WHERE s.path = ?1 ORDER BY o.id",
        )?;
        let mut rows = statement.query([path_text(source)])?;
        let mut groups = BTreeMap::new();
        while let Some(row) = rows.next()? {
            let observation = deserialize_routing_observation(row.get(0)?)?;
            if !observation.candidate_status.is_terminal() {
                continue;
            }
            let latest = row.get::<_, Option<String>>(4)?;
            let human = if let Some(json) = latest {
                let event: RoutingFeedbackV1 =
                    serde_json::from_str(&json).context("invalid local routing feedback event")?;
                anyhow::ensure!(
                    event.schema_version == 1
                        && event.revision > 0
                        && event.consent.scope == "routing-observation-v1"
                        && event.feedback_event_id == row.get::<_, String>(1)?
                        && event.observation_id == observation.id
                        && row.get::<_, String>(2)? == observation.run_id
                        && i64::from(event.revision) == row.get::<_, i64>(3)?,
                    "local routing feedback identity or version mismatch"
                );
                Some(match event.outcome.as_str() {
                    "accept" => RoutingHumanOutcome::Accepted,
                    "reject" => RoutingHumanOutcome::Rejected,
                    _ => anyhow::bail!("invalid local routing feedback outcome"),
                })
            } else {
                observation
                    .human_evaluation
                    .as_ref()
                    .map(|human| human.outcome.clone())
            };
            let features = &observation.prediction.task_features;
            let key = (
                features.language.clone(),
                features.task_kind.as_str(),
                features.scope.as_str(),
                observation.prediction.selected_harness.clone(),
            );
            groups
                .entry(key)
                .or_insert_with(|| LocalRoutingEvidence {
                    task_features: features.clone(),
                    harness: observation.prediction.selected_harness.clone(),
                    ..Default::default()
                })
                .count(&observation, human.as_ref());
        }
        Ok(groups.into_values().collect())
    }

    pub fn save_routing_human_evaluation(
        &mut self,
        run_id: &str,
        evaluation: &RoutingHumanEvaluation,
    ) -> Result<RoutingObservation> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let json = transaction
            .query_row(
                "SELECT observation_json FROM routing_observations WHERE run_id = ?1",
                [run_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .with_context(|| format!("run {run_id} has no routing observation"))?;
        let mut observation = deserialize_routing_observation(json)?;
        // Recover a known state left by an older binary before applying a new revision.
        record_routing_feedback(&transaction, &observation)?;
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
        record_routing_feedback(&transaction, &observation)?;
        transaction
            .commit()
            .context("failed to commit routing evaluation")?;
        Ok(observation)
    }

    pub fn synced_routing_payload(&self, run_id: &str) -> Result<Option<String>> {
        self.connection
            .query_row(
                "SELECT payload_json FROM sync_outbox WHERE run_id = ?1 \
             AND record_type = 'routing-observation-v1' AND status = 'synced'",
                [run_id],
                |row| row.get(0),
            )
            .optional()
            .context("failed to read synced routing parent")
    }

    pub fn routing_feedback_for_run(&self, run_id: &str) -> Result<Vec<RoutingFeedbackV1>> {
        let mut statement = self.connection.prepare(
            "SELECT payload_json FROM routing_feedback_events WHERE run_id = ?1 ORDER BY revision",
        )?;
        let rows = statement.query_map([run_id], |row| row.get::<_, String>(0))?;
        rows.map(|row| serde_json::from_str(&row?).context("invalid routing feedback event"))
            .collect()
    }

    pub fn reconcile_routing_feedback(&self, run_id: Option<&str>) -> Result<usize> {
        let transaction =
            Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let mut statement = transaction.prepare(
            "SELECT o.observation_json FROM routing_observations o JOIN sync_outbox s \
             ON s.record_id = o.id WHERE s.record_type = 'routing-observation-v1' AND s.status = 'synced' \
             AND (?1 IS NULL OR o.run_id = ?1)",
        )?;
        let observations = statement
            .query_map([run_id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        let mut created = 0;
        for json in observations {
            created += usize::from(record_routing_feedback(
                &transaction,
                &deserialize_routing_observation(json)?,
            )?);
        }
        // The semantic event survives independently of outbox retention.
        transaction.execute(
            "INSERT OR IGNORE INTO sync_outbox(record_id, run_id, record_type, payload_json, status) \
             SELECT f.id, f.run_id, 'routing-feedback-v1', f.payload_json, 'pending' \
             FROM routing_feedback_events f JOIN sync_outbox p ON p.record_id = f.observation_id \
             WHERE p.record_type = 'routing-observation-v1' AND p.status = 'synced' \
             AND (?1 IS NULL OR f.run_id = ?1)", [run_id],
        )?;
        transaction.commit()?;
        Ok(created)
    }

    pub(crate) fn sync_payload_for_record(&self, record_id: &str) -> Result<String> {
        self.connection
            .query_row(
                "SELECT payload_json FROM sync_outbox WHERE record_id = ?1",
                [record_id],
                |row| row.get(0),
            )
            .context("sync payload is missing")
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
            SyncRecordType::RoutingFeedbackV1 => self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM routing_feedback_events WHERE id = ?1 AND run_id = ?2 AND payload_json = ?3)",
                params![record_id, run_id, payload_json], |row| row.get::<_, bool>(0),
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
                    record_type != SyncRecordType::RoutingFeedbackV1
                        && !matches!(status.as_str(), "synced" | "conflict"),
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
        anyhow::ensure!(
            record_type != SyncRecordType::RoutingFeedbackV1,
            "routing feedback requires a revision selector"
        );
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
            "SELECT s.record_id, s.run_id, s.record_type, s.payload_json FROM sync_outbox s \
             LEFT JOIN routing_feedback_events f ON f.id = s.record_id \
             WHERE s.status IN ('pending', 'failed') ORDER BY s.run_id, COALESCE(f.revision, 0), s.record_id",
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

fn insert_attempt(transaction: &Transaction<'_>, attempt: &AttemptRecord) -> Result<()> {
    let details = serde_json::to_string(attempt)?;
    let old: Option<String> = transaction
        .query_row(
            "SELECT details_json FROM attempts WHERE id=?1 AND completed_at IS NOT NULL",
            [&attempt.id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    if let Some(old) = old {
        anyhow::ensure!(old == details, "completed attempt evidence is immutable");
        return Ok(());
    }
    transaction.execute(
        r#"INSERT INTO attempts(
                id, run_id, candidate_id, role, ordinal, generation, harness_id,
                harness_version, requested_model, resolved_model, observed_model,
                requested_effort, resolved_effort, observed_effort, started_at,
                completed_at, outcome, raw_telemetry_path, identity_provenance,
                resource_snapshot_json, details_json
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                ?12, ?13, ?14, ?15, ?16, ?17, ?18, 'observed', ?19, ?20
            )"#,
        params![
            attempt.id,
            attempt.run_id,
            attempt.candidate_id,
            attempt.role,
            attempt.ordinal,
            attempt.generation,
            attempt.harness_id,
            attempt.harness_version,
            attempt.requested_model,
            attempt.resolved_model,
            attempt.observed_model,
            attempt.requested_effort,
            attempt.resolved_effort,
            attempt.observed_effort,
            timestamp(attempt.started_at),
            attempt.completed_at.map(timestamp),
            attempt.outcome,
            path_text(&attempt.raw_telemetry_path),
            attempt
                .resource
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?,
            details,
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
    if run.mode == RunMode::Allocation {
        return Ok(());
    }
    let Some(prediction) = &run.routing else {
        return Ok(());
    };
    let [candidate] = run.candidates.as_slice() else {
        return Ok(());
    };
    if !candidate.status.is_terminal() {
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

/// Called only inside an immediate transaction: state change, revision and outbox
/// become durable together. The wire snapshot is also the local semantic event.
fn record_routing_feedback(
    transaction: &Transaction<'_>,
    observation: &RoutingObservation,
) -> Result<bool> {
    let Some(human) = &observation.human_evaluation else {
        return Ok(false);
    };
    let parent = transaction
        .query_row(
            "SELECT payload_json FROM sync_outbox WHERE record_id = ?1 \
         AND record_type = 'routing-observation-v1' AND status = 'synced'",
            [&observation.id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(parent) = parent else {
        return Ok(false);
    };
    let parent: RoutingObservationV1 =
        serde_json::from_str(&parent).context("invalid synced routing parent")?;
    anyhow::ensure!(
        parent.schema_version == 1
            && parent.observation_id == observation.id
            && parent.run_id == observation.run_id
            && parent.consent.scope == "routing-observation-v1",
        "synced routing parent identity or version mismatch"
    );
    let latest = transaction.query_row(
        "SELECT payload_json FROM routing_feedback_events WHERE observation_id = ?1 ORDER BY revision DESC LIMIT 1",
        [&observation.id], |row| row.get::<_, String>(0),
    ).optional()?;
    let (previous_revision, previous_state) = if let Some(latest) = latest {
        let latest: RoutingFeedbackV1 = serde_json::from_str(&latest)?;
        (
            latest.revision,
            Some((latest.outcome, latest.reasons, latest.explanation)),
        )
    } else {
        let previous = parent
            .human_evaluation
            .map(|old| -> Result<_> {
                let outcome = match old.outcome.as_str() {
                    "accepted" => "accept",
                    "rejected" => "reject",
                    _ => anyhow::bail!("invalid human outcome in synced routing parent"),
                };
                Ok((outcome.to_owned(), old.reasons, old.explanation))
            })
            .transpose()?;
        (0, previous)
    };
    let outcome = match human.outcome {
        crate::RoutingHumanOutcome::Accepted => "accept",
        crate::RoutingHumanOutcome::Rejected => "reject",
    };
    if previous_state
        .as_ref()
        .is_some_and(|(old_outcome, reasons, explanation)| {
            old_outcome == outcome && reasons == &human.reasons && explanation == &human.explanation
        })
    {
        return Ok(false);
    }
    let revision = previous_revision
        .checked_add(1)
        .context("routing feedback revision exhausted")?;
    let event = RoutingFeedbackV1 {
        schema_version: 1,
        feedback_event_id: format!("routing-feedback-{}-{revision}", observation.run_id),
        observation_id: observation.id.clone(),
        contributor_id: parent.contributor_id,
        revision,
        consent: parent.consent,
        outcome: outcome.into(),
        reasons: human.reasons.clone(),
        explanation: human.explanation.clone(),
        evaluated_at: human.evaluated_at,
    };
    let payload = serde_json::to_string_pretty(&event)?;
    transaction.execute(
        "INSERT INTO routing_feedback_events(id, observation_id, run_id, revision, created_at, payload_json) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![event.feedback_event_id, observation.id, observation.run_id, revision, timestamp(Utc::now()), payload],
    )?;
    transaction.execute(
        "INSERT INTO sync_outbox(record_id, run_id, record_type, payload_json, status) \
         VALUES (?1, ?2, 'routing-feedback-v1', ?3, 'pending')",
        params![event.feedback_event_id, observation.run_id, payload],
    )?;
    Ok(true)
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

fn insert_benchmark_prior(
    connection: &Connection,
    prior: &BenchmarkPrior,
    origin: &str,
) -> Result<()> {
    anyhow::ensure!(
        prior.successes <= prior.attempts,
        "benchmark prior successes cannot exceed attempts"
    );
    connection.execute(
        r#"INSERT INTO benchmark_priors(
                source, dataset, dataset_version, harness, model, language,
                task_kind, scope, successes, attempts, updated_at, origin
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
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
            origin,
        ],
    )?;
    Ok(())
}

fn prior_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<BenchmarkPrior> {
    Ok(BenchmarkPrior {
        source: row.get(0)?,
        dataset: row.get(1)?,
        dataset_version: row.get(2)?,
        harness: row.get(3)?,
        model: row.get(4)?,
        language: row.get(5)?,
        task_kind: parse_task_kind(row.get(6)?)?,
        scope: parse_task_scope(row.get(7)?)?,
        successes: row.get(8)?,
        attempts: row.get(9)?,
        updated_at: timestamp_from_sql(10, row.get(10)?)?,
    })
}

fn parse_task_kind(value: String) -> rusqlite::Result<crate::TaskKind> {
    match value.as_str() {
        "bug_fix" => Ok(crate::TaskKind::BugFix),
        "feature" => Ok(crate::TaskKind::Feature),
        "refactor" => Ok(crate::TaskKind::Refactor),
        "tests" => Ok(crate::TaskKind::Tests),
        "unknown" => Ok(crate::TaskKind::Unknown),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn parse_task_scope(value: String) -> rusqlite::Result<crate::TaskScope> {
    match value.as_str() {
        "localized" => Ok(crate::TaskScope::Localized),
        "multi_file" => Ok(crate::TaskScope::MultiFile),
        "broad" => Ok(crate::TaskScope::Broad),
        "unknown" => Ok(crate::TaskScope::Unknown),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
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

fn validate_terminal_write(
    connection: &Connection,
    run: &RunRecord,
    explicit_review: bool,
) -> Result<()> {
    if run.phase3.is_some() {
        let mut statement = connection.prepare("SELECT id,details_json FROM attempts WHERE run_id=?1 AND completed_at IS NOT NULL AND details_json IS NOT NULL")?;
        for row in statement.query_map([&run.id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })? {
            let (id, original) = row?;
            let attempt = run
                .attempts
                .iter()
                .find(|a| a.id == id)
                .context("completed attempt cannot be removed")?;
            anyhow::ensure!(
                serde_json::to_string(attempt)? == original,
                "completed attempt evidence is immutable"
            );
        }
    }
    let old: Option<String> = connection
        .query_row(
            "SELECT outcome_json FROM runs WHERE id=?1",
            [&run.id],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(old) = old {
        let old: crate::RunOutcome = serde_json::from_str(&old)?;
        anyhow::ensure!(
            old.work_result != crate::WorkResult::Cancelled
                || (run.outcome.work_result == crate::WorkResult::Cancelled
                    && run.outcome.lifecycle == crate::LifecycleState::Finished),
            "cancelled work cannot reopen"
        );
        anyhow::ensure!(
            old.review != crate::ReviewState::Rejected
                || run.outcome.review == crate::ReviewState::Rejected
                || (explicit_review && run.outcome.lifecycle == crate::LifecycleState::Finished),
            "rejected work cannot automatically reopen"
        );
    }
    Ok(())
}

fn persist_questions(connection: &Connection, run: &RunRecord) -> Result<()> {
    let Some(policy) = &run.phase3 else {
        return Ok(());
    };
    for question in &policy.questions {
        anyhow::ensure!(question.run_id == run.id, "question belongs to another run");
        let json = serde_json::to_string(question)?;
        let existing: Option<String> = connection
            .query_row(
                "SELECT question_json FROM clarifications WHERE id=?1",
                [&question.id],
                |row| row.get(0),
            )
            .optional()?;
        if existing.as_deref() == Some(&json) {
            continue;
        }
        let state = match question.state {
            crate::QuestionState::Pending => "pending",
            crate::QuestionState::Answered => "answered",
            crate::QuestionState::Cancelled => "cancelled",
        };
        if let Some(existing) = existing {
            let old: crate::Clarification = serde_json::from_str(&existing)?;
            anyhow::ensure!(
                old.run_id == run.id
                    && old.attempt_id == question.attempt_id
                    && old.generation == question.generation
                    && old.report == question.report
                    && old.state == crate::QuestionState::Pending
                    && question.state != crate::QuestionState::Pending
                    && question.revision == old.revision + 1
                    && question.actor_uid == Some(policy.owner_uid),
                "stale or unauthorized question mutation"
            );
            let changed = connection.execute("UPDATE clarifications SET state=?2,revision=?3,question_json=?4 WHERE id=?1 AND run_id=?5 AND revision=?6 AND state='pending'",params![question.id,state,question.revision,json,run.id,old.revision])?;
            anyhow::ensure!(changed == 1, "question changed concurrently");
        } else {
            anyhow::ensure!(
                question.state == crate::QuestionState::Pending && question.revision == 1,
                "new question must be pending"
            );
            let active: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM admission_requests WHERE run_id=?1 AND status IN ('queued','admitted','reconciliation'))",[&run.id],|r|r.get(0))?;
            anyhow::ensure!(
                !active,
                "cannot wait on a human while admission is unresolved"
            );
            connection.execute(
                "INSERT INTO clarifications VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    question.id,
                    run.id,
                    question.attempt_id,
                    question.generation,
                    question.revision,
                    state,
                    json
                ],
            )?;
        }
    }
    Ok(())
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
            phase3: None,
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
            mode: crate::RunMode::Legacy,
            state_revision: 0,
            outcome: crate::RunOutcome::default(),
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
            attempts: Vec::new(),
            routing: None,
            allocation: None,
            capacity: None,
            admission: None,
            coherence: None,
            evaluation: None,
            applied_candidate: None,
            attachment: None,
        }
    }

    fn insert_legacy_run(connection: &Connection, id: &str) -> Result<()> {
        connection.execute(
            "INSERT INTO sources(path, kind, fingerprint, created_at) VALUES ('/legacy', 'directory', 'legacy', ?1)",
            [timestamp(at(1))],
        )?;
        connection.execute(
            r#"INSERT INTO runs(
                    id, source_id, task, exact_prompt, baseline_path, baseline_commit, status,
                    created_at, dispatch_version, os, architecture, execution_backend,
                    timeout_secs, cpus, memory, max_parallel
                ) VALUES (?1, 1, 'task', 'prompt', '/baseline', 'commit',
                          'ready_for_evaluation', ?2, '0.1.0', 'test', 'test', 'local',
                          30, 1.0, '1g', 1)"#,
            params![id, timestamp(at(1))],
        )?;
        Ok(())
    }

    #[test]
    fn future_schema_is_refused_without_migration_or_backup() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("state.db");
        let db = Database::open(&path)?;
        db.connection.execute(
            "INSERT INTO schema_migrations VALUES(22,'future','fixture')",
            [],
        )?;
        drop(db);
        assert!(
            Database::open(&path)
                .err()
                .unwrap()
                .to_string()
                .contains("newer than this binary")
        );
        assert_eq!(Database::open_read_only(&path)?.schema_version()?, 22);
        assert!(!fs::read_dir(temp.path())?.any(|e| {
            e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("dispatch.schema")
        }));
        Ok(())
    }

    #[test]
    fn schema19_projection_and_direct_grant_survive_planning_migration() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let path = tmp.path().join("prior.db");
        let connection = Connection::open(&path)?;
        connection.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,name TEXT NOT NULL,applied_at TEXT NOT NULL);")?;
        for (version, name, sql) in MIGRATIONS.iter().filter(|(v, _, _)| *v <= 19) {
            connection.execute_batch(sql)?;
            connection.execute(
                "INSERT INTO schema_migrations VALUES (?1,?2,?3)",
                params![version, name, timestamp(at(1))],
            )?;
            if *version == 12 {
                insert_legacy_run(&connection, "prior")?;
            }
        }
        let original = serde_json::to_string(&run("prior"))?;
        connection.execute(
            "UPDATE runs SET run_projection_json=?1 WHERE id='prior'",
            [&original],
        )?;
        let grant = json!({"principal":"old","owner_uid":0,"source":"/project","state_root":"/state","config_digest":"old","profiles":[],"timeout_secs":30,"max_invocations":2,"allow_unsafe_local":true,"delegate_factual":false,"expires_at":at(4)});
        connection.execute(
            "INSERT INTO control_grants VALUES ('old','hash',?1)",
            [grant.to_string()],
        )?;
        drop(connection);
        let database = Database::open(&path)?;
        assert_eq!(database.schema_version()?, 21);
        let backups = fs::read_dir(tmp.path())?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("dispatch.schema-19-")
            })
            .collect::<Vec<_>>();
        assert_eq!(backups.len(), 1);
        let backup = Database::open_read_only(backups[0].path())?;
        assert_eq!(backup.schema_version()?, 19);
        assert_eq!(
            backup.connection.query_row(
                "SELECT run_projection_json FROM runs WHERE id='prior'",
                [],
                |r| r.get::<_, String>(0)
            )?,
            original
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(backups[0].metadata()?.permissions().mode() & 0o777, 0o600);
        }
        drop(backup);
        drop(Database::open(&path)?);
        assert_eq!(
            fs::read_dir(tmp.path())?
                .filter_map(|e| e.ok())
                .filter(|e| e
                    .file_name()
                    .to_string_lossy()
                    .starts_with("dispatch.schema-19-"))
                .count(),
            1
        );
        assert_eq!(
            database.connection.query_row(
                "SELECT run_projection_json FROM runs WHERE id='prior'",
                [],
                |r| r.get::<_, String>(0)
            )?,
            original
        );
        assert!(
            database
                .committed_run_projection("prior")?
                .unwrap()
                .phase3
                .is_none()
        );
        let scope: crate::commands::Scope = serde_json::from_value(grant)?;
        assert!(!scope.allow_plan);
        assert_eq!(
            database
                .connection
                .query_row("SELECT COUNT(*) FROM planned_goals", [], |r| r
                    .get::<_, u32>(0))?,
            0
        );
        Ok(())
    }

    /// S1 (`attached_work_mode`): a schema-20 state directory with a run that
    /// has rows in `attempts`, `control_runs` and `planned_goals` (the three
    /// tables the migration-13-style rebuild of `runs` must not orphan)
    /// upgrades cleanly to schema 21, keeps the index and trigger migration
    /// 19 added on `runs`, and a fresh `run_mode = 'attached'` row can be
    /// written and read back afterward.
    #[test]
    fn schema20_run_with_child_rows_survives_attached_mode_migration() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("schema20.db");
        let connection = Connection::open(&path)?;
        connection.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at TEXT NOT NULL);")?;
        for (version, name, sql) in MIGRATIONS.iter().filter(|(v, _, _)| *v <= 20) {
            if *version == 13 {
                connection.pragma_update(None, "foreign_keys", false)?;
            }
            connection.execute_batch(sql)?;
            connection.execute(
                "INSERT INTO schema_migrations VALUES (?1,?2,?3)",
                params![version, name, timestamp(at(1))],
            )?;
            if *version == 13 {
                connection.pragma_update(None, "foreign_keys", true)?;
            }
            if *version == 12 {
                insert_legacy_run(&connection, "pre-attach")?;
            }
        }
        connection.execute_batch("PRAGMA foreign_keys = ON;")?;
        connection.execute(
            "INSERT INTO attempts(id,run_id,candidate_id,role,ordinal,generation,harness_id,\
             started_at,outcome,raw_telemetry_path) VALUES \
             ('attempt-pre','pre-attach','candidate','executor',1,1,'codex',?1,'completed','telemetry')",
            [timestamp(at(1))],
        )?;
        connection.execute(
            "INSERT INTO control_grants VALUES ('grant-pre','hash-pre','{}')",
            [],
        )?;
        connection.execute(
            "INSERT INTO control_runs VALUES ('pre-attach','grant-pre','session-pre',0)",
            [],
        )?;
        connection.execute(
            "INSERT INTO planned_goals VALUES ('pre-attach','revision-pre','{}',NULL)",
            [],
        )?;
        let pre_attach_projection = serde_json::to_string(&run("pre-attach"))?;
        connection.execute(
            "UPDATE runs SET run_projection_json=?1 WHERE id='pre-attach'",
            [&pre_attach_projection],
        )?;
        drop(connection);

        let mut migrated = Database::open(&path)?;
        assert_eq!(migrated.schema_version()?, 21);
        let backups = fs::read_dir(temp.path())?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("dispatch.schema-20-")
            })
            .count();
        assert_eq!(backups, 1);

        // Every foreign key into `runs`, across every referencing table,
        // survived the rename/recreate/drop rebuild.
        let violations: i64 = migrated.connection.query_row(
            "SELECT COUNT(*) FROM pragma_foreign_key_check",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(violations, 0);
        assert_eq!(
            migrated.connection.query_row(
                "SELECT outcome FROM attempts WHERE id='attempt-pre'",
                [],
                |row| row.get::<_, String>(0)
            )?,
            "completed"
        );
        assert_eq!(
            migrated.connection.query_row(
                "SELECT session_id FROM control_runs WHERE run_id='pre-attach'",
                [],
                |row| row.get::<_, String>(0)
            )?,
            "session-pre"
        );
        assert_eq!(
            migrated.connection.query_row(
                "SELECT revision FROM planned_goals WHERE run_id='pre-attach'",
                [],
                |row| row.get::<_, String>(0)
            )?,
            "revision-pre"
        );

        // The index and the private-evidence-immutability trigger migration
        // 19 added on `runs` are not silently dropped by the rebuild.
        assert!(migrated.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name='private_runs_source_window')",
            [],
            |row| row.get::<_, bool>(0)
        )?);
        assert!(migrated.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='trigger' AND name='private_decision_immutable')",
            [],
            |row| row.get::<_, bool>(0)
        )?);

        // The pre-migration run has no attachment: an old `run.json`/
        // projection with no `attachment` key deserializes to `None`.
        let old_run = migrated.committed_run_projection("pre-attach")?.unwrap();
        assert!(old_run.attachment.is_none());
        assert_eq!(old_run.mode, RunMode::Legacy);

        // A `run_mode = 'attached'` row can be written and read back through
        // the ordinary `sync_run` path now that the CHECK is widened.
        let mut attached = run("attached-run");
        attached.mode = RunMode::Attached;
        attached.source_kind = SourceKind::GitWorktree;
        attached.attachment = Some(crate::AttachmentRecord {
            version: 1,
            workspace: PathBuf::from("/repo-worktree"),
            integration_root: PathBuf::from("/repo"),
            repo_key: Some("sha256:deadbeef".into()),
            provenance: crate::BaselineProvenance::GitMergeBase {
                commit: "0123456789abcdef".into(),
            },
            confidence: crate::AttachConfidence::Full,
            agent: Some("claude".into()),
            command: Some(vec!["claude".into()]),
            owner: None,
            agent_process: None,
            owner_state: crate::OwnerState::Live,
            capabilities: crate::AttachCapabilities {
                observe: true,
                signal: true,
                control: true,
                integrate: false,
            },
            attached_at: at(1),
            finished_at: None,
            finish_reason: None,
        });
        migrated.sync_run(&attached)?;
        assert_eq!(
            migrated.connection.query_row(
                "SELECT run_mode FROM runs WHERE id='attached-run'",
                [],
                |row| row.get::<_, String>(0)
            )?,
            "attached"
        );
        let restored = migrated.committed_run_projection("attached-run")?.unwrap();
        assert_eq!(restored.mode, RunMode::Attached);
        assert_eq!(restored.attachment, attached.attachment);
        Ok(())
    }

    #[test]
    fn applies_migration_and_enables_foreign_keys() -> Result<()> {
        let database = Database::open_in_memory()?;
        assert_eq!(database.schema_version()?, 21);
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
            "routing_feedback_events",
            "attempts",
            "allocation_decisions",
            "goal_feedback_revisions",
            "resource_pools",
            "capacity_observations",
            "capacity_authorizations",
            "capacity_observation_constraints",
            "admission_requests",
            "pool_leases",
            "clarifications",
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
    fn capacity_observations_append_without_replacing_prior_samples() -> Result<()> {
        let database = Database::open_in_memory()?;
        database.connection.execute(
            "INSERT INTO resource_pools(id,provider,funding_source,provider_buckets_json,max_active,next_fence,created_at,updated_at) VALUES('pool','openai','chatgpt-plus','[\"codex\"]',1,0,?1,?1)",
            [timestamp(at(1))],
        )?;
        let first = crate::capacity::unknown_observation("pool", "default", at(2), 300, "first");
        let second = crate::capacity::unknown_observation("pool", "default", at(3), 300, "second");
        database.append_capacity_observation(&first)?;
        database.append_capacity_observation(&second)?;
        let rows: i64 = database.connection.query_row(
            "SELECT COUNT(*) FROM capacity_observations WHERE pool_id='pool'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(rows, 2);
        assert_eq!(
            database.latest_capacity_observation("pool")?.unwrap().id,
            second.id
        );
        assert!(database.append_capacity_observation(&first).is_err());
        Ok(())
    }

    #[test]
    fn uncertain_cleanup_and_release_survive_equal_and_stale_projection_writes() -> Result<()> {
        for uncertain in [false, true] {
            let temp = tempfile::tempdir()?;
            let state = crate::state::State::discover(Some(temp.path().join("state")))?;
            state.initialize()?;
            let mut database = Database::open(state.db_path())?;
            let mut current = run(&ulid::Ulid::new().to_string());
            current.state_revision = 3;
            database.sync_run(&current)?;
            let coordinator = crate::admission::AdmissionCoordinator::new(
                state.db_path(),
                Duration::from_secs(20),
                Duration::from_secs(1),
            );
            coordinator.register_pool("pool", "openai", "chatgpt-plus", &["codex".into()])?;
            let request = coordinator.enqueue(&current.id, "pool", "owner", 1, 0)?;
            let crate::admission::AcquireResult::Acquired(token) =
                coordinator.try_acquire(&request)?
            else {
                panic!("not admitted")
            };
            current.admission = database.admission_summary_for_run(&current.id)?;
            database.sync_run(&current)?;
            let stale = current.clone();
            let expected = if uncertain {
                database.connection.execute("UPDATE pool_leases SET launch_lifecycle='spawn_may_have_occurred' WHERE pool_id='pool'",[])?;
                coordinator.cleanup_after_unrecorded_spawn(
                    &token,
                    &crate::admission::ProcessIdentity::current(),
                    None,
                    false,
                )?;
                let states:(String,String)=database.connection.query_row("SELECT l.state,r.status FROM pool_leases l JOIN admission_requests r ON r.id=l.request_id",[],|r|Ok((r.get(0)?,r.get(1)?)))?;
                assert_eq!(states, ("reconciliation".into(), "reconciliation".into()));
                assert!(!coordinator.release_not_launched(&token)?);
                crate::AdmissionState::Reconciliation
            } else {
                assert!(coordinator.release(&token)?);
                crate::AdmissionState::Released
            };
            // Real production projection writer must overlay SQL authority,
            // even when handed an equal-revision stale in-memory snapshot.
            database.sync_run(&stale)?;
            assert_eq!(
                database
                    .committed_run_projection(&current.id)?
                    .unwrap()
                    .admission
                    .unwrap()
                    .state,
                expected
            );
            database.commit_transition(
                &mut current,
                EventRecord {
                    run_id: stale.id.clone(),
                    event_type: "projection.test".into(),
                    timestamp: Utc::now(),
                    ..EventRecord::default()
                },
            )?;
            assert_eq!(
                crate::orchestrator::run_result(&current)
                    .admission
                    .as_ref()
                    .unwrap()
                    .state,
                expected
            );
            for revision in [3, 4, 5] {
                let mut stale_file = stale.clone();
                stale_file.state_revision = revision;
                state.save_run(&stale_file)?;
                let public = state.load_run(&current.id)?;
                assert_eq!(public.admission.as_ref().unwrap().state, expected);
                let json = serde_json::to_value(public)?;
                assert_eq!(
                    json["admission"]["state"],
                    if uncertain {
                        "reconciliation"
                    } else {
                        "released"
                    }
                );
            }
        }
        Ok(())
    }

    #[test]
    fn stale_run_projection_cannot_overwrite_newer_revision() -> Result<()> {
        let mut database = Database::open_in_memory()?;
        let mut current = run("projection-fence");
        database.sync_run(&current)?;
        let stale = current.clone();
        current.state_revision = 2;
        current.status = RunStatus::Failed;
        database.sync_run(&current)?;
        assert!(database.sync_run(&stale).is_err());
        let stored = database.committed_run_projection(&current.id)?.unwrap();
        assert_eq!(stored.state_revision, 2);
        assert_eq!(stored.status, RunStatus::Failed);
        Ok(())
    }

    #[test]
    fn migration_preserves_live_orphans_from_schema_14_and_already_applied_15() -> Result<()> {
        for version in [14, 15] {
            let temp = tempfile::tempdir()?;
            let path = temp.path().join("old.db");
            let connection = Connection::open(&path)?;
            connection.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at TEXT NOT NULL);")?;
            for (number, name, sql) in MIGRATIONS.iter().take(12) {
                connection.execute_batch(sql)?;
                connection.execute(
                    "INSERT INTO schema_migrations VALUES(?1,?2,?3)",
                    params![number, name, timestamp(at(1))],
                )?;
            }
            insert_legacy_run(&connection, "old-run")?;
            for (number, name, sql) in MIGRATIONS.iter().take(version).skip(12) {
                connection.execute_batch(sql)?;
                connection.execute(
                    "INSERT INTO schema_migrations VALUES(?1,?2,?3)",
                    params![number, name, timestamp(at(1))],
                )?;
            }
            connection.execute_batch("INSERT INTO resource_pools(id,provider,funding_source,provider_buckets_json,max_active,next_fence,created_at,updated_at) VALUES('pool','openai','chatgpt-plus','[\"codex\"]',1,1,'2020-01-01','2020-01-01');
                INSERT INTO admission_requests(id,run_id,pool_id,owner_session,generation,priority,enqueued_at,heartbeat_at,expires_at,status) VALUES('request','old-run','pool','dead-owner',1,0,'2020-01-01T00:00:00Z','2020-01-01T00:00:00Z','2020-01-01T00:00:00Z','admitted');")?;
            let child = crate::admission::ProcessIdentity::current();
            connection.execute("INSERT INTO pool_leases(pool_id,request_id,owner_session,generation,fence,state,start_intent_at,heartbeat_at,expires_at,owner_pid,child_pid,child_start_identity,child_boot_identity,child_process_group) VALUES('pool','request','dead-owner',1,1,'running',?1,?1,?1,4294967295,?2,?3,?4,?5)",params![timestamp(at(1)),child.pid,child.start,child.boot,child.process_group])?;
            drop(connection);
            let migrated = Database::open(&path)?;
            crate::admission::AdmissionCoordinator::new(
                &path,
                Duration::from_secs(10),
                Duration::from_secs(1),
            )
            .reconcile_expired("pool")?;
            let lifecycle: String = migrated.connection.query_row(
                "SELECT launch_lifecycle FROM pool_leases WHERE pool_id='pool'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(lifecycle, "cleanup_uncertain");
            assert_eq!(
                migrated
                    .admission_summary_for_run("old-run")?
                    .unwrap()
                    .state,
                crate::AdmissionState::Reconciliation
            );
            assert_eq!(Database::open(&path)?.schema_version()?, 21);
        }
        Ok(())
    }

    #[test]
    fn phase_seven_migration_preserves_feedback_and_unknown_history() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("phase-six.db");
        let connection = Connection::open(&path)?;
        connection.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,name TEXT NOT NULL,applied_at TEXT NOT NULL);")?;
        for (version, name, sql) in MIGRATIONS.iter().take(18) {
            connection.execute_batch(sql)?;
            connection.execute(
                "INSERT INTO schema_migrations VALUES (?1,?2,?3)",
                params![version, name, timestamp(at(1))],
            )?;
            if *version == 12 {
                insert_legacy_run(&connection, "phase-six-history")?;
            }
        }
        for revision in 1..=2 {
            connection.execute("INSERT INTO goal_feedback_revisions VALUES (?1,'phase-six-history',?2,'accepted','[]',NULL,?3)",params![format!("retained-{revision}"),revision,timestamp(at(revision))])?;
        }
        drop(connection);
        let migrated = Database::open(&path)?;
        assert_eq!(migrated.schema_version()?, 21);
        assert_eq!(
            migrated
                .latest_goal_feedback("phase-six-history")?
                .unwrap()
                .revision,
            2
        );
        assert_eq!(
            migrated.connection.query_row(
                "SELECT COUNT(*) FROM goal_feedback_revisions",
                [],
                |r| r.get::<_, u64>(0)
            )?,
            2
        );
        assert_eq!(
            migrated
                .connection
                .query_row("SELECT COUNT(*) FROM private_annotations", [], |r| r
                    .get::<_, u64>(0))?,
            0
        );
        assert_eq!(
            migrated.connection.query_row(
                "SELECT COUNT(*) FROM private_policy_transitions",
                [],
                |r| r.get::<_, u64>(0)
            )?,
            0
        );
        assert_eq!(
            migrated.connection.query_row(
                "SELECT COUNT(*) FROM pragma_foreign_key_check",
                [],
                |r| r.get::<_, u64>(0)
            )?,
            0
        );
        drop(migrated);
        assert_eq!(Database::open(&path)?.schema_version()?, 21);
        Ok(())
    }

    #[test]
    fn phase_three_migration_preserves_phase_two_rows() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("phase-two.db");
        let connection = Connection::open(&path)?;
        connection.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,name TEXT NOT NULL,applied_at TEXT NOT NULL);")?;
        for (version, name, sql) in MIGRATIONS.iter().take(16) {
            if *version == 13 {
                connection.pragma_update(None, "foreign_keys", false)?;
            }
            connection.execute_batch(sql)?;
            connection.execute(
                "INSERT INTO schema_migrations VALUES (?1,?2,?3)",
                params![version, name, timestamp(at(1))],
            )?;
            if *version == 12 {
                insert_legacy_run(&connection, "phase-two")?;
            }
        }
        connection.execute("INSERT INTO attempts(id,run_id,candidate_id,role,ordinal,generation,harness_id,started_at,outcome,raw_telemetry_path) VALUES ('attempt','phase-two','candidate','executor',1,1,'codex',?1,'completed','telemetry')",[timestamp(at(1))])?;
        drop(connection);
        let migrated = Database::open(&path)?;
        assert_eq!(migrated.schema_version()?, 21);
        assert_eq!(
            migrated.connection.query_row(
                "SELECT outcome FROM attempts WHERE id='attempt'",
                [],
                |r| r.get::<_, String>(0)
            )?,
            "completed"
        );
        assert!(
            migrated
                .connection
                .query_row(
                    "SELECT details_json FROM attempts WHERE id='attempt'",
                    [],
                    |r| r.get::<_, Option<String>>(0)
                )?
                .is_none()
        );
        assert_eq!(
            migrated
                .connection
                .query_row("SELECT COUNT(*) FROM clarifications", [], |r| r
                    .get::<_, i64>(0))?,
            0
        );
        migrated.health_check()
    }

    #[test]
    fn allocation_migration_preserves_phase_zero_rows_and_foreign_keys() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("phase-zero.db");
        let connection = Connection::open(&path)?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON; CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at TEXT NOT NULL);",
        )?;
        for (version, name, sql) in MIGRATIONS.iter().take(12) {
            connection.execute_batch(sql)?;
            connection.execute(
                "INSERT INTO schema_migrations VALUES (?1, ?2, ?3)",
                params![version, name, timestamp(at(1))],
            )?;
        }
        insert_legacy_run(&connection, "phase-zero-run")?;
        connection.execute(
            "INSERT INTO events(run_id, event_type, timestamp, payload_json) VALUES ('phase-zero-run', 'legacy', ?1, '{}')",
            [timestamp(at(2))],
        )?;
        drop(connection);

        let migrated = Database::open(&path)?;
        assert_eq!(migrated.schema_version()?, 21);
        let violations: i64 = migrated.connection.query_row(
            "SELECT COUNT(*) FROM pragma_foreign_key_check",
            [],
            |row| row.get(0),
        )?;
        let event_count: i64 = migrated.connection.query_row(
            "SELECT COUNT(*) FROM events WHERE run_id = 'phase-zero-run'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!((violations, event_count), (0, 1));
        Ok(())
    }

    #[test]
    fn opens_file_database_and_creates_parent_directories() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("nested/state/dispatch.db");
        let database = Database::open(&path)?;
        assert!(path.is_file());
        assert_eq!(database.schema_version()?, 21);
        assert_eq!(
            database
                .connection
                .pragma_query_value(None, "synchronous", |row| row.get::<_, i64>(0))?,
            2
        );
        drop(database);

        // Opening an already-migrated database is idempotent.
        assert_eq!(Database::open(&path)?.schema_version()?, 21);
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
        insert_legacy_run(&connection, "run-legacy-sync")?;
        let mut legacy = Database { connection };
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
    fn feedback_migration_preserves_typed_outbox_bytes_status_and_retry_metadata() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("v9.db");
        let connection = Connection::open(&path)?;
        connection.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at TEXT NOT NULL);")?;
        for (version, name, sql) in MIGRATIONS.iter().take(9) {
            connection.execute_batch(sql)?;
            connection.execute(
                "INSERT INTO schema_migrations VALUES (?1, ?2, ?3)",
                params![version, name, timestamp(at(1))],
            )?;
        }
        insert_legacy_run(&connection, "migration-run")?;
        let mut previous = Database { connection };
        previous.enable_sync("migration-contributor", at(6))?;
        for status in ["pending", "failed", "conflict", "synced"] {
            for record_type in ["evaluation-v1", "routing-observation-v1"] {
                previous.connection.execute(
                    "INSERT INTO sync_outbox VALUES (?1, 'migration-run', ?2, ?3, ?4, 'attempt', 'error', 'synced-at')",
                    params![format!("{record_type}-{status}"), record_type, format!("{{\n  \"state\": \"{status}\"\n}}"), status],
                )?;
            }
        }
        let settings = previous.sync_settings()?;
        drop(previous);
        let migrated = Database::open(&path)?;
        assert_eq!(migrated.schema_version()?, 21);
        assert_eq!(migrated.sync_settings()?, settings);
        for status in ["pending", "failed", "conflict", "synced"] {
            for record_type in ["evaluation-v1", "routing-observation-v1"] {
                let row: (String, String, String, String, String) = migrated.connection.query_row(
                    "SELECT payload_json, status, last_attempt, last_error, synced_at FROM sync_outbox WHERE record_id = ?1",
                    [format!("{record_type}-{status}")], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
                )?;
                assert_eq!(
                    row,
                    (
                        format!("{{\n  \"state\": \"{status}\"\n}}"),
                        status.into(),
                        "attempt".into(),
                        "error".into(),
                        "synced-at".into()
                    )
                );
            }
        }
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
            ..EventRecord::default()
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
