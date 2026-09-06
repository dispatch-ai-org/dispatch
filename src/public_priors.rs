use std::{cmp::Ordering, fs, path::Path, process::Command};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    BenchmarkPrior, TaskKind, TaskScope, VERSION,
    db::Database,
    executor::{trusted_host_executable, trusted_host_path},
    state::State,
    sync::configured_cloud_url,
};

pub const PUBLIC_PRIOR_SCHEMA_VERSION: u32 = 1;
const BUNDLED_SNAPSHOT: &[u8] = include_bytes!("../public-priors/public-priors-v1.json");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PublicPriorSnapshotV1 {
    pub schema_version: u32,
    pub snapshot_id: String,
    pub generated_at: DateTime<Utc>,
    pub entries: Vec<PublicPriorEntryV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PublicPriorEntryV1 {
    pub source: String,
    pub dataset: String,
    pub dataset_version: String,
    pub harness: String,
    pub model: Option<String>,
    pub language: Option<String>,
    pub task_kind: TaskKind,
    pub scope: TaskScope,
    pub successes: u64,
    pub attempts: u64,
}

impl PublicPriorSnapshotV1 {
    pub fn from_priors(priors: Vec<BenchmarkPrior>) -> Result<Self> {
        anyhow::ensure!(!priors.is_empty(), "public prior snapshot has no entries");
        let generated_at = priors
            .iter()
            .map(|prior| prior.updated_at)
            .max()
            .context("public prior snapshot has no generation timestamp")?;
        let mut entries = priors
            .into_iter()
            .map(PublicPriorEntryV1::from)
            .collect::<Vec<_>>();
        entries.sort_by(compare_entries);
        let snapshot_id = content_id(&entries)?;
        let snapshot = Self {
            schema_version: PUBLIC_PRIOR_SCHEMA_VERSION,
            snapshot_id,
            generated_at,
            entries,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self> {
        let snapshot: Self = serde_json::from_slice(bytes).context("invalid public prior JSON")?;
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn to_json_bytes(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut bytes = serde_json::to_vec_pretty(self)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.schema_version == PUBLIC_PRIOR_SCHEMA_VERSION,
            "unsupported public prior schema version {}",
            self.schema_version
        );
        anyhow::ensure!(
            !self.entries.is_empty(),
            "public prior snapshot has no entries"
        );
        anyhow::ensure!(
            self.entries
                .windows(2)
                .all(|pair| compare_entries(&pair[0], &pair[1]) != Ordering::Greater),
            "public prior entries are not canonically sorted"
        );
        for entry in &self.entries {
            entry.validate()?;
        }
        anyhow::ensure!(
            self.snapshot_id == content_id(&self.entries)?,
            "public prior snapshot ID does not match its entries"
        );
        Ok(())
    }
}

impl PublicPriorEntryV1 {
    fn validate(&self) -> Result<()> {
        checked_text(&self.source, "source")?;
        checked_text(&self.dataset, "dataset")?;
        checked_text(&self.dataset_version, "dataset_version")?;
        checked_text(&self.harness, "harness")?;
        if let Some(model) = &self.model {
            checked_text(model, "model")?;
        }
        if let Some(language) = &self.language {
            anyhow::ensure!(
                matches!(language.as_str(), "rust" | "python" | "go" | "cpp"),
                "unsupported public prior language {language:?}"
            );
        }
        anyhow::ensure!(self.attempts > 0, "public prior attempts must be positive");
        anyhow::ensure!(
            self.successes <= self.attempts,
            "public prior successes cannot exceed attempts"
        );
        Ok(())
    }

    pub fn to_prior(&self, updated_at: DateTime<Utc>) -> BenchmarkPrior {
        BenchmarkPrior {
            source: self.source.clone(),
            dataset: self.dataset.clone(),
            dataset_version: self.dataset_version.clone(),
            harness: self.harness.clone(),
            model: self.model.clone(),
            language: self.language.clone(),
            task_kind: self.task_kind.clone(),
            scope: self.scope.clone(),
            successes: self.successes,
            attempts: self.attempts,
            updated_at,
        }
    }
}

impl From<BenchmarkPrior> for PublicPriorEntryV1 {
    fn from(prior: BenchmarkPrior) -> Self {
        Self {
            source: prior.source,
            dataset: prior.dataset,
            dataset_version: prior.dataset_version,
            harness: prior.harness,
            model: prior.model,
            language: prior.language,
            task_kind: prior.task_kind,
            scope: prior.scope,
            successes: prior.successes,
            attempts: prior.attempts,
        }
    }
}

pub fn bundled_snapshot() -> Result<PublicPriorSnapshotV1> {
    PublicPriorSnapshotV1::from_json_bytes(BUNDLED_SNAPSHOT)
        .context("bundled public prior snapshot is invalid")
}

pub fn export_public_priors(state: &State, output: &Path) -> Result<PublicPriorSnapshotV1> {
    state.initialize()?;
    let database = Database::open(state.db_path())?;
    let snapshot = PublicPriorSnapshotV1::from_priors(database.manual_benchmark_priors()?)?;
    fs::write(output, snapshot.to_json_bytes()?)
        .with_context(|| format!("failed to write {}", output.display()))?;
    Ok(snapshot)
}

pub fn open_database(state: &State) -> Result<Database> {
    state.initialize()?;
    let mut database = Database::open(state.db_path())?;
    let bundled = bundled_snapshot()?;
    let install = match database.distributed_public_prior_snapshot()? {
        None => true,
        Some(current) => {
            current.installed_from == "bundled" && current.snapshot_id != bundled.snapshot_id
        }
    };
    if install {
        database.replace_distributed_public_priors(&bundled, "bundled")?;
    }
    Ok(database)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshResult {
    Updated,
    Current,
}

pub fn refresh(state: &State) -> Result<(RefreshResult, PublicPriorSnapshotV1)> {
    let mut database = open_database(state)?;
    let base_url = configured_cloud_url()?;
    let endpoint = format!("{base_url}/v1/public-priors/latest");
    let curl = trusted_host_executable("curl")?;
    let output = Command::new(curl)
        .env_clear()
        .env("PATH", trusted_host_path())
        .args([
            "--silent",
            "--show-error",
            "--fail",
            "--connect-timeout",
            "5",
            "--max-time",
            "10",
            "--max-filesize",
            "2097152",
            "--header",
            &format!("User-Agent: dispatch/{VERSION}"),
            &endpoint,
        ])
        .output()
        .context("failed to start public routing data refresh")?;
    anyhow::ensure!(
        output.status.success(),
        "public routing data request failed"
    );
    let snapshot = PublicPriorSnapshotV1::from_json_bytes(&output.stdout)?;
    let current = database
        .distributed_public_prior_snapshot()?
        .is_some_and(|current| current.snapshot_id == snapshot.snapshot_id);
    if current {
        return Ok((RefreshResult::Current, snapshot));
    }
    database.replace_distributed_public_priors(&snapshot, "downloaded")?;
    Ok((RefreshResult::Updated, snapshot))
}

fn checked_text(value: &str, field: &str) -> Result<()> {
    anyhow::ensure!(
        !value.is_empty() && value.trim() == value,
        "public prior {field} must be non-empty without surrounding whitespace"
    );
    anyhow::ensure!(
        value.len() <= 256 && !value.chars().any(char::is_control),
        "public prior {field} is malformed"
    );
    Ok(())
}

fn content_id(entries: &[PublicPriorEntryV1]) -> Result<String> {
    let bytes = serde_json::to_vec(entries)?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn compare_entries(left: &PublicPriorEntryV1, right: &PublicPriorEntryV1) -> Ordering {
    left.source
        .cmp(&right.source)
        .then_with(|| left.dataset.cmp(&right.dataset))
        .then_with(|| left.dataset_version.cmp(&right.dataset_version))
        .then_with(|| left.harness.cmp(&right.harness))
        .then_with(|| left.model.cmp(&right.model))
        .then_with(|| left.language.cmp(&right.language))
        .then_with(|| left.task_kind.as_str().cmp(right.task_kind.as_str()))
        .then_with(|| left.scope.as_str().cmp(right.scope.as_str()))
}
