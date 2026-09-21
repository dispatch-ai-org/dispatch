//! Shared local resource setup. Discovery is read-only; only an explicitly
//! confirmed proposal can write configuration. Launch authorization stays in core.
use crate::{
    CapacityValue, ResourceTier,
    config::{ExecutionConfig, ResourceConfig, ResourceProfile},
    state::State,
};
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub struct Discovery {
    pub executable: PathBuf,
    pub version: String,
    pub funding: String,
    pub account: String,
    pub claude: Option<crate::harness::claude::SubscriptionEvidence>,
}
impl Discovery {
    pub fn summary(&self) -> String {
        format!(
            "{}\nAccount fingerprint: {}\nFunding: {} · quota may be unknown\nExecutable: {}",
            self.version,
            &self.account[..self.account.len().min(12)],
            self.funding,
            self.executable.display()
        )
    }
}

pub fn executable(provider: &str) -> Result<PathBuf> {
    ensure!(
        matches!(provider, "codex" | "claude"),
        "choose codex or claude"
    );
    // Setup never executes a repository's configured arbitrary executable.
    let path = which::which(provider).with_context(|| format!("{provider} is not installed on PATH. Install the provider CLI, then return to Resources."))?;
    let path = fs::canonicalize(path)?;
    Ok(path)
}

pub async fn discover(provider: &str, executable: PathBuf) -> Result<Discovery> {
    let version = crate::harness::probe_version(&executable)
        .await?
        .context("provider version unavailable; install a supported CLI")?;
    if provider == "claude" {
        let temp = tempfile::tempdir()?;
        let request = crate::harness::HarnessRunRequest::new(temp.path(), "", temp.path());
        let account = crate::harness::claude::discover_account(
            &executable,
            &crate::executor::Executor::new(ExecutionConfig::default()),
            &request,
        )
        .await?;
        let now = Utc::now();
        let evidence = crate::harness::claude::SubscriptionEvidence {
            contract_version: 1,
            cli_version: version.clone(),
            executable_sha256: hex::encode(Sha256::digest(fs::read(&executable)?)),
            account_sha256: account.clone(),
            checked_at: now,
            valid_until: now + chrono::TimeDelta::hours(24),
            // Discovery cannot assert inclusion, credits or account management.
            print_mode_included: false,
            usage_credits_disabled: false,
            unmanaged_account: false,
        };
        Ok(Discovery {
            executable,
            version,
            funding: "claude-subscription".into(),
            account,
            claude: Some(evidence),
        })
    } else {
        ensure!(provider == "codex", "unsupported provider");
        let result = crate::capacity::probe_codex(
            &executable,
            "setup",
            &[],
            "standard",
            &Default::default(),
        )
        .await;
        let observed = result.observation;
        ensure!(
            matches!(&observed.auth_mode, CapacityValue::Reported { value } if value == "chatgpt"),
            "ChatGPT subscription login was not confirmed. Choose Login, then revalidate; API and unknown authentication cannot be included."
        );
        let CapacityValue::Reported { value: ref plan } = observed.plan_type else {
            anyhow::bail!("subscription plan is unknown; no profile was authorized")
        };
        let funding = format!("chatgpt-{plan}");
        ensure!(
            crate::capacity::funding_change(&observed, &funding).is_none(),
            "paid credits or unsupported service detected; no profile was authorized"
        );
        let CapacityValue::Reported { value: account } = observed.funding_identity else {
            anyhow::bail!("account identity is unknown; no profile was authorized")
        };
        Ok(Discovery {
            executable,
            version,
            funding,
            account: account.trim_start_matches("sha256:").into(),
            claude: None,
        })
    }
}

pub fn status(state: &State) -> Result<String> {
    let config = ResourceConfig::load(&state.root)?;
    let mut lines = vec![
        "Accounts / Resources".to_owned(),
        "Local operation · no Dispatch login required".into(),
    ];
    if !config.allocation_enabled {
        lines.push("Included-resource allocation is not configured.".into());
    }
    if !config.capacity.admission {
        lines.push(
            "Shared admission is disabled; guided setup will not override this policy.".into(),
        );
    }
    for provider in ["codex", "claude"] {
        lines.push(format!(
            "{provider}: {}",
            if which::which(provider).is_ok() {
                "installed · auth checked on revalidation"
            } else {
                "not found on PATH"
            }
        ));
    }
    for (i, p) in config.profiles.iter().enumerate() {
        let eligibility = p
            .eligibility()
            .map(|_| "funding configured · launch revalidation required".into())
            .unwrap_or_else(|e| format!("needs attention: {e}"));
        lines.push(format!(
            "[{}] {} / {} · {}\n    {eligibility}",
            i + 1,
            p.harness,
            p.model,
            p.effort.as_deref().unwrap_or("default effort")
        ));
    }
    if config.profiles.is_empty() {
        lines.push("No resource profiles. Add Codex or Claude to begin.".into());
    }
    Ok(crate::presenter::sanitize(&lines.join("\n")))
}

pub struct Proposal {
    pub config: ResourceConfig,
    pub index: usize,
    original: Option<Vec<u8>>,
    pub discovery: Discovery,
}
impl Proposal {
    pub fn prepare(
        state: &State,
        discovery: Discovery,
        provider: &str,
        existing: Option<usize>,
        model: String,
        effort: String,
        tier: ResourceTier,
    ) -> Result<Self> {
        let path = state.root.join("resources.yml");
        let original = read_config(&path)?;
        let mut config: ResourceConfig = match &original {
            Some(bytes) => serde_yaml::from_slice(bytes)?,
            None => ResourceConfig {
                version: 1,
                ..Default::default()
            },
        };
        config.validate()?;
        ensure!(
            config.capacity.admission,
            "shared admission is disabled; change that advanced policy explicitly before setup"
        );
        let highest = config
            .profiles
            .iter()
            .map(|p| p.authorization_revision)
            .max()
            .unwrap_or(0);
        let retained: u64 = if state.db_path().exists() {
            let db = crate::db::Database::open_read_only(state.db_path())?;
            if db.schema_version()? >= 15 {
                db.connection().query_row(
                    "SELECT COALESCE(MAX(authorization_revision),0) FROM capacity_authorizations",
                    [],
                    |r| r.get(0),
                )?
            } else {
                0
            }
        } else {
            0
        };
        let revision = highest
            .max(retained)
            .checked_add(1)
            .context("authorization revision exhausted")?;
        let index = existing.unwrap_or(config.profiles.len());
        let profile = if let Some(index) = existing {
            let mut profile = config
                .profiles
                .get(index)
                .context("resource selection changed")?
                .clone();
            ensure!(
                profile.harness == provider
                    && profile.runtime == "local"
                    && profile.service_mode == "standard",
                "guided revalidation supports the selected provider in local standard mode only"
            );
            // Account scope cannot be silently moved into another shared pool.
            if let Some(old) = &profile.claude_subscription {
                ensure!(
                    old.account_sha256 == discovery.account,
                    "account changed: add a new resource explicitly; existing account scope was preserved"
                );
            }
            ensure!(
                profile.funding_source == discovery.funding || provider == "claude",
                "subscription plan changed: add a new resource explicitly"
            );
            profile.authorization_revision = revision;
            profile.claude_subscription = discovery.claude.clone();
            profile
        } else {
            let shared = config.profiles.iter().find(|p| {
                p.enabled
                    && p.harness == provider
                    && (provider == "codex" && p.funding_source == discovery.funding
                        || p.claude_subscription
                            .as_ref()
                            .is_some_and(|e| e.account_sha256 == discovery.account))
            });
            // Existing explicit bucket mappings require deliberate advanced setup.
            ensure!(
                shared.is_none_or(|p| p.provider_buckets.is_empty()),
                "existing account uses explicit bucket mappings; revalidate its existing profile instead"
            );
            ResourceProfile {
                enabled: true,
                provider: if provider == "codex" {
                    "openai"
                } else {
                    "anthropic"
                }
                .into(),
                harness: provider.into(),
                funding_source: discovery.funding.clone(),
                model,
                effort: Some(effort),
                service_mode: "standard".into(),
                runtime: "local".into(),
                pool: shared
                    .map(|p| p.pool.clone())
                    .unwrap_or_else(|| format!("{provider}-included-{}", &discovery.account[..12])),
                provider_buckets: vec![],
                tier,
                included: false,
                no_overage_verified: false,
                authorization_revision: revision,
                claude_subscription: discovery.claude.clone(),
            }
        };
        ensure!(
            profile.enabled,
            "disabled resources stay disabled; enable them explicitly in advanced configuration"
        );
        if existing.is_some() {
            config.profiles[index] = profile;
        } else {
            config.profiles.push(profile);
        }
        config.validate()?;
        Ok(Self {
            config,
            index,
            original,
            discovery,
        })
    }
    pub fn summary(&self) -> String {
        let p = &self.config.profiles[self.index];
        format!(
            "{}\n\n{} / {} / {} · {:?}\nLocal runtime · standard service · included only\n{}\n\nConfirm: this exact model and effort are included in my subscription.\nI verified that paid overage / usage credits are disabled.{}\n\nType confirm to authorize and save, or cancel to keep configuration.",
            self.discovery.summary(),
            p.harness,
            p.model,
            p.effort.as_deref().unwrap_or("default"),
            p.tier,
            if p.harness == "claude" {
                "Funding assertion expires in 24 hours."
            } else {
                "Launch checks current account and retained funding evidence."
            },
            if p.harness == "claude" {
                "\nI verified controlled print mode is included and this account is unmanaged."
            } else {
                ""
            }
        )
    }
    /// Call only for the displayed, explicitly confirmed proposal. No trust is
    /// renewed by discovery, cancellation, or possession of provider credentials.
    pub fn confirm(mut self, state: &State) -> Result<()> {
        let p = &mut self.config.profiles[self.index];
        p.included = true;
        p.no_overage_verified = true;
        if let Some(e) = &mut p.claude_subscription {
            e.print_mode_included = true;
            e.usage_credits_disabled = true;
            e.unmanaged_account = true;
        }
        p.eligibility()?;
        self.config.allocation_enabled = true;
        self.config.validate()?;
        state.initialize()?;
        atomic_config(
            &state.root.join("resources.yml"),
            self.original.as_deref(),
            serde_yaml::to_string(&self.config)?.as_bytes(),
        )
    }
}

fn read_config(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            ensure!(
                meta.is_file() && !meta.file_type().is_symlink(),
                "configuration must be a regular file"
            );
            Ok(Some(fs::read(path)?))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
/// Serialize cooperating writers, refuse stale proposals, then replace atomically.
/// Unknown advanced YAML fields survive project-check updates (via Value below).
fn atomic_config(path: &Path, expected: Option<&[u8]>, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("configuration parent missing")?;
    let lock_path = path.with_extension("setup-lock");
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let lock = options.open(lock_path)?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        ensure!(
            unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
            "another setup write is in progress; re-open Resources"
        );
    }
    ensure!(
        read_config(path)?.as_deref() == expected,
        "configuration changed while awaiting confirmation; reopen setup to compare again"
    );
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    fs::File::open(parent)?.sync_all()?;
    drop(lock);
    Ok(())
}

pub fn check_choices(source: &Path) -> Vec<&'static str> {
    let mut choices = Vec::new();
    if source.join("verify.sh").is_file() {
        choices.push("sh ./verify.sh");
    }
    if source.join("Cargo.toml").is_file() {
        choices.push("cargo test --locked");
    }
    if source.join("Makefile").is_file() {
        choices.push("make test");
    }
    choices
}
pub fn save_checks(source: &Path, command: &str, expected: Option<Vec<u8>>) -> Result<()> {
    ensure!(
        check_choices(source).contains(&command),
        "check choice is no longer available"
    );
    let (_, path) = crate::Config::discover(source, None)?;
    let path = path.unwrap_or_else(|| source.join("dispatch.yml"));
    let mut value: serde_yaml::Value = expected
        .as_ref()
        .map(|b| serde_yaml::from_slice(b))
        .transpose()?
        .unwrap_or(serde_yaml::Value::Mapping(Default::default()));
    value["checks"]["verify"] = serde_yaml::to_value(vec![command])?;
    let bytes = serde_yaml::to_string(&value)?;
    let config: crate::Config = serde_yaml::from_str(&bytes)?;
    config.validate()?;
    atomic_config(&path, expected.as_deref(), bytes.as_bytes())
}
pub fn project_config_bytes(source: &Path) -> Result<Option<Vec<u8>>> {
    let (_, path) = crate::Config::discover(source, None)?;
    path.map(|p| read_config(&p))
        .transpose()
        .map(Option::flatten)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn discovery() -> Discovery {
        Discovery {
            executable: "/fixture/codex".into(),
            version: "fixture only".into(),
            funding: "chatgpt-plus".into(),
            account: "a".repeat(64),
            claude: None,
        }
    }
    fn proposal(state: &State) -> Proposal {
        Proposal::prepare(
            state,
            discovery(),
            "codex",
            None,
            "fixed-model".into(),
            "medium".into(),
            ResourceTier::Standard,
        )
        .unwrap()
    }
    #[test]
    fn cancellation_writes_nothing_and_confirmation_is_atomic_and_private() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let state = State {
            root: temp.path().join("state"),
        };
        drop(proposal(&state));
        assert!(!state.root.exists());
        proposal(&state).confirm(&state)?;
        let config = ResourceConfig::load(&state.root)?;
        assert!(config.allocation_enabled && config.profiles[0].eligibility().is_ok());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(state.root.join("resources.yml"))?
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        Ok(())
    }
    #[test]
    fn concurrent_and_manually_changed_configuration_cannot_be_overwritten() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let state = State {
            root: temp.path().into(),
        };
        let first = proposal(&state);
        let stale = proposal(&state);
        first.confirm(&state)?;
        let before = fs::read(state.root.join("resources.yml"))?;
        assert!(stale.confirm(&state).is_err());
        assert_eq!(fs::read(state.root.join("resources.yml"))?, before);
        let current = Proposal::prepare(
            &state,
            discovery(),
            "codex",
            Some(0),
            "".into(),
            "".into(),
            ResourceTier::Light,
        )?;
        drop(current);
        assert_eq!(fs::read(state.root.join("resources.yml"))?, before);
        Ok(())
    }
    #[test]
    fn revalidation_does_not_change_model_tier_pool_or_disabled_policy() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let state = State {
            root: temp.path().into(),
        };
        proposal(&state).confirm(&state)?;
        let update = Proposal::prepare(
            &state,
            discovery(),
            "codex",
            Some(0),
            "other-model".into(),
            "low".into(),
            ResourceTier::Light,
        )?;
        assert_eq!(update.config.profiles[0].model, "fixed-model");
        assert_eq!(update.config.profiles[0].tier, ResourceTier::Standard);
        assert_eq!(update.config.profiles[0].authorization_revision, 2);
        update.confirm(&state)?;
        let mut config = ResourceConfig::load(&state.root)?;
        config.profiles[0].enabled = false;
        fs::write(
            state.root.join("resources.yml"),
            serde_yaml::to_string(&config)?,
        )?;
        assert!(
            Proposal::prepare(
                &state,
                discovery(),
                "codex",
                Some(0),
                "".into(),
                "".into(),
                ResourceTier::Light
            )
            .is_err()
        );
        Ok(())
    }
    #[test]
    fn checks_are_explicit_choices_preserve_configuration_and_refuse_drift() -> Result<()> {
        let t = tempfile::tempdir()?;
        fs::write(t.path().join("verify.sh"), "exit 0")?;
        fs::write(
            t.path().join("dispatch.yml"),
            "execution:\n  timeout_secs: 123\nprivate_evidence:\n  shadow: false\n",
        )?;
        let before = project_config_bytes(t.path())?;
        save_checks(t.path(), "sh ./verify.sh", before.clone())?;
        let (config, _) = crate::Config::discover(t.path(), None)?;
        assert_eq!(config.execution.timeout_secs, 123);
        assert_eq!(config.checks.verify, vec!["sh ./verify.sh"]);
        assert!(save_checks(t.path(), "sh ./verify.sh", before).is_err());
        assert!(save_checks(t.path(), "curl attacker", project_config_bytes(t.path())?).is_err());
        Ok(())
    }
    #[test]
    fn symlink_configuration_is_never_replaced() -> Result<()> {
        #[cfg(unix)]
        {
            let t = tempfile::tempdir()?;
            let outside = t.path().join("original");
            fs::write(&outside, "private")?;
            std::os::unix::fs::symlink(&outside, t.path().join("resources.yml"))?;
            assert!(read_config(&t.path().join("resources.yml")).is_err());
            assert_eq!(fs::read_to_string(outside)?, "private");
        }
        Ok(())
    }
}
