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
    pub effort: Option<String>,
    pub executable: Option<PathBuf>,
    pub extra_args: Vec<String>,
    /// Selected allocation service mode. Project YAML cannot set this value.
    #[serde(skip)]
    pub allocation_service_mode: Option<String>,
    #[serde(skip)]
    pub claude_subscription: Option<crate::harness::claude::SubscriptionEvidence>,
}

impl HarnessesConfig {
    pub fn get(&self, harness: &str) -> &HarnessConfig {
        match harness {
            "claude" => &self.claude,
            "cursor" => &self.cursor,
            _ => &self.codex,
        }
    }
    pub fn bind_profile(&mut self, profile: &ResourceProfile) {
        let config = match profile.harness.as_str() {
            "claude" => &mut self.claude,
            _ => &mut self.codex,
        };
        config.model = Some(profile.model.clone());
        config.effort = profile.effort.clone();
        config.allocation_service_mode = Some(profile.service_mode.clone());
        config.claude_subscription = profile.claude_subscription.clone();
    }
    pub fn bind(&mut self, choice: &crate::ResourceChoice, resources: &ResourceConfig) {
        if let Some(profile) = resources.profiles.iter().find(|p| {
            p.enabled
                && p.harness == choice.harness
                && p.provider == choice.provider
                && p.funding_source == choice.funding_source
                && p.pool == choice.pool
                && p.model == choice.resolved_model
                && p.effort == choice.effort
        }) {
            self.bind_profile(profile);
        }
    }
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
        for (name, harness) in [
            ("claude", &self.harnesses.claude),
            ("codex", &self.harnesses.codex),
            ("cursor", &self.harnesses.cursor),
        ] {
            validate_model(harness.model.as_deref())
                .with_context(|| format!("invalid harnesses.{name}.model"))?;
            validate_effort(harness.effort.as_deref())
                .with_context(|| format!("invalid harnesses.{name}.effort"))?;
        }
        Ok(())
    }

    pub fn example_yaml() -> &'static str {
        "execution:\n  backend: local\n  timeout_secs: 1800\n  cpus: 2\n  memory: 4g\n  max_parallel: 3\n  docker_image: ubuntu:24.04\n  forwarded_env: []\nchecks:\n  baseline: []\n  # Replace [] with your project's verification commands, such as [cargo test].\n  verify: []\nharnesses:\n  claude:\n    model: null\n    effort: null\n    extra_args: []\n  codex:\n    model: null\n    effort: null\n    extra_args: []\n  cursor:\n    model: null\n    effort: null\n    extra_args: []\n"
    }
}

pub fn validate_model(model: Option<&str>) -> Result<()> {
    if let Some(model) = model {
        anyhow::ensure!(!model.trim().is_empty(), "model must not be empty");
        anyhow::ensure!(
            !model.starts_with('-') && !model.chars().any(char::is_whitespace),
            "model must be one non-option identifier"
        );
    }
    Ok(())
}

pub fn validate_effort(effort: Option<&str>) -> Result<()> {
    if let Some(effort) = effort {
        anyhow::ensure!(
            matches!(effort, "minimal" | "low" | "medium" | "high" | "xhigh"),
            "effort must be one of minimal, low, medium, high, or xhigh"
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ResourceConfig {
    pub version: u32,
    pub allocation_enabled: bool,
    pub capacity: CapacityConfig,
    pub profiles: Vec<ResourceProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CapacityConfig {
    pub codex_probe: bool,
    pub probe_timeout_secs: u64,
    pub freshness_secs: u64,
    pub admission: bool,
    pub lease_secs: u64,
    pub heartbeat_secs: u64,
    pub aging_secs: u64,
}

impl Default for CapacityConfig {
    fn default() -> Self {
        Self {
            codex_probe: true,
            probe_timeout_secs: 5,
            freshness_secs: 300,
            admission: true,
            lease_secs: 20,
            heartbeat_secs: 5,
            aging_secs: 60,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceProfile {
    #[serde(default = "enabled_by_default", skip_serializing_if = "is_enabled")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_subscription: Option<crate::harness::claude::SubscriptionEvidence>,
    pub provider: String,
    pub funding_source: String,
    pub harness: String,
    pub model: String,
    pub effort: Option<String>,
    pub service_mode: String,
    pub runtime: String,
    pub pool: String,
    /// Opaque provider limit IDs known to constrain this shared pool.
    #[serde(default)]
    pub provider_buckets: Vec<String>,
    pub tier: crate::ResourceTier,
    #[serde(default)]
    pub included: bool,
    #[serde(default)]
    pub no_overage_verified: bool,
    /// User-controlled epoch for the time-bound no-overage authorization.
    /// Incrementing this is the explicit acknowledgement required after a
    /// conflicting account or funding observation invalidates an epoch.
    #[serde(default = "default_authorization_revision")]
    pub authorization_revision: u64,
}

impl ResourceProfile {
    /// A local display-name change cannot manufacture a new account allowance.
    pub fn funding_scope(&self) -> String {
        self.claude_subscription
            .as_ref()
            .filter(|_| self.harness == "claude")
            .map(|e| format!("claude-account:{}", e.account_sha256))
            .unwrap_or_else(|| self.funding_source.clone())
    }
    pub fn admission_buckets(&self) -> Vec<String> {
        // Claude v1 has no authoritative preflight bucket mapping. All choices
        // for the authenticated account conservatively share one allowance.
        if self.harness == "claude" {
            Vec::new()
        } else {
            self.provider_buckets.clone()
        }
    }
    pub fn eligibility(&self) -> Result<()> {
        anyhow::ensure!(self.enabled, "resource profile disabled");
        anyhow::ensure!(
            self.included && self.no_overage_verified,
            "included-only funding is not validated"
        );
        if self.harness == "claude" {
            crate::harness::claude::eligibility(self)?;
        }
        Ok(())
    }
}

fn enabled_by_default() -> bool {
    true
}
fn is_enabled(value: &bool) -> bool {
    *value
}

fn default_authorization_revision() -> u64 {
    1
}

impl ResourceConfig {
    pub fn load(state_root: &Path) -> Result<Self> {
        let path = state_root.join("resources.yml");
        if !path.is_file() {
            return Ok(Self {
                version: 1,
                ..Self::default()
            });
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read resource config {}", path.display()))?;
        let config: Self = serde_yaml::from_str(&raw)
            .with_context(|| format!("failed to parse resource config {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(self.version == 1, "resources.yml version must be 1");
        anyhow::ensure!(
            self.capacity.probe_timeout_secs > 0,
            "capacity probe timeout must be positive"
        );
        anyhow::ensure!(
            self.capacity.freshness_secs > 0,
            "capacity freshness must be positive"
        );
        anyhow::ensure!(
            self.capacity.lease_secs > 0,
            "capacity lease duration must be positive"
        );
        anyhow::ensure!(
            self.capacity.heartbeat_secs > 0,
            "capacity heartbeat must be positive"
        );
        anyhow::ensure!(
            self.capacity.heartbeat_secs < self.capacity.lease_secs,
            "capacity heartbeat must be shorter than the lease duration"
        );
        anyhow::ensure!(
            self.capacity.aging_secs > 0,
            "capacity aging must be positive"
        );
        for profile in &self.profiles {
            if !profile.enabled {
                continue;
            }
            anyhow::ensure!(
                profile.authorization_revision > 0,
                "resource profile authorization_revision must be positive"
            );
            anyhow::ensure!(
                matches!(
                    (profile.harness.as_str(), profile.provider.as_str()),
                    ("codex", "openai") | ("claude", "anthropic")
                ),
                "resource profile provider/harness must be openai/codex or anthropic/claude"
            );
            validate_model(Some(&profile.model))?;
            validate_effort(profile.effort.as_deref())?;
            if profile.included {
                anyhow::ensure!(
                    profile.service_mode == "standard",
                    "included allocation profiles must use standard service mode"
                );
            }
            if profile.harness == "claude" {
                crate::harness::claude::validate_profile(profile)?;
            }
            for (name, value) in [
                ("funding_source", &profile.funding_source),
                ("service_mode", &profile.service_mode),
                ("runtime", &profile.runtime),
                ("pool", &profile.pool),
            ] {
                anyhow::ensure!(
                    !value.trim().is_empty(),
                    "resource profile {name} must not be empty"
                );
            }
            anyhow::ensure!(
                profile
                    .provider_buckets
                    .iter()
                    .all(|bucket| !bucket.trim().is_empty()),
                "provider bucket IDs must not be empty"
            );
        }
        for (index, left) in self.profiles.iter().enumerate() {
            for right in &self.profiles[index + 1..] {
                if !left.enabled || !right.enabled {
                    continue;
                }
                if left.pool == right.pool {
                    anyhow::ensure!(
                        left.provider == right.provider
                            && left.funding_scope() == right.funding_scope()
                            && left.provider_buckets == right.provider_buckets,
                        "profiles in one resource pool must share provider, funding source, and provider bucket mapping"
                    );
                }
                let overlapping_buckets = left.admission_buckets().is_empty()
                    || right.provider_buckets.is_empty()
                    || left
                        .provider_buckets
                        .iter()
                        .any(|bucket| right.provider_buckets.contains(bucket));
                if left.provider == right.provider
                    && left.funding_scope() == right.funding_scope()
                    && overlapping_buckets
                {
                    anyhow::ensure!(
                        left.pool == right.pool,
                        "profiles sharing a funding source and allowance bucket must use one resource pool"
                    );
                }
            }
        }
        Ok(())
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

    #[test]
    fn parses_chatgpt_and_claude_shaped_resource_profiles() {
        let config: ResourceConfig = serde_yaml::from_str(
            "version: 1\nallocation_enabled: false\nprofiles:\n  - provider: openai\n    funding_source: chatgpt-plus\n    harness: codex\n    model: codex-light\n    effort: low\n    service_mode: standard\n    runtime: local\n    pool: chatgpt-codex\n    tier: light\n    included: true\n    no_overage_verified: true\n  - provider: anthropic\n    funding_source: claude-pro\n    harness: claude\n    model: claude-example\n    effort: null\n    service_mode: standard\n    runtime: local\n    pool: claude-code\n    tier: strong\n    included: true\n    no_overage_verified: false\n",
        )
        .unwrap();
        config.validate().unwrap();
        assert_eq!(config.profiles.len(), 2);
        assert_eq!(config.profiles[1].provider, "anthropic");
    }

    #[test]
    fn shared_allowance_cannot_be_split_into_per_model_tanks() {
        let config: ResourceConfig = serde_yaml::from_str(
            "version: 1\nprofiles:\n  - provider: openai\n    funding_source: chatgpt-plus\n    harness: codex\n    model: light\n    effort: low\n    service_mode: standard\n    runtime: local\n    pool: light-only\n    provider_buckets: [codex]\n    tier: light\n    included: true\n    no_overage_verified: true\n  - provider: openai\n    funding_source: chatgpt-plus\n    harness: codex\n    model: strong\n    effort: high\n    service_mode: standard\n    runtime: local\n    pool: strong-only\n    provider_buckets: [codex]\n    tier: strong\n    included: true\n    no_overage_verified: true\n",
        )
        .unwrap();
        assert!(
            config
                .validate()
                .unwrap_err()
                .to_string()
                .contains("one resource pool")
        );
    }
}
