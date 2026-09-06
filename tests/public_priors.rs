use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    thread,
};

use assert_cmd::cargo_bin_cmd;
use chrono::{TimeZone, Utc};
use dispatch::{
    BenchmarkPrior, TaskFeatures, TaskKind, TaskScope,
    db::Database,
    public_priors::{PublicPriorSnapshotV1, bundled_snapshot},
    router::rank_harnesses,
    state::State,
};

fn prior(harness: &str, successes: u64, attempts: u64) -> BenchmarkPrior {
    BenchmarkPrior {
        source: "harbor-framework/harbor".to_owned(),
        dataset: "terminal-bench".to_owned(),
        dataset_version: "4.0".to_owned(),
        harness: harness.to_owned(),
        model: Some("openai/gpt-5.6-luna".to_owned()),
        language: None,
        task_kind: TaskKind::Unknown,
        scope: TaskScope::Unknown,
        successes,
        attempts,
        updated_at: Utc.with_ymd_and_hms(2026, 8, 26, 0, 0, 0).unwrap(),
    }
}

#[test]
fn maintainer_export_is_byte_stable_and_excludes_distributed_rows() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let state = State {
        root: temp.path().join("state"),
    };
    state.initialize()?;
    let mut database = Database::open(state.db_path())?;
    database.upsert_benchmark_prior(&prior("cursor", 29, 50))?;
    database.upsert_benchmark_prior(&prior("codex", 37, 50))?;
    database.replace_distributed_public_priors(&bundled_snapshot()?, "bundled")?;
    drop(database);
    let first = temp.path().join("first.json");
    let second = temp.path().join("second.json");

    for output in [&first, &second] {
        cargo_bin_cmd!("dispatch")
            .args([
                "--state-dir",
                state.root.to_str().unwrap(),
                "datasets",
                "export-public-priors",
                output.to_str().unwrap(),
            ])
            .assert()
            .success();
    }

    let first_bytes = fs::read(&first)?;
    assert_eq!(first_bytes, fs::read(&second)?);
    let snapshot = PublicPriorSnapshotV1::from_json_bytes(&first_bytes)?;
    assert_eq!(snapshot.entries.len(), 2);
    assert_eq!(snapshot.entries[0].harness, "codex");
    assert_eq!(snapshot.entries[1].harness, "cursor");
    Ok(())
}

#[test]
fn refresh_replaces_only_compact_distributed_priors() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let state = State {
        root: temp.path().join("state"),
    };
    let replacement = PublicPriorSnapshotV1::from_priors(vec![prior("cursor", 29, 50)])?;
    let url = serve_once(replacement.to_json_bytes()?, "200 OK");

    cargo_bin_cmd!("dispatch")
        .args([
            "--state-dir",
            state.root.to_str().unwrap(),
            "data",
            "refresh",
        ])
        .env("DISPATCH_CLOUD_URL", url)
        .assert()
        .success()
        .stdout(predicates::str::contains("Public routing data updated"));

    let database = Database::open(state.db_path())?;
    assert_eq!(
        database
            .distributed_public_prior_snapshot()?
            .unwrap()
            .snapshot_id,
        replacement.snapshot_id
    );
    assert_eq!(database.distributed_public_prior_count()?, 1);
    assert!(!state.datasets_dir().exists() || fs::read_dir(state.datasets_dir())?.next().is_none());
    let ranked = rank_harnesses(
        &database,
        &TaskFeatures::default(),
        &["codex".to_owned(), "cursor".to_owned()],
    )?;
    assert_eq!(ranked[0].harness, "cursor");
    assert!(ranked[0].evidence.is_some());
    assert!(ranked[1].evidence.is_none());
    Ok(())
}

#[test]
fn malformed_refresh_keeps_bundled_priors_available() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let state = State {
        root: temp.path().join("state"),
    };
    let url = serve_once(br#"{"schema_version":2}"#.to_vec(), "200 OK");

    cargo_bin_cmd!("dispatch")
        .args([
            "--state-dir",
            state.root.to_str().unwrap(),
            "data",
            "refresh",
        ])
        .env("DISPATCH_CLOUD_URL", url)
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "Existing local data remains available",
        ));

    let database = Database::open(state.db_path())?;
    assert_eq!(
        database
            .distributed_public_prior_snapshot()?
            .unwrap()
            .snapshot_id,
        bundled_snapshot()?.snapshot_id
    );
    Ok(())
}

fn serve_once(body: Vec<u8>, status: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        let read = stream.read(&mut request).unwrap();
        assert!(
            String::from_utf8_lossy(&request[..read]).starts_with("GET /v1/public-priors/latest ")
        );
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(&body).unwrap();
    });
    format!("http://{address}")
}

#[test]
fn public_prior_snapshot_round_trips_with_stable_bytes() -> anyhow::Result<()> {
    let snapshot =
        PublicPriorSnapshotV1::from_priors(vec![prior("cursor", 29, 50), prior("codex", 37, 50)])?;
    let first = snapshot.to_json_bytes()?;
    let parsed = PublicPriorSnapshotV1::from_json_bytes(&first)?;

    assert_eq!(parsed, snapshot);
    assert_eq!(parsed.to_json_bytes()?, first);
    assert_eq!(parsed.schema_version, 1);
    assert_eq!(parsed.entries[0].harness, "codex");
    Ok(())
}

#[test]
fn invalid_public_prior_counts_and_schema_are_rejected() -> anyhow::Result<()> {
    let valid = PublicPriorSnapshotV1::from_priors(vec![prior("codex", 37, 50)])?;
    let bytes = valid.to_json_bytes()?;
    let bad_counts =
        String::from_utf8(bytes.clone())?.replace("\"successes\": 37", "\"successes\": 51");
    assert!(PublicPriorSnapshotV1::from_json_bytes(bad_counts.as_bytes()).is_err());

    let bad_schema =
        String::from_utf8(bytes)?.replace("\"schema_version\": 1", "\"schema_version\": 2");
    assert!(PublicPriorSnapshotV1::from_json_bytes(bad_schema.as_bytes()).is_err());
    Ok(())
}

#[test]
fn malformed_morphology_and_private_fields_are_rejected() -> anyhow::Result<()> {
    let valid = PublicPriorSnapshotV1::from_priors(vec![prior("codex", 37, 50)])?;
    let text = String::from_utf8(valid.to_json_bytes()?)?;
    let malformed = text.replace("\"task_kind\": \"unknown\"", "\"task_kind\": \"repair\"");
    assert!(PublicPriorSnapshotV1::from_json_bytes(malformed.as_bytes()).is_err());

    let private = text.replacen(
        "{\n",
        "{\n  \"task_text\": \"private task\",\n  \"source_path\": \"/private/repo\",\n",
        1,
    );
    assert!(PublicPriorSnapshotV1::from_json_bytes(private.as_bytes()).is_err());
    Ok(())
}

#[test]
fn bundled_snapshot_is_valid_and_installs_idempotently() -> anyhow::Result<()> {
    let snapshot = bundled_snapshot()?;
    assert!(!snapshot.entries.is_empty());
    let mut database = Database::open_in_memory()?;

    database.replace_distributed_public_priors(&snapshot, "bundled")?;
    database.replace_distributed_public_priors(&snapshot, "bundled")?;

    let current = database.distributed_public_prior_snapshot()?.unwrap();
    assert_eq!(current.snapshot_id, snapshot.snapshot_id);
    assert_eq!(
        database.distributed_public_prior_count()?,
        snapshot.entries.len()
    );
    Ok(())
}

#[test]
fn manual_replacement_cannot_delete_the_distributed_snapshot() -> anyhow::Result<()> {
    let snapshot = bundled_snapshot()?;
    let mut database = Database::open_in_memory()?;
    database.replace_distributed_public_priors(&snapshot, "bundled")?;
    let manual = snapshot.entries[0].to_prior(Utc::now());

    database.replace_benchmark_prior(&manual)?;

    assert_eq!(database.distributed_public_prior_count()?, 1);
    assert_eq!(database.manual_benchmark_priors()?.len(), 1);
    Ok(())
}

#[test]
fn invalid_replacement_leaves_the_previous_snapshot_intact() -> anyhow::Result<()> {
    let original = PublicPriorSnapshotV1::from_priors(vec![prior("codex", 37, 50)])?;
    let mut database = Database::open_in_memory()?;
    database.replace_distributed_public_priors(&original, "bundled")?;

    let mut invalid = PublicPriorSnapshotV1::from_priors(vec![prior("cursor", 29, 50)])?;
    invalid.entries[0].attempts = 0;
    assert!(
        database
            .replace_distributed_public_priors(&invalid, "downloaded")
            .is_err()
    );

    let current = database.distributed_public_prior_snapshot()?.unwrap();
    assert_eq!(current.snapshot_id, original.snapshot_id);
    assert_eq!(database.distributed_public_prior_count()?, 1);
    Ok(())
}
