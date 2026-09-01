use std::{cmp::Ordering, collections::BTreeSet};

use anyhow::Result;

use crate::{BenchmarkPrior, TaskFeatures, TaskKind, TaskScope, db::Database};

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
