use std::{fs, path::PathBuf};

use assert_cmd::cargo_bin_cmd;
use dispatch::{
    TaskFeatures, TaskKind, TaskScope,
    datasets::{import_swe_bench, read_swe_bench_snapshot},
    db::Database,
    router::rank_harnesses,
    state::State,
};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/swe-bench/minimal")
}

fn state(temp: &tempfile::TempDir) -> State {
    State {
        root: temp.path().join("state"),
    }
}

fn features() -> TaskFeatures {
    TaskFeatures {
        language: Some("python".to_owned()),
        task_kind: TaskKind::BugFix,
        scope: TaskScope::Unknown,
    }
}

fn copy_fixture(destination: &std::path::Path) -> anyhow::Result<()> {
    fs::create_dir_all(destination.join("results"))?;
    fs::copy(
        fixture().join("metadata.yaml"),
        destination.join("metadata.yaml"),
    )?;
    fs::copy(
        fixture().join("instances.jsonl"),
        destination.join("instances.jsonl"),
    )?;
    fs::copy(
        fixture().join("results/results.json"),
        destination.join("results/results.json"),
    )?;
    Ok(())
}

#[test]
fn parses_minimal_real_shaped_swe_bench_snapshot() -> anyhow::Result<()> {
    let snapshot = read_swe_bench_snapshot(&fixture())?;

    assert_eq!(snapshot.observations.len(), 2);
    assert_eq!(snapshot.observations[0].task_id, "django__django-11099");
    assert!(snapshot.observations[0].resolved);
    assert_eq!(snapshot.observations[1].task_id, "sympy__sympy-20590");
    assert!(!snapshot.observations[1].resolved);
    Ok(())
}

#[test]
fn preserves_benchmark_dataset_system_and_model_identities() -> anyhow::Result<()> {
    let snapshot = read_swe_bench_snapshot(&fixture())?;

    assert_eq!(snapshot.source, "swe-bench/experiments");
    assert_eq!(snapshot.dataset, "SWE-bench_Verified");
    assert!(snapshot.dataset_version.starts_with("sha256:"));
    assert_eq!(snapshot.harness, "mini-SWE-agent");
    assert_eq!(snapshot.model, "claude-3-7-sonnet-20250219");
    assert_ne!(snapshot.harness, snapshot.model);
    Ok(())
}

#[test]
fn aggregates_observations_into_existing_benchmark_prior() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let state = state(&temp);

    let imported = import_swe_bench(&state, &fixture())?;

    assert_eq!(imported.prior.successes, 1);
    assert_eq!(imported.prior.attempts, 2);
    assert_eq!(imported.prior.language.as_deref(), Some("python"));
    assert_eq!(imported.prior.task_kind, TaskKind::BugFix);
    assert_eq!(imported.prior.scope, TaskScope::Unknown);
    assert_eq!(
        imported.prior.dataset_version.strip_prefix("sha256:"),
        imported
            .raw_snapshot_path
            .file_name()
            .and_then(|name| name.to_str())
    );
    assert_eq!(
        fs::read(imported.raw_snapshot_path.join("metadata.yaml"))?,
        fs::read(fixture().join("metadata.yaml"))?
    );
    Ok(())
}

#[test]
fn importing_same_snapshot_twice_is_idempotent() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let state = state(&temp);

    let first = import_swe_bench(&state, &fixture())?;
    let second = import_swe_bench(&state, &fixture())?;

    assert_eq!(first.raw_snapshot_path, second.raw_snapshot_path);
    let database = Database::open(state.db_path())?;
    let priors = database.matching_benchmark_priors(&features(), "mini-SWE-agent")?;
    assert_eq!(priors.len(), 1);
    assert_eq!((priors[0].successes, priors[0].attempts), (1, 2));
    Ok(())
}

#[test]
fn newer_snapshot_replaces_cached_aggregate_deterministically() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let state = state(&temp);
    let newer = temp.path().join("newer");
    copy_fixture(&newer)?;
    fs::write(
        newer.join("instances.jsonl"),
        concat!(
            "{\"instance_id\":\"django__django-11099\"}\n",
            "{\"instance_id\":\"sympy__sympy-20590\"}\n",
            "{\"instance_id\":\"pytest-dev__pytest-10081\"}\n",
        ),
    )?;
    fs::write(
        newer.join("results/results.json"),
        r#"{"no_generation":[],"no_logs":[],"resolved":["django__django-11099","pytest-dev__pytest-10081"]}"#,
    )?;

    let original = import_swe_bench(&state, &fixture())?;
    let replacement = import_swe_bench(&state, &newer)?;

    assert_ne!(
        original.prior.dataset_version,
        replacement.prior.dataset_version
    );
    assert_ne!(original.raw_snapshot_path, replacement.raw_snapshot_path);
    let database = Database::open(state.db_path())?;
    let priors = database.matching_benchmark_priors(&features(), "mini-SWE-agent")?;
    assert_eq!(priors.len(), 1);
    assert_eq!((priors[0].successes, priors[0].attempts), (2, 3));
    Ok(())
}

#[test]
fn malformed_or_incomplete_records_fail_without_persisting() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let state = state(&temp);
    let malformed = temp.path().join("malformed");
    copy_fixture(&malformed)?;
    fs::write(
        malformed.join("results/results.json"),
        r#"{"resolved":["unknown__repo-1"]}"#,
    )?;

    let error = import_swe_bench(&state, &malformed).unwrap_err();

    assert!(error.to_string().contains("unknown__repo-1"));
    let database = Database::open(state.db_path())?;
    assert!(
        database
            .matching_benchmark_priors(&features(), "mini-SWE-agent")?
            .is_empty()
    );
    Ok(())
}

#[test]
fn missing_or_ambiguous_upstream_identity_fails_closed() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let missing_agent = temp.path().join("missing-agent");
    copy_fixture(&missing_agent)?;
    fs::write(
        missing_agent.join("metadata.yaml"),
        "tags:\n  model: [claude-3-7-sonnet-20250219]\n  system:\n    attempts: 1\n",
    )?;
    assert!(import_swe_bench(&state(&temp), &missing_agent).is_err());

    let multiple_models = temp.path().join("multiple-models");
    copy_fixture(&multiple_models)?;
    fs::write(
        multiple_models.join("metadata.yaml"),
        "tags:\n  agent: mini-SWE-agent\n  model: [model-a, model-b]\n  system:\n    attempts: 1\n",
    )?;
    assert!(import_swe_bench(&state(&temp), &multiple_models).is_err());
    Ok(())
}

#[test]
fn imported_prior_is_immediately_consumed_only_by_matching_harness() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let state = state(&temp);
    import_swe_bench(&state, &fixture())?;
    let database = Database::open(state.db_path())?;
    let available = vec!["codex".to_owned(), "mini-SWE-agent".to_owned()];

    let ranked = rank_harnesses(&database, &features(), &available)?;

    assert_eq!(ranked[0].harness, "mini-SWE-agent");
    assert_eq!(ranked[0].score, Some(0.5));
    assert_eq!(ranked[1].harness, "codex");
    assert_eq!(ranked[1].score, None);
    Ok(())
}

#[test]
fn datasets_import_cli_caches_swe_bench_snapshot() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let state_root = temp.path().join("state");

    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state_root)
        .args(["datasets", "import", "swe-bench"])
        .arg(fixture())
        .assert()
        .success()
        .stdout(predicates::str::contains("mini-SWE-agent"));

    let database = Database::open(state_root.join("dispatch.db"))?;
    assert_eq!(
        database
            .matching_benchmark_priors(&features(), "mini-SWE-agent")?
            .len(),
        1
    );
    Ok(())
}
