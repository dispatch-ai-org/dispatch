use std::{cmp::Ordering, collections::BTreeSet};

use anyhow::{Context, Result};
use chrono::Utc;

use crate::{
    AllocationAlternative, AllocationDecision, BenchmarkPrior, CapabilitySnapshot, ModelCapability,
    ResourceChoice, ResourceTier, TaskFeatures, TaskKind, TaskScope,
    config::{ResourceConfig, ResourceProfile},
    db::Database,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceMatch {
    /// Number of morphology dimensions that the prior establishes and matches.
    pub specificity: u8,
    pub prior: BenchmarkPrior,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HarnessPrediction {
    pub harness: String,
    pub successes: u64,
    pub attempts: u64,
    pub score: Option<f64>,
    pub evidence: Option<EvidenceMatch>,
}

pub fn select_resource(
    resources: &ResourceConfig,
    features: &TaskFeatures,
    execution_backend: &str,
    requested_model: Option<&str>,
    requested_effort: Option<&str>,
    fixed_model: Option<&str>,
    fixed_effort: Option<&str>,
) -> Result<AllocationDecision> {
    select_resource_filtered(
        resources,
        features,
        execution_backend,
        requested_model,
        requested_effort,
        fixed_model,
        fixed_effort,
        &[],
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn select_resource_filtered(
    resources: &ResourceConfig,
    features: &TaskFeatures,
    execution_backend: &str,
    requested_model: Option<&str>,
    requested_effort: Option<&str>,
    fixed_model: Option<&str>,
    fixed_effort: Option<&str>,
    exclusions: &[Option<String>],
) -> Result<AllocationDecision> {
    if let (Some(requested), Some(fixed)) = (requested_model, fixed_model) {
        anyhow::ensure!(
            requested == fixed,
            "--model {requested:?} conflicts with fixed harnesses.codex.model {fixed:?}"
        );
    }
    if let (Some(requested), Some(fixed)) = (requested_effort, fixed_effort) {
        anyhow::ensure!(
            requested == fixed,
            "--effort {requested:?} conflicts with fixed harnesses.codex.effort {fixed:?}"
        );
    }
    let model_constraint = requested_model.or(fixed_model);
    let effort_constraint = requested_effort.or(fixed_effort);
    let (desired_tier, policy_reason) = allocation_policy(features);
    let choices = resources
        .profiles
        .iter()
        .map(profile_choice)
        .collect::<Vec<_>>();
    let alternatives = resources
        .profiles
        .iter()
        .zip(&choices)
        .enumerate()
        .map(|(index, (profile, choice))| {
            let exclusion = if let Some(reason) = exclusions.get(index).and_then(Option::as_ref) {
                Some(reason.clone())
            } else if let Err(error) = profile.eligibility() {
                Some(error.to_string())
            } else if profile.runtime != execution_backend {
                Some(format!(
                    "profile runtime does not match the {execution_backend} execution backend"
                ))
            } else if model_constraint.is_some_and(|model| profile.model != model) {
                Some("model does not satisfy the fixed selection constraint".to_owned())
            } else if effort_constraint
                .is_some_and(|effort| profile.effort.as_deref() != Some(effort))
            {
                Some("effort does not satisfy the fixed selection constraint".to_owned())
            } else if model_constraint.is_none()
                && tier_rank(&profile.tier) < tier_rank(&desired_tier)
            {
                Some(format!(
                    "policy requires at least the {} tier",
                    desired_tier.as_str()
                ))
            } else {
                None
            };
            AllocationAlternative {
                choice: choice.clone(),
                eligible: exclusion.is_none(),
                exclusion,
            }
        })
        .collect::<Vec<_>>();
    let selected = alternatives
        .iter()
        .filter(|alternative| alternative.eligible)
        .min_by_key(|alternative| tier_rank(&alternative.choice.tier))
        .map(|alternative| alternative.choice.clone())
        .with_context(|| match model_constraint {
            Some(model) => format!(
                "no included, no-overage-verified resource profile matches model {model:?}{}",
                effort_constraint
                    .map(|effort| format!(" and effort {effort:?}"))
                    .unwrap_or_default()
            ),
            None => format!(
                "no included, no-overage-verified resource profile is configured for the {} tier",
                desired_tier.as_str()
            ),
        })?;
    let capability = CapabilitySnapshot {
        version: 1,
        source: "user_validated_profile".into(),
        harness: selected.harness.clone(),
        observed_at: Utc::now(),
        models: resources
            .profiles
            .iter()
            .filter(|profile| profile.harness == selected.harness)
            .map(|profile| ModelCapability {
                model: profile.model.clone(),
                efforts: profile.effort.iter().cloned().collect(),
                included: profile.included,
                no_overage_verified: profile.no_overage_verified,
            })
            .collect(),
    };
    let reason = if requested_model.is_some() || requested_effort.is_some() {
        "Explicit model or effort constraint selected this resource.".into()
    } else if fixed_model.is_some() || fixed_effort.is_some() {
        "Project harness configuration fixed this resource.".into()
    } else if selected.tier != desired_tier {
        format!(
            "Minimum suitable lane is {}; the first available configured {} resource was selected.",
            desired_tier.as_str(),
            selected.tier.as_str()
        )
    } else {
        policy_reason.to_owned()
    };
    Ok(AllocationDecision {
        version: 1,
        policy_version: "allocation-portfolio-v3".into(),
        task_features: features.clone(),
        selected,
        reason,
        capability,
        alternatives,
    })
}

fn tier_rank(tier: &ResourceTier) -> u8 {
    match tier {
        ResourceTier::Light => 0,
        ResourceTier::Standard => 1,
        ResourceTier::Strong => 2,
    }
}

fn profile_choice(profile: &ResourceProfile) -> ResourceChoice {
    ResourceChoice {
        provider: profile.provider.clone(),
        funding_source: profile.funding_source.clone(),
        harness: profile.harness.clone(),
        requested_model: profile.model.clone(),
        resolved_model: profile.model.clone(),
        effort: profile.effort.clone(),
        service_mode: profile.service_mode.clone(),
        runtime: profile.runtime.clone(),
        pool: profile.pool.clone(),
        tier: profile.tier.clone(),
        no_overage_verified: profile.no_overage_verified,
        internal_composition: "unknown".into(),
    }
}

fn allocation_policy(features: &TaskFeatures) -> (ResourceTier, &'static str) {
    match (&features.task_kind, &features.scope) {
        (_, TaskScope::Broad) => (
            ResourceTier::Strong,
            "Using strong: the task has broad scope.",
        ),
        (TaskKind::Feature | TaskKind::Refactor, _) => (
            ResourceTier::Strong,
            "Using strong: the task is classified as feature or refactor work.",
        ),
        (TaskKind::Tests, TaskScope::Localized) => (
            ResourceTier::Light,
            "Using light: the task requests tests for an identified file.",
        ),
        (TaskKind::Unknown, _) | (_, TaskScope::Unknown) => (
            ResourceTier::Standard,
            "Using standard: the task type or affected files are unknown.",
        ),
        _ => (
            ResourceTier::Standard,
            "Using standard: the task is ordinary implementation or tests spanning multiple files.",
        ),
    }
}

pub fn rank_harnesses(
    database: &Database,
    features: &TaskFeatures,
    available_harnesses: &[String],
) -> Result<Vec<HarnessPrediction>> {
    let available = available_harnesses.iter().cloned().collect::<BTreeSet<_>>();
    let mut predictions = Vec::with_capacity(available.len());

    for harness in available {
        let evidence = select_evidence(database, features, &harness)?;
        let successes = evidence
            .as_ref()
            .map_or(0, |evidence| evidence.prior.successes);
        let attempts = evidence
            .as_ref()
            .map_or(0, |evidence| evidence.prior.attempts);
        predictions.push(HarnessPrediction {
            harness,
            successes,
            attempts,
            score: evidence
                .as_ref()
                .map(|_| successes as f64 / attempts as f64),
            evidence,
        });
    }

    predictions.sort_by(compare_predictions);
    Ok(predictions)
}

fn select_evidence(
    database: &Database,
    features: &TaskFeatures,
    harness: &str,
) -> Result<Option<EvidenceMatch>> {
    let mut selected = None;
    for (compatible, specificity) in compatible_features(features) {
        for prior in database.matching_benchmark_priors(&compatible, harness)? {
            if prior.attempts == 0 {
                continue;
            }
            let candidate = EvidenceMatch { specificity, prior };
            if selected
                .as_ref()
                .is_none_or(|current| compare_evidence(&candidate, current).is_gt())
            {
                selected = Some(candidate);
            }
        }
    }
    Ok(selected)
}

fn compatible_features(features: &TaskFeatures) -> Vec<(TaskFeatures, u8)> {
    let languages = match &features.language {
        Some(language) => vec![(Some(language.clone()), 1), (None, 0)],
        None => vec![(None, 0)],
    };
    let task_kinds = match features.task_kind {
        TaskKind::Unknown => vec![(TaskKind::Unknown, 0)],
        _ => vec![(features.task_kind.clone(), 1), (TaskKind::Unknown, 0)],
    };
    let scopes = match features.scope {
        TaskScope::Unknown => vec![(TaskScope::Unknown, 0)],
        _ => vec![(features.scope.clone(), 1), (TaskScope::Unknown, 0)],
    };

    let mut compatible = Vec::with_capacity(languages.len() * task_kinds.len() * scopes.len());
    for (language, language_specificity) in languages {
        for (task_kind, task_kind_specificity) in &task_kinds {
            for (scope, scope_specificity) in &scopes {
                compatible.push((
                    TaskFeatures {
                        language: language.clone(),
                        task_kind: task_kind.clone(),
                        scope: scope.clone(),
                    },
                    language_specificity + task_kind_specificity + scope_specificity,
                ));
            }
        }
    }
    compatible
}

fn compare_evidence(left: &EvidenceMatch, right: &EvidenceMatch) -> Ordering {
    left.specificity
        .cmp(&right.specificity)
        .then_with(|| left.prior.attempts.cmp(&right.prior.attempts))
        .then_with(|| compare_prior_identity(&right.prior, &left.prior))
}

fn compare_prior_identity(left: &BenchmarkPrior, right: &BenchmarkPrior) -> Ordering {
    left.source
        .cmp(&right.source)
        .then_with(|| left.dataset.cmp(&right.dataset))
        .then_with(|| left.dataset_version.cmp(&right.dataset_version))
        .then_with(|| left.model.cmp(&right.model))
        .then_with(|| left.language.cmp(&right.language))
        .then_with(|| left.task_kind.as_str().cmp(right.task_kind.as_str()))
        .then_with(|| left.scope.as_str().cmp(right.scope.as_str()))
}

fn compare_predictions(left: &HarnessPrediction, right: &HarnessPrediction) -> Ordering {
    match (left.attempts, right.attempts) {
        (0, 0) => left.harness.cmp(&right.harness),
        (0, _) => Ordering::Greater,
        (_, 0) => Ordering::Less,
        _ => ((right.successes as u128) * (left.attempts as u128))
            .cmp(&((left.successes as u128) * (right.attempts as u128)))
            .then_with(|| left.harness.cmp(&right.harness)),
    }
}

#[cfg(test)]
mod allocation_tests {
    use super::*;

    fn profile(tier: ResourceTier, model: &str, effort: &str) -> ResourceProfile {
        ResourceProfile {
            enabled: true,
            claude_subscription: None,
            provider: "openai".into(),
            funding_source: "chatgpt-plus".into(),
            harness: "codex".into(),
            model: model.into(),
            effort: Some(effort.into()),
            service_mode: "standard".into(),
            runtime: "local".into(),
            pool: "chatgpt-codex".into(),
            provider_buckets: vec!["codex".into()],
            tier,
            included: true,
            no_overage_verified: true,
            authorization_revision: 1,
        }
    }

    fn resources() -> ResourceConfig {
        ResourceConfig {
            version: 1,
            allocation_enabled: true,
            capacity: crate::config::CapacityConfig::default(),
            profiles: vec![
                profile(ResourceTier::Light, "light-model", "low"),
                profile(ResourceTier::Standard, "standard-model", "medium"),
                profile(ResourceTier::Strong, "strong-model", "high"),
            ],
        }
    }

    #[test]
    fn treats_models_of_one_harness_as_distinct_deterministic_choices() {
        let features = TaskFeatures {
            language: Some("rust".into()),
            task_kind: TaskKind::Tests,
            scope: TaskScope::Localized,
        };
        let decision =
            select_resource(&resources(), &features, "local", None, None, None, None).unwrap();
        assert_eq!(decision.selected.requested_model, "light-model");
        assert_eq!(decision.alternatives.len(), 3);
        assert_eq!(decision.capability.models.len(), 3);
    }

    #[test]
    fn natural_goal_without_paths_uses_standard_and_preserves_unknown_features() {
        let source = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("main.c"), "int ballRadius = 20;\n").unwrap();
        let features = crate::classifier::classify_task(
            source.path(),
            "Let's adjust the size of the bouncing ball to make it twice as big.",
        )
        .unwrap();
        assert_eq!(features.task_kind, TaskKind::Unknown);
        assert_eq!(features.scope, TaskScope::Unknown);
        let mut resources = resources();
        resources
            .profiles
            .retain(|p| p.tier != ResourceTier::Strong);
        let decision =
            select_resource(&resources, &features, "local", None, None, None, None).unwrap();
        assert_eq!(decision.selected.requested_model, "standard-model");
        assert_eq!(decision.task_features, features);
        assert!(decision.reason.contains("unknown"));
        assert_eq!(decision.policy_version, "allocation-portfolio-v3");
        resources.profiles[1].no_overage_verified = false;
        assert!(select_resource(&resources, &features, "local", None, None, None, None).is_err());
    }

    #[test]
    fn uncertain_scope_does_not_require_strong_or_qualify_for_light() {
        for kind in [TaskKind::Tests, TaskKind::BugFix, TaskKind::Unknown] {
            for scope in [
                TaskScope::Localized,
                TaskScope::MultiFile,
                TaskScope::Unknown,
                TaskScope::Broad,
            ] {
                let features = TaskFeatures {
                    language: None,
                    task_kind: kind.clone(),
                    scope: scope.clone(),
                };
                let decision =
                    select_resource(&resources(), &features, "local", None, None, None, None)
                        .unwrap();
                let expected = if scope == TaskScope::Broad {
                    ResourceTier::Strong
                } else if kind == TaskKind::Tests && scope == TaskScope::Localized {
                    ResourceTier::Light
                } else {
                    ResourceTier::Standard
                };
                assert_eq!(decision.selected.tier, expected);
            }
        }
        for kind in [TaskKind::Feature, TaskKind::Refactor] {
            let features = TaskFeatures {
                task_kind: kind,
                ..TaskFeatures::default()
            };
            let mut resources = resources();
            assert_eq!(
                select_resource(&resources, &features, "local", None, None, None, None)
                    .unwrap()
                    .selected
                    .tier,
                ResourceTier::Strong
            );
            resources
                .profiles
                .retain(|p| p.tier != ResourceTier::Strong);
            assert!(
                select_resource(&resources, &features, "local", None, None, None, None).is_err()
            );
        }
    }

    #[test]
    fn fixed_constraints_win_and_unverified_overage_is_ineligible() {
        let mut resources = resources();
        resources.profiles[2].no_overage_verified = false;
        let features = TaskFeatures::default();
        let error = select_resource(
            &resources,
            &features,
            "local",
            Some("strong-model"),
            None,
            Some("strong-model"),
            None,
        )
        .unwrap_err();
        assert!(error.to_string().contains("no-overage-verified"));

        let decision = select_resource(
            &resources,
            &features,
            "local",
            Some("standard-model"),
            Some("medium"),
            Some("standard-model"),
            Some("medium"),
        )
        .unwrap();
        assert_eq!(decision.selected.tier, ResourceTier::Standard);
    }

    #[test]
    fn conflicting_cli_and_project_constraints_fail_instead_of_substituting() {
        let error = select_resource(
            &resources(),
            &TaskFeatures::default(),
            "local",
            Some("light-model"),
            None,
            Some("strong-model"),
            None,
        )
        .unwrap_err();
        assert!(error.to_string().contains("conflicts"));
    }
}
