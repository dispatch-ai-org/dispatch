use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use chrono::Utc;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{BenchmarkPrior, TaskKind, TaskScope, db::Database, state::State};

mod terminal_bench;

pub use terminal_bench::{
    HarborTrialObservation, TerminalBenchSnapshot, import_terminal_bench,
    read_terminal_bench_snapshot,
};

const SOURCE: &str = "swe-bench/experiments";
const DATASET: &str = "SWE-bench_Verified";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweBenchObservation {
    pub task_id: String,
    pub resolved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweBenchSnapshot {
    pub source: String,
    pub dataset: String,
    pub dataset_version: String,
    pub harness: String,
    pub model: String,
    pub observations: Vec<SweBenchObservation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasetImportReport {
    pub prior: BenchmarkPrior,
    pub raw_snapshot_path: PathBuf,
}

struct ParsedBundle {
    snapshot: SweBenchSnapshot,
    snapshot_id: String,
    metadata: Vec<u8>,
    instances: Vec<u8>,
    results: Vec<u8>,
}

#[derive(Deserialize)]
struct Metadata {
    tags: MetadataTags,
}

#[derive(Deserialize)]
struct MetadataTags {
    agent: String,
    model: Vec<String>,
    system: MetadataSystem,
}

#[derive(Deserialize)]
struct MetadataSystem {
    attempts: serde_yaml::Value,
}

#[derive(Deserialize)]
struct Instance {
    instance_id: String,
}

#[derive(Deserialize)]
struct ResultsFile {
    #[serde(default)]
    no_generation: Vec<String>,
    #[serde(default)]
    no_logs: Vec<String>,
    resolved: Vec<String>,
}

pub fn read_swe_bench_snapshot(path: &Path) -> Result<SweBenchSnapshot> {
    Ok(parse_bundle(path)?.snapshot)
}

pub fn import_swe_bench(state: &State, path: &Path) -> Result<DatasetImportReport> {
    let bundle = parse_bundle(path)?;
    state.initialize()?;

    let raw_snapshot_path = state
        .datasets_dir()
        .join("swe-bench")
        .join(&bundle.snapshot_id);
    fs::create_dir_all(raw_snapshot_path.join("results"))?;
    fs::write(raw_snapshot_path.join("metadata.yaml"), &bundle.metadata)?;
    fs::write(raw_snapshot_path.join("instances.jsonl"), &bundle.instances)?;
    fs::write(
        raw_snapshot_path.join("results/results.json"),
        &bundle.results,
    )?;

    let successes = u64::try_from(
        bundle
            .snapshot
            .observations
            .iter()
            .filter(|observation| observation.resolved)
            .count(),
    )?;
    let attempts = u64::try_from(bundle.snapshot.observations.len())?;
    let prior = BenchmarkPrior {
        source: bundle.snapshot.source,
        dataset: bundle.snapshot.dataset,
        dataset_version: bundle.snapshot.dataset_version,
        harness: bundle.snapshot.harness,
        model: Some(bundle.snapshot.model),
        language: Some("python".to_owned()),
        task_kind: TaskKind::BugFix,
        scope: TaskScope::Unknown,
        successes,
        attempts,
        updated_at: Utc::now(),
    };
    let mut database = Database::open(state.db_path())?;
    database.replace_benchmark_prior(&prior)?;

    Ok(DatasetImportReport {
        prior,
        raw_snapshot_path,
    })
}

fn parse_bundle(path: &Path) -> Result<ParsedBundle> {
    anyhow::ensure!(path.is_dir(), "SWE-bench import path is not a directory");
    let metadata = read(path.join("metadata.yaml"))?;
    let instances = read(path.join("instances.jsonl"))?;
    let results = read(path.join("results/results.json"))?;

    let metadata_value: Metadata =
        serde_yaml::from_slice(&metadata).context("invalid SWE-bench metadata.yaml")?;
    anyhow::ensure!(
        !metadata_value.tags.agent.trim().is_empty(),
        "SWE-bench metadata tags.agent must not be empty"
    );
    anyhow::ensure!(
        metadata_value.tags.model.len() == 1,
        "SWE-bench metadata must identify exactly one model"
    );
    let model = metadata_value.tags.model[0].clone();
    anyhow::ensure!(
        !model.trim().is_empty(),
        "SWE-bench metadata model must not be empty"
    );
    anyhow::ensure!(
        is_single_attempt(&metadata_value.tags.system.attempts),
        "SWE-bench import supports only pass@1 submissions"
    );

    let instance_text =
        std::str::from_utf8(&instances).context("SWE-bench instances.jsonl is not UTF-8")?;
    let mut instance_ids = BTreeSet::new();
    for (index, line) in instance_text.lines().enumerate() {
        anyhow::ensure!(
            !line.trim().is_empty(),
            "SWE-bench instances.jsonl line {} is empty",
            index + 1
        );
        let instance: Instance = serde_json::from_str(line)
            .with_context(|| format!("invalid SWE-bench instances.jsonl line {}", index + 1))?;
        anyhow::ensure!(
            !instance.instance_id.trim().is_empty(),
            "SWE-bench instance ID must not be empty"
        );
        anyhow::ensure!(
            instance_ids.insert(instance.instance_id.clone()),
            "duplicate SWE-bench instance ID {}",
            instance.instance_id
        );
    }
    anyhow::ensure!(
        !instance_ids.is_empty(),
        "SWE-bench instances.jsonl has no task records"
    );

    let results_value: ResultsFile =
        serde_json::from_slice(&results).context("invalid SWE-bench results/results.json")?;
    let resolved = checked_ids("resolved", &results_value.resolved, &instance_ids)?;
    let no_generation = checked_ids("no_generation", &results_value.no_generation, &instance_ids)?;
    let no_logs = checked_ids("no_logs", &results_value.no_logs, &instance_ids)?;
    anyhow::ensure!(
        resolved.is_disjoint(&no_generation)
            && resolved.is_disjoint(&no_logs)
            && no_generation.is_disjoint(&no_logs),
        "SWE-bench result categories overlap"
    );

    let observations = instance_ids
        .into_iter()
        .map(|task_id| SweBenchObservation {
            resolved: resolved.contains(&task_id),
            task_id,
        })
        .collect();
    let snapshot_id = digest(&[&metadata, &instances, &results]);
    let dataset_version = format!("sha256:{snapshot_id}");
    Ok(ParsedBundle {
        snapshot: SweBenchSnapshot {
            source: SOURCE.to_owned(),
            dataset: DATASET.to_owned(),
            dataset_version,
            harness: metadata_value.tags.agent,
            model,
            observations,
        },
        snapshot_id,
        metadata,
        instances,
        results,
    })
}

fn checked_ids(
    label: &str,
    values: &[String],
    instance_ids: &BTreeSet<String>,
) -> Result<BTreeSet<String>> {
    let mut checked = BTreeSet::new();
    for value in values {
        anyhow::ensure!(
            instance_ids.contains(value),
            "SWE-bench {label} contains unknown instance ID {value}"
        );
        anyhow::ensure!(
            checked.insert(value.clone()),
            "SWE-bench {label} contains duplicate instance ID {value}"
        );
    }
    Ok(checked)
}

fn is_single_attempt(value: &serde_yaml::Value) -> bool {
    match value {
        serde_yaml::Value::Number(number) => number.as_u64() == Some(1),
        serde_yaml::Value::String(value) => value == "1",
        _ => false,
    }
}

fn digest(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.len().to_be_bytes());
        hasher.update(part);
    }
    hex::encode(hasher.finalize())
}

fn read(path: PathBuf) -> Result<Vec<u8>> {
    fs::read(&path).with_context(|| format!("failed to read {}", path.display()))
}
