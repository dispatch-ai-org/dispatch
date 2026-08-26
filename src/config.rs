use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub execution: ExecutionConfig,
    pub checks: ChecksConfig,
    pub harnesses: HarnessesConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ExecutionConfig {
    pub backend: String,
    pub timeout_secs: u64,
    pub cpus: f64,
    pub memory: String,
    pub max_parallel: usize,
    pub docker_image: String,
    pub forwarded_env: Vec<String>,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            backend: "local".into(),
            timeout_secs: 1800,
            cpus: 2.0,
            memory: "4g".into(),
            max_parallel: 3,
            docker_image: "ubuntu:24.04".into(),
            forwarded_env: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChecksConfig {
    pub baseline: Vec<String>,
    pub verify: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HarnessesConfig {
    pub claude: HarnessConfig,
    pub codex: HarnessConfig,
    pub cursor: HarnessConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HarnessConfig {
    pub model: Option<String>,
    pub executable: Option<PathBuf>,
    pub extra_args: Vec<String>,
}

impl Config {
    pub fn discover(source: &Path, explicit: Option<&Path>) -> Result<(Self, Option<PathBuf>)> {
        let path = explicit.map(Path::to_path_buf).or_else(|| {
            [
                "dispatch.yml",
                "dispatch.yaml",
                ".dispatch.yml",
                ".dispatch.yaml",
            ]
            .into_iter()
            .map(|name| source.join(name))
            .find(|p| p.is_file())
        });
        match path {
            Some(path) => {
                let raw = fs::read_to_string(&path)
                    .with_context(|| format!("failed to read config {}", path.display()))?;
                let config = serde_yaml::from_str(&raw)
                    .with_context(|| format!("failed to parse config {}", path.display()))?;
                Ok((config, Some(path)))
            }
            None => Ok((Self::default(), None)),
        }
    }

    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            matches!(self.execution.backend.as_str(), "local" | "docker"),
            "execution.backend must be local or docker"
        );
        anyhow::ensure!(
            self.execution.timeout_secs > 0,
            "execution.timeout_secs must be positive"
        );
        anyhow::ensure!(self.execution.cpus > 0.0, "execution.cpus must be positive");
        anyhow::ensure!(
            self.execution.max_parallel > 0,
            "execution.max_parallel must be positive"
        );
        anyhow::ensure!(
            !self.execution.memory.trim().is_empty(),
            "execution.memory must not be empty"
        );
        Ok(())
    }

    pub fn example_yaml() -> &'static str {
        "execution:\n  backend: local\n  timeout_secs: 1800\n  cpus: 2\n  memory: 4g\n  max_parallel: 3\n  docker_image: ubuntu:24.04\n  forwarded_env: []\nchecks:\n  baseline: []\n  verify: []\nharnesses:\n  claude:\n    model: null\n    extra_args: []\n  codex:\n    model: null\n    extra_args: []\n  cursor:\n    model: null\n    extra_args: []\n"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        Config::default().validate().unwrap();
    }

    #[test]
    fn parses_partial_config_with_defaults() {
        let config: Config = serde_yaml::from_str(
            "execution:\n  timeout_secs: 12\nchecks:\n  verify: [cargo test]\n",
        )
        .unwrap();
        assert_eq!(config.execution.timeout_secs, 12);
        assert_eq!(config.execution.backend, "local");
        assert_eq!(config.checks.verify, vec!["cargo test"]);
    }
}
