use chrono::{DateTime, TimeZone, Utc};
use dispatch::{
    BenchmarkPrior, TaskFeatures, TaskKind, TaskScope, db::Database, router::rank_harnesses,
};

fn at(second: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 31, 12, 0, second).unwrap()
}

fn features() -> TaskFeatures {
    TaskFeatures {
        language: Some("rust".to_owned()),
        task_kind: TaskKind::BugFix,
        scope: TaskScope::Localized,
    }
}

fn prior(harness: &str, model: Option<&str>, successes: u64, attempts: u64) -> BenchmarkPrior {
    BenchmarkPrior {
        source: "public-benchmark".to_owned(),
        dataset: "swe-bench".to_owned(),
        dataset_version: "verified-2026-08".to_owned(),
        harness: harness.to_owned(),
        model: model.map(str::to_owned),
        language: Some("rust".to_owned()),
        task_kind: TaskKind::BugFix,
        scope: TaskScope::Localized,
        successes,
        attempts,
        updated_at: at(1),
    }
}

#[test]
fn cached_prior_round_trips_with_distinct_identity_and_provenance() -> anyhow::Result<()> {
    let database = Database::open_in_memory()?;
    let cached = prior("codex", Some("gpt-5"), 7, 10);

    database.upsert_benchmark_prior(&cached)?;

    let matches = database.matching_benchmark_priors(&features(), "codex")?;
    assert_eq!(matches, vec![cached]);
    assert_eq!(matches[0].harness, "codex");
    assert_eq!(matches[0].model.as_deref(), Some("gpt-5"));
    assert_eq!(matches[0].source, "public-benchmark");
    assert_eq!(matches[0].dataset, "swe-bench");
    assert_eq!(matches[0].dataset_version, "verified-2026-08");
    Ok(())
}

#[test]
fn cached_prior_upsert_replaces_observations_for_the_same_identity() -> anyhow::Result<()> {
    let database = Database::open_in_memory()?;
    let original = prior("codex", None, 2, 4);
    let mut updated = original.clone();
    updated.successes = 8;
    updated.attempts = 10;
    updated.updated_at = at(2);

    database.upsert_benchmark_prior(&original)?;
    database.upsert_benchmark_prior(&updated)?;

    assert_eq!(
        database.matching_benchmark_priors(&features(), "codex")?,
        vec![updated]
    );
    Ok(())
}

#[test]
fn prior_lookup_requires_exact_task_morphology_and_harness() -> anyhow::Result<()> {
    let database = Database::open_in_memory()?;
    let cached = prior("codex", None, 3, 5);
    database.upsert_benchmark_prior(&cached)?;

    assert_eq!(
        database.matching_benchmark_priors(&features(), "codex")?,
        vec![cached]
    );
    assert!(
        database
            .matching_benchmark_priors(
                &TaskFeatures {
                    task_kind: TaskKind::Feature,
                    ..features()
                },
                "codex",
            )?
            .is_empty()
    );
    assert!(
        database
            .matching_benchmark_priors(
                &TaskFeatures {
                    scope: TaskScope::Broad,
                    ..features()
                },
                "codex",
            )?
            .is_empty()
    );
    assert!(
        database
            .matching_benchmark_priors(
                &TaskFeatures {
                    language: None,
                    ..features()
                },
                "codex",
            )?
            .is_empty()
    );
    assert!(
        database
            .matching_benchmark_priors(&features(), "cursor")?
            .is_empty()
    );
    Ok(())
}

#[test]
fn router_ranks_higher_observed_success_ratio_first() -> anyhow::Result<()> {
    let database = Database::open_in_memory()?;
    database.upsert_benchmark_prior(&prior("codex", None, 9, 10))?;
    database.upsert_benchmark_prior(&prior("cursor", None, 3, 5))?;
    let available = vec!["cursor".to_owned(), "unseen".to_owned(), "codex".to_owned()];

    let ranked = rank_harnesses(&database, &features(), &available)?;

    assert_eq!(ranked[0].harness, "codex");
    assert_eq!(ranked[0].score, Some(0.9));
    assert_eq!(ranked[1].harness, "cursor");
    assert_eq!(ranked[1].score, Some(0.6));
    assert_eq!(ranked[2].harness, "unseen");
    assert_eq!(ranked[2].score, None);
    Ok(())
}

#[test]
fn router_reports_no_score_for_missing_or_zero_attempt_evidence() -> anyhow::Result<()> {
    let database = Database::open_in_memory()?;
    database.upsert_benchmark_prior(&prior("codex", None, 0, 0))?;
    let available = vec!["cursor".to_owned(), "codex".to_owned()];

    let ranked = rank_harnesses(&database, &features(), &available)?;

    assert_eq!(ranked[0].harness, "codex");
    assert_eq!(ranked[0].attempts, 0);
    assert_eq!(ranked[0].score, None);
    assert_eq!(ranked[1].harness, "cursor");
    assert_eq!(ranked[1].attempts, 0);
    assert_eq!(ranked[1].score, None);
    Ok(())
}

#[test]
fn router_breaks_equal_score_ties_by_harness_identity() -> anyhow::Result<()> {
    let database = Database::open_in_memory()?;
    database.upsert_benchmark_prior(&prior("zeta", None, 2, 4))?;
    database.upsert_benchmark_prior(&prior("alpha", None, 1, 2))?;
    let available = vec!["zeta".to_owned(), "alpha".to_owned()];

    let ranked = rank_harnesses(&database, &features(), &available)?;

    assert_eq!(
        ranked
            .iter()
            .map(|prediction| prediction.harness.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha", "zeta"]
    );
    Ok(())
}
