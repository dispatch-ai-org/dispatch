use std::{cmp::Ordering, collections::BTreeSet};

use anyhow::{Context, Result};

use crate::{TaskFeatures, db::Database};

#[derive(Debug, Clone, PartialEq)]
pub struct HarnessPrediction {
    pub harness: String,
    pub successes: u64,
    pub attempts: u64,
    pub score: Option<f64>,
}

pub fn rank_harnesses(
    database: &Database,
    features: &TaskFeatures,
    available_harnesses: &[String],
) -> Result<Vec<HarnessPrediction>> {
    let available = available_harnesses.iter().cloned().collect::<BTreeSet<_>>();
    let mut predictions = Vec::with_capacity(available.len());

    for harness in available {
        let mut successes = 0_u64;
        let mut attempts = 0_u64;
        for prior in database.matching_benchmark_priors(features, &harness)? {
            successes = successes
                .checked_add(prior.successes)
                .context("benchmark prior successes overflowed")?;
            attempts = attempts
                .checked_add(prior.attempts)
                .context("benchmark prior attempts overflowed")?;
        }
        predictions.push(HarnessPrediction {
            harness,
            successes,
            attempts,
            score: (attempts > 0).then_some(successes as f64 / attempts as f64),
        });
    }

    predictions.sort_by(compare_predictions);
    Ok(predictions)
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
