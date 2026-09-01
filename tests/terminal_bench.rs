use std::{fs, path::PathBuf};

use assert_cmd::cargo_bin_cmd;
use dispatch::{
    TaskFeatures, TaskKind, TaskScope,
    datasets::{import_terminal_bench, read_terminal_bench_snapshot},
    db::Database,
    router::rank_harnesses,
    state::State,
};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/terminal-bench/minimal")
}

fn state(temp: &tempfile::TempDir) -> State {
    State {
        root: temp.path().join("state"),
    }
}

fn unknown_features() -> TaskFeatures {
    TaskFeatures {
        language: None,
        task_kind: TaskKind::Unknown,
        scope: TaskScope::Unknown,
    }
}

fn copy_fixture(destination: &std::path::Path) -> anyhow::Result<()> {
    fs::create_dir_all(destination)?;
    fs::copy(
        fixture().join("config.json"),
        destination.join("config.json"),
    )?;
    for trial in ["fix-permissions__attempt-1", "git-recovery__attempt-1"] {
        fs::create_dir_all(destination.join(trial))?;
        fs::copy(
            fixture().join(trial).join("result.json"),
            destination.join(trial).join("result.json"),
        )?;
    }
    Ok(())
}

fn edit_json(
    path: &std::path::Path,
    edit: impl FnOnce(&mut serde_json::Value),
) -> anyhow::Result<()> {
    let mut value = serde_json::from_slice(&fs::read(path)?)?;
    edit(&mut value);
    fs::write(path, serde_json::to_vec_pretty(&value)?)?;
    Ok(())
}

fn make_cursor_fixture(destination: &std::path::Path) -> anyhow::Result<()> {
    copy_fixture(destination)?;
    edit_json(&destination.join("config.json"), |value| {
        value["agents"][0]["name"] = "cursor-cli".into();
    })?;
    for trial in ["fix-permissions__attempt-1", "git-recovery__attempt-1"] {
        edit_json(&destination.join(trial).join("result.json"), |value| {
            value["agent_info"]["name"] = "cursor-cli".into();
            value["agent_info"]["version"] = "2026.08".into();
        })?;
    }
    Ok(())
}

#[test]
fn parses_minimal_real_shaped_terminal_bench_job() -> anyhow::Result<()> {
    let snapshot = read_terminal_bench_snapshot(&fixture())?;

    assert_eq!(snapshot.observations.len(), 2);
    assert_eq!(snapshot.observations[0].task_id, "fix-permissions");
    assert_eq!(
        snapshot.observations[0].trial_id,
        "11111111-1111-4111-8111-111111111111"
    );
    assert_eq!(snapshot.observations[0].reward, 1.0);
    assert!(snapshot.observations[0].succeeded);
    assert_eq!(snapshot.observations[1].reward, 0.0);
    assert!(!snapshot.observations[1].succeeded);
    Ok(())
}

#[test]
fn preserves_dataset_agent_version_and_model_identities() -> anyhow::Result<()> {
    let snapshot = read_terminal_bench_snapshot(&fixture())?;

    assert_eq!(snapshot.source, "harbor-framework/harbor");
    assert_eq!(snapshot.dataset, "terminal-bench/terminal-bench-2");
    assert!(snapshot.dataset_version.starts_with("2.0+sha256:"));
    assert_eq!(snapshot.agent, "codex");
    assert_eq!(snapshot.agent_version, "0.42.0");
    assert_eq!(snapshot.harness, "codex");
    assert_eq!(snapshot.model.as_deref(), Some("openai/gpt-5"));
    assert_ne!(snapshot.agent, snapshot.model.as_deref().unwrap());
    Ok(())
}

#[test]
fn aggregates_trials_and_caches_the_exact_normalization_inputs() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let state = state(&temp);

    let imported = import_terminal_bench(&state, &fixture())?;

    assert_eq!((imported.prior.successes, imported.prior.attempts), (1, 2));
    assert_eq!(imported.prior.source, "harbor-framework/harbor");
    assert_eq!(imported.prior.dataset, "terminal-bench/terminal-bench-2");
    assert_eq!(imported.prior.harness, "codex");
    assert_eq!(imported.prior.model.as_deref(), Some("openai/gpt-5"));
    assert_eq!(imported.prior.language, None);
    assert_eq!(imported.prior.task_kind, TaskKind::Unknown);
    assert_eq!(imported.prior.scope, TaskScope::Unknown);
    assert_eq!(
        fs::read(imported.raw_snapshot_path.join("config.json"))?,
        fs::read(fixture().join("config.json"))?
    );
    assert_eq!(
        fs::read(
            imported
                .raw_snapshot_path
                .join("fix-permissions__attempt-1/result.json")
        )?,
        fs::read(fixture().join("fix-permissions__attempt-1/result.json"))?
    );
    Ok(())
}

#[test]
fn identical_import_is_idempotent_and_new_snapshot_replaces_it() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let state = state(&temp);
    let first = import_terminal_bench(&state, &fixture())?;
    let repeated = import_terminal_bench(&state, &fixture())?;
    assert_eq!(first.raw_snapshot_path, repeated.raw_snapshot_path);

    let newer = temp.path().join("newer");
    copy_fixture(&newer)?;
    edit_json(
        &newer.join("git-recovery__attempt-1/result.json"),
        |value| value["verifier_result"]["rewards"]["reward"] = 1.into(),
    )?;
    let replacement = import_terminal_bench(&state, &newer)?;

    assert_ne!(first.raw_snapshot_path, replacement.raw_snapshot_path);
    assert_ne!(
        first.prior.dataset_version,
        replacement.prior.dataset_version
    );
    let database = Database::open(state.db_path())?;
    let priors = database.matching_benchmark_priors(&unknown_features(), "codex")?;
    assert_eq!(priors.len(), 1);
    assert_eq!((priors[0].successes, priors[0].attempts), (2, 2));
    Ok(())
}

#[test]
fn incomplete_trial_fails_without_replacing_valid_evidence() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let state = state(&temp);
    import_terminal_bench(&state, &fixture())?;
    let malformed = temp.path().join("malformed");
    copy_fixture(&malformed)?;
    edit_json(
        &malformed.join("fix-permissions__attempt-1/result.json"),
        |value| value["verifier_result"]["rewards"] = serde_json::json!({}),
    )?;

    let error = import_terminal_bench(&state, &malformed).unwrap_err();

    assert!(error.to_string().contains("reward"));
    let database = Database::open(state.db_path())?;
    let priors = database.matching_benchmark_priors(&unknown_features(), "codex")?;
    assert_eq!(priors.len(), 1);
    assert_eq!((priors[0].successes, priors[0].attempts), (1, 2));
    Ok(())
}

#[test]
fn maps_only_explicitly_equivalent_harbor_agents() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;

    let cursor = temp.path().join("cursor");
    make_cursor_fixture(&cursor)?;
    let cursor_snapshot = read_terminal_bench_snapshot(&cursor)?;
    assert_eq!(cursor_snapshot.agent, "cursor-cli");
    assert_eq!(cursor_snapshot.harness, "cursor");

    let unknown = temp.path().join("unknown");
    copy_fixture(&unknown)?;
    edit_json(&unknown.join("config.json"), |value| {
        value["agents"][0]["name"] = "terminus-2".into();
    })?;
    for trial in ["fix-permissions__attempt-1", "git-recovery__attempt-1"] {
        edit_json(&unknown.join(trial).join("result.json"), |value| {
            value["agent_info"]["name"] = "terminus-2".into();
            value["agent_info"]["version"] = "1.0.0".into();
        })?;
    }
    let unknown_snapshot = read_terminal_bench_snapshot(&unknown)?;
    assert_eq!(unknown_snapshot.agent, "terminus-2");
    assert_eq!(unknown_snapshot.harness, "terminus-2");
    Ok(())
}

#[test]
fn router_consumes_harbor_codex_and_cursor_as_generic_fallback() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let state = state(&temp);
    import_terminal_bench(&state, &fixture())?;
    let cursor = temp.path().join("cursor");
    make_cursor_fixture(&cursor)?;
    import_terminal_bench(&state, &cursor)?;
    let database = Database::open(state.db_path())?;
    let available = vec!["cursor".to_owned(), "codex".to_owned()];

    let known = TaskFeatures {
        language: Some("rust".to_owned()),
        task_kind: TaskKind::BugFix,
        scope: TaskScope::Localized,
    };
    let ranked = rank_harnesses(&database, &known, &available)?;

    assert_eq!(ranked[0].harness, "codex");
    assert_eq!(ranked[0].score, Some(0.5));
    assert_eq!(ranked[0].evidence.as_ref().unwrap().specificity, 0);
    assert_eq!(
        ranked[0].evidence.as_ref().unwrap().prior.source,
        "harbor-framework/harbor"
    );
    assert_eq!(ranked[1].harness, "cursor");
    assert_eq!(ranked[1].score, Some(0.5));
    assert_eq!(ranked[1].evidence.as_ref().unwrap().specificity, 0);
    Ok(())
}

#[test]
fn datasets_import_cli_caches_terminal_bench_snapshot() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let state_root = temp.path().join("state");

    cargo_bin_cmd!("dispatch")
        .args(["--state-dir"])
        .arg(&state_root)
        .args(["datasets", "import", "terminal-bench"])
        .arg(fixture())
        .assert()
        .success()
        .stdout(predicates::str::contains("codex"));

    let database = Database::open(state_root.join("dispatch.db"))?;
    assert_eq!(
        database
            .matching_benchmark_priors(&unknown_features(), "codex")?
            .len(),
        1
    );
    Ok(())
}
