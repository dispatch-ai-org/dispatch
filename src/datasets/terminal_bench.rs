use std::{collections::BTreeSet, fs, path::Path};

use anyhow::{Context, Result};
use chrono::Utc;
use serde::Deserialize;

use crate::{BenchmarkPrior, TaskKind, TaskScope, state::State};

use super::{DatasetImportReport, cache_snapshot, digest, finish_import, read};

const SOURCE: &str = "harbor-framework/harbor";

#[derive(Debug, Clone, PartialEq)]
pub struct HarborTrialObservation {
    pub task_id: String,
    pub trial_id: String,
    pub trial_name: String,
    pub reward: f64,
    pub succeeded: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TerminalBenchSnapshot {
    pub source: String,
    pub dataset: String,
    pub dataset_version: String,
    pub upstream_source: String,
    pub agent: String,
    pub agent_version: String,
    pub harness: String,
    pub model: Option<String>,
    pub observations: Vec<HarborTrialObservation>,
}

struct ParsedJob {
    snapshot: TerminalBenchSnapshot,
    snapshot_id: String,
    config: Vec<u8>,
    trials: Vec<TrialFile>,
}

struct TrialFile {
    directory: String,
    contents: Vec<u8>,
}

#[derive(Deserialize)]
struct JobConfig {
    agents: Vec<JobAgent>,
    datasets: Vec<DatasetConfig>,
}

#[derive(Deserialize)]
struct JobAgent {
    name: String,
}

#[derive(Deserialize)]
struct DatasetConfig {
    name: String,
    version: Option<String>,
    #[serde(rename = "ref")]
    reference: Option<String>,
}

#[derive(Deserialize)]
struct TrialResult {
    id: String,
    task_name: String,
    trial_name: String,
    task_id: serde_json::Value,
    source: Option<String>,
    agent_info: AgentInfo,
    verifier_result: Option<VerifierResult>,
}

#[derive(Deserialize)]
struct AgentInfo {
    name: String,
    version: String,
    model_info: Option<ModelInfo>,
}

#[derive(Deserialize)]
struct ModelInfo {
    name: String,
    provider: Option<String>,
}

#[derive(Deserialize)]
struct VerifierResult {
    rewards: Option<serde_json::Map<String, serde_json::Value>>,
}

pub fn read_terminal_bench_snapshot(path: &Path) -> Result<TerminalBenchSnapshot> {
    Ok(parse_job(path)?.snapshot)
}

pub fn import_terminal_bench(state: &State, path: &Path) -> Result<DatasetImportReport> {
    let job = parse_job(path)?;
    let raw_snapshot_path = cache_snapshot(
        state,
        "terminal-bench",
        &job.snapshot_id,
        std::iter::once(("config.json".into(), job.config)).chain(job.trials.into_iter().map(
            |trial| {
                (
                    Path::new(&trial.directory).join("result.json"),
                    trial.contents,
                )
            },
        )),
    )?;

    let successes = u64::try_from(
        job.snapshot
            .observations
            .iter()
            .filter(|observation| observation.succeeded)
            .count(),
    )?;
    let attempts = u64::try_from(job.snapshot.observations.len())?;
    let prior = BenchmarkPrior {
        source: job.snapshot.source,
        dataset: job.snapshot.dataset,
        dataset_version: job.snapshot.dataset_version,
        harness: job.snapshot.harness,
        model: job.snapshot.model,
        language: None,
        task_kind: TaskKind::Unknown,
        scope: TaskScope::Unknown,
        successes,
        attempts,
        updated_at: Utc::now(),
    };
    finish_import(state, prior, raw_snapshot_path)
}

fn parse_job(path: &Path) -> Result<ParsedJob> {
    anyhow::ensure!(
        path.is_dir(),
        "Terminal-Bench import path is not a directory"
    );
    let config = read(path.join("config.json"))?;
    let config_value: JobConfig =
        serde_json::from_slice(&config).context("invalid Harbor config.json")?;
    anyhow::ensure!(
        config_value.datasets.len() == 1,
        "Harbor import requires exactly one dataset"
    );
    let dataset = &config_value.datasets[0];
    anyhow::ensure!(
        matches!(
            dataset.name.as_str(),
            "terminal-bench/terminal-bench" | "terminal-bench/terminal-bench-2"
        ),
        "Harbor job dataset is not an official Terminal-Bench package"
    );
    let upstream_version = dataset_version(dataset)?;

    let configured_agents = config_value
        .agents
        .iter()
        .map(|agent| checked_text(&agent.name, "Harbor config agent name"))
        .collect::<Result<BTreeSet<_>>>()?;
    anyhow::ensure!(!configured_agents.is_empty(), "Harbor config has no agents");

    let trials = read_trial_files(path)?;
    let mut observations = Vec::with_capacity(trials.len());
    let mut trial_ids = BTreeSet::new();
    let mut trial_names = BTreeSet::new();
    let mut common_identity: Option<(String, String, Option<String>, String)> = None;

    for trial_file in &trials {
        let trial: TrialResult = serde_json::from_slice(&trial_file.contents)
            .with_context(|| format!("invalid Harbor {}/result.json", trial_file.directory))?;
        let trial_id = checked_text(&trial.id, "Harbor trial ID")?;
        let trial_name = checked_text(&trial.trial_name, "Harbor trial name")?;
        anyhow::ensure!(
            trial_name == trial_file.directory,
            "Harbor trial name does not match its directory"
        );
        anyhow::ensure!(
            trial_ids.insert(trial_id.clone()),
            "duplicate Harbor trial ID {trial_id}"
        );
        anyhow::ensure!(
            trial_names.insert(trial_name.clone()),
            "duplicate Harbor trial name {trial_name}"
        );
        anyhow::ensure!(
            trial.task_id.is_object(),
            "Harbor trial task_id must be an object"
        );
        let task_id = checked_text(&trial.task_name, "Harbor task name")?;
        let agent = checked_text(&trial.agent_info.name, "Harbor agent name")?;
        anyhow::ensure!(
            configured_agents.contains(agent.as_str()),
            "Harbor trial agent {agent} is absent from config.json"
        );
        let agent_version = checked_text(&trial.agent_info.version, "Harbor agent version")?;
        let model = model_identity(trial.agent_info.model_info)?;
        let upstream_source = checked_text(
            trial.source.as_deref().unwrap_or_default(),
            "Harbor trial source",
        )?;
        let identity = (
            agent.clone(),
            agent_version.clone(),
            model.clone(),
            upstream_source,
        );
        if let Some(expected) = &common_identity {
            anyhow::ensure!(
                expected == &identity,
                "Harbor import cannot aggregate mixed agent, model, version, or source identities"
            );
        } else {
            common_identity = Some(identity);
        }

        let reward = objective_reward(trial.verifier_result.as_ref())?;
        observations.push(HarborTrialObservation {
            task_id,
            trial_id,
            trial_name,
            reward,
            succeeded: reward == 1.0,
        });
    }

    let (agent, agent_version, model, upstream_source) =
        common_identity.context("Harbor job has no trial results")?;
    // Keep Harbor's existing filename/content hash order, distinct from SWE-bench.
    let snapshot_id = digest(
        [b"config.json".as_slice(), config.as_slice()]
            .into_iter()
            .chain(
                trials
                    .iter()
                    .flat_map(|trial| [trial.directory.as_bytes(), trial.contents.as_slice()]),
            ),
    );
    let dataset_version = format!("{upstream_version}+sha256:{snapshot_id}");
    Ok(ParsedJob {
        snapshot: TerminalBenchSnapshot {
            source: SOURCE.to_owned(),
            dataset: dataset.name.clone(),
            dataset_version,
            upstream_source,
            harness: dispatch_harness(&agent).to_owned(),
            agent,
            agent_version,
            model,
            observations,
        },
        snapshot_id,
        config,
        trials,
    })
}

fn read_trial_files(path: &Path) -> Result<Vec<TrialFile>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(path).with_context(|| format!("failed to read {}", path.display()))? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let result_path = entry.path().join("result.json");
        if !result_path.is_file() {
            continue;
        }
        let directory = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("Harbor trial directory name is not UTF-8"))?;
        files.push(TrialFile {
            directory,
            contents: read(result_path)?,
        });
    }
    files.sort_by(|left, right| left.directory.cmp(&right.directory));
    anyhow::ensure!(
        !files.is_empty(),
        "Harbor job has no trial result.json files"
    );
    Ok(files)
}

fn dataset_version(dataset: &DatasetConfig) -> Result<String> {
    let version = dataset
        .version
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let reference = dataset
        .reference
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let (Some(version), Some(reference)) = (version, reference) {
        anyhow::ensure!(
            version == reference,
            "Harbor dataset version and ref disagree"
        );
    }
    version
        .or(reference)
        .map(str::to_owned)
        .context("Harbor dataset has no version or ref")
}

fn model_identity(model: Option<ModelInfo>) -> Result<Option<String>> {
    let Some(model) = model else {
        return Ok(None);
    };
    let name = checked_text(&model.name, "Harbor model name")?;
    match model.provider {
        Some(provider) => {
            let provider = checked_text(&provider, "Harbor model provider")?;
            Ok(Some(format!("{provider}/{name}")))
        }
        None => Ok(Some(name)),
    }
}

fn objective_reward(result: Option<&VerifierResult>) -> Result<f64> {
    let reward = result
        .and_then(|result| result.rewards.as_ref())
        .and_then(|rewards| rewards.get("reward"))
        .and_then(serde_json::Value::as_f64)
        .context("Harbor trial has no numeric objective reward")?;
    anyhow::ensure!(
        reward == 0.0 || reward == 1.0,
        "Terminal-Bench objective reward must be 0 or 1"
    );
    Ok(reward)
}

fn dispatch_harness(agent: &str) -> &str {
    match agent {
        "codex" => "codex",
        "cursor-cli" => "cursor",
        _ => agent,
    }
}

fn checked_text(value: &str, label: &str) -> Result<String> {
    let value = value.trim();
    anyhow::ensure!(!value.is_empty(), "{label} must not be empty");
    Ok(value.to_owned())
}
